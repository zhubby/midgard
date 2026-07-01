use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{ConfigMap, PersistentVolumeClaim, Secret, Service};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use kube::Client;
use kube::api::Api;
use kube::runtime::{Controller, watcher};
use midgard_core::{CapabilityDescriptor, RiskLevel};
use midgard_operator::controller::root_api;
use midgard_operator::probe::start_probe_servers;
use midgard_operator::traits::OperatorDefinition;
use tokio::time::sleep;
use uuid::Uuid;

use crate::api::{ValkeyCluster, ValkeyNode};
use crate::controller::{Context, cluster, node};
use crate::error::{Error, Result};
use crate::lease::{LeaseConfig, LeaseGuard};
use crate::protocol::{self, VALKEY_MIDDLEWARE_KIND};

#[derive(Clone, Debug)]
pub struct ValkeyOperatorConfig {
    pub server_endpoint: String,
    pub workspace_id: String,
    pub operator_id: Option<String>,
    pub watch_namespaces: Vec<String>,
    pub tls_ca_path: Option<PathBuf>,
    pub allow_insecure_without_tls: bool,
    pub lease: LeaseConfig,
    pub heartbeat_interval: Duration,
    pub health_probe_bind_address: Option<String>,
    pub metrics_bind_address: Option<String>,
}

impl ValkeyOperatorConfig {
    pub fn operator_id(&self) -> String {
        self.operator_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("midgard-valkey-operator-{}", self.workspace_id))
    }

    pub fn validate(&self) -> Result<()> {
        if self.server_endpoint.trim().is_empty() {
            return Err(Error::InvalidConfig(
                "operator server endpoint is required".to_string(),
            ));
        }
        if Uuid::parse_str(&self.workspace_id).is_err() {
            return Err(Error::InvalidConfig(
                "workspace id must be a UUID".to_string(),
            ));
        }
        if !self.allow_insecure_without_tls && self.server_endpoint.starts_with("http://") {
            return Err(Error::InvalidConfig(
                "operator server endpoint must use HTTPS unless insecure mode is enabled"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

impl OperatorDefinition for ValkeyOperatorConfig {
    fn operator_id(&self) -> String {
        ValkeyOperatorConfig::operator_id(self)
    }

    fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    fn middleware_kind(&self) -> &str {
        VALKEY_MIDDLEWARE_KIND
    }

    fn display_name(&self) -> &str {
        "Midgard Valkey Operator"
    }

    fn supported_operations(&self) -> Vec<String> {
        vec![
            "create".to_string(),
            "update".to_string(),
            "delete".to_string(),
            "query".to_string(),
            "refresh".to_string(),
            "reconcile".to_string(),
        ]
    }

    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor::new("valkey.query", "Query Valkey clusters", RiskLevel::Low),
            CapabilityDescriptor::new(
                "valkey.reconcile",
                "Reconcile Valkey desired state",
                RiskLevel::Medium,
            ),
            CapabilityDescriptor::new("valkey.create", "Create Valkey clusters", RiskLevel::High),
            CapabilityDescriptor::new("valkey.update", "Update Valkey clusters", RiskLevel::High),
            CapabilityDescriptor::new(
                "valkey.delete",
                "Delete Valkey clusters",
                RiskLevel::Critical,
            ),
        ]
    }

    fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }
}

impl Default for ValkeyOperatorConfig {
    fn default() -> Self {
        Self {
            server_endpoint: "https://127.0.0.1:8081".to_string(),
            workspace_id: String::new(),
            operator_id: None,
            watch_namespaces: Vec::new(),
            tls_ca_path: None,
            allow_insecure_without_tls: false,
            lease: crate::lease::default_config(),
            heartbeat_interval: Duration::from_secs(10),
            health_probe_bind_address: None,
            metrics_bind_address: None,
        }
    }
}

pub async fn run(config: ValkeyOperatorConfig) -> Result<()> {
    config.validate()?;
    start_internal_servers(&config)?;
    let client = Client::try_default().await?;

    loop {
        let holder_identity = format!("{}-{}", config.operator_id(), Uuid::new_v4());
        let guard =
            acquire_lock_with_retry(client.clone(), config.lease.clone(), holder_identity).await?;
        tracing::info!(
            holder_identity = %guard.holder_identity(),
            lease_namespace = %config.lease.namespace,
            lease_name = %config.lease.name,
            "acquired valkey operator lease"
        );

        match run_locked(config.clone(), client.clone(), guard).await {
            Ok(LockedRunOutcome::Shutdown) => return Ok(()),
            Ok(LockedRunOutcome::Retry) => {}
            Err(err) => tracing::warn!(error = %err, "valkey operator runtime exited"),
        }

        sleep(config.lease.retry_interval).await;
    }
}

async fn acquire_lock_with_retry(
    client: Client,
    lease: LeaseConfig,
    holder_identity: String,
) -> Result<LeaseGuard> {
    loop {
        match LeaseGuard::acquire(client.clone(), lease.clone(), holder_identity.clone()).await {
            Ok(guard) => return Ok(guard),
            Err(midgard_operator::OperatorError::LeaseHeld(holder)) => {
                tracing::info!(
                    holder = %holder,
                    lease_namespace = %lease.namespace,
                    lease_name = %lease.name,
                    "waiting for valkey operator lease"
                );
                sleep(lease.retry_interval).await;
            }
            Err(err) => return Err(err.into()),
        }
    }
}

enum LockedRunOutcome {
    Retry,
    Shutdown,
}

async fn run_locked(
    config: ValkeyOperatorConfig,
    client: Client,
    guard: LeaseGuard,
) -> Result<LockedRunOutcome> {
    let controllers = run_controllers(client.clone(), config.watch_namespaces.clone());
    let protocol = protocol::run_channel(config, client);
    let lease = guard.renew_until_lost();

    tokio::select! {
        result = controllers => {
            result?;
            Ok(LockedRunOutcome::Retry)
        }
        result = protocol => {
            result?;
            Ok(LockedRunOutcome::Retry)
        }
        result = lease => {
            result?;
            Ok(LockedRunOutcome::Retry)
        }
        _ = shutdown_signal() => {
            tracing::info!("shutdown signal received");
            Ok(LockedRunOutcome::Shutdown)
        }
    }
}

async fn run_controllers(client: Client, watch_namespaces: Vec<String>) -> Result<()> {
    let context = Arc::new(Context {
        client: client.clone(),
        watch_namespaces: watch_namespaces.clone(),
    });

    let cluster_api = root_api::<ValkeyCluster>(client.clone(), &watch_namespaces);
    let node_api = root_api::<ValkeyNode>(client.clone(), &watch_namespaces);

    let cluster_controller = Controller::new(cluster_api, watcher::Config::default())
        .owns(
            Api::<Service>::all(client.clone()),
            watcher::Config::default(),
        )
        .owns(
            Api::<ConfigMap>::all(client.clone()),
            watcher::Config::default(),
        )
        .owns(
            Api::<Secret>::all(client.clone()),
            watcher::Config::default(),
        )
        .owns(
            Api::<PodDisruptionBudget>::all(client.clone()),
            watcher::Config::default(),
        )
        .owns(
            Api::<ValkeyNode>::all(client.clone()),
            watcher::Config::default(),
        )
        .run(cluster::reconcile, cluster::error_policy, context.clone())
        .for_each(|result| async {
            match result {
                Ok((obj, action)) => {
                    tracing::info!(
                        name = %obj.name,
                        namespace = ?obj.namespace,
                        ?action,
                        "reconciled ValkeyCluster"
                    )
                }
                Err(err) => tracing::error!(%err, "ValkeyCluster controller error"),
            }
        });

    let node_controller = Controller::new(node_api, watcher::Config::default())
        .owns(
            Api::<ConfigMap>::all(client.clone()),
            watcher::Config::default(),
        )
        .owns(
            Api::<PersistentVolumeClaim>::all(client.clone()),
            watcher::Config::default(),
        )
        .owns(
            Api::<StatefulSet>::all(client.clone()),
            watcher::Config::default(),
        )
        .owns(
            Api::<Deployment>::all(client.clone()),
            watcher::Config::default(),
        )
        .run(node::reconcile, node::error_policy, context.clone())
        .for_each(|result| async {
            match result {
                Ok((obj, action)) => {
                    tracing::info!(
                        name = %obj.name,
                        namespace = ?obj.namespace,
                        ?action,
                        "reconciled ValkeyNode"
                    )
                }
                Err(err) => tracing::error!(%err, "ValkeyNode controller error"),
            }
        });

    tracing::info!("starting midgard valkey operator controllers");
    tokio::select! {
        _ = cluster_controller => {},
        _ = node_controller => {},
    }
    Ok(())
}

fn start_internal_servers(config: &ValkeyOperatorConfig) -> Result<()> {
    start_probe_servers(
        config.health_probe_bind_address.as_deref(),
        config.metrics_bind_address.as_deref(),
        Some(metrics_body()),
        "valkey operator",
    )?;
    Ok(())
}

fn metrics_body() -> String {
    concat!(
        "# HELP midgard_valkey_operator_build_info Build information for the Midgard Valkey operator.\n",
        "# TYPE midgard_valkey_operator_build_info gauge\n",
        "midgard_valkey_operator_build_info{version=\"",
        env!("CARGO_PKG_VERSION"),
        "\"} 1\n"
    )
    .to_string()
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use midgard_operator::traits::OperatorDefinition;

    use super::*;

    #[test]
    fn default_operator_id_is_deterministic_for_workspace() {
        let config = ValkeyOperatorConfig {
            workspace_id: "11111111-1111-1111-1111-111111111111".to_string(),
            ..ValkeyOperatorConfig::default()
        };

        assert_eq!(
            config.operator_id(),
            "midgard-valkey-operator-11111111-1111-1111-1111-111111111111"
        );
    }

    #[test]
    fn insecure_mode_allows_http_endpoint() {
        let config = ValkeyOperatorConfig {
            server_endpoint: "http://127.0.0.1:8081".to_string(),
            workspace_id: "11111111-1111-1111-1111-111111111111".to_string(),
            allow_insecure_without_tls: true,
            ..ValkeyOperatorConfig::default()
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn secure_mode_rejects_http_endpoint() {
        let config = ValkeyOperatorConfig {
            server_endpoint: "http://127.0.0.1:8081".to_string(),
            workspace_id: "11111111-1111-1111-1111-111111111111".to_string(),
            ..ValkeyOperatorConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn operator_definition_exposes_existing_valkey_capabilities() {
        let config = ValkeyOperatorConfig {
            workspace_id: "11111111-1111-1111-1111-111111111111".to_string(),
            ..ValkeyOperatorConfig::default()
        };

        assert_eq!(config.middleware_kind(), VALKEY_MIDDLEWARE_KIND);
        assert_eq!(config.display_name(), "Midgard Valkey Operator");
        assert_eq!(
            config.supported_operations(),
            vec![
                "create".to_string(),
                "update".to_string(),
                "delete".to_string(),
                "query".to_string(),
                "refresh".to_string(),
                "reconcile".to_string(),
            ]
        );
        let capabilities = config.capabilities();
        assert_eq!(capabilities[0].id, "valkey.query");
        assert_eq!(capabilities[0].name, "Query Valkey clusters");
        assert_eq!(capabilities[0].risk_level, RiskLevel::Low);
        assert_eq!(capabilities[1].id, "valkey.reconcile");
        assert_eq!(capabilities[1].risk_level, RiskLevel::Medium);
        assert_eq!(capabilities[2].id, "valkey.create");
        assert_eq!(capabilities[2].risk_level, RiskLevel::High);
        assert_eq!(capabilities[3].id, "valkey.update");
        assert_eq!(capabilities[3].risk_level, RiskLevel::High);
        assert_eq!(capabilities[4].id, "valkey.delete");
        assert_eq!(capabilities[4].risk_level, RiskLevel::Critical);
    }

    #[test]
    fn probe_address_accepts_colon_port() {
        let addr = midgard_operator::probe::optional_probe_addr(Some(":8081"))
            .unwrap()
            .unwrap();

        assert_eq!(addr.to_string(), "0.0.0.0:8081");
    }
}
