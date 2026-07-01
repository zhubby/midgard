use std::collections::BTreeMap;
use std::fs;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, DeleteParams, ListParams};
use kube::{Client, ResourceExt};
use midgard_operator::control::{capability_message, heartbeat_message, registration_message};
use midgard_operator::traits::OperatorResourceAdapter;
use midgard_protocol::operator::{
    InventorySnapshot, MiddlewareResource, MiddlewareStatus, OperationResult, OperationStatus,
    OperatorToServer, ServerAck, ServerCommand, ServerToOperator,
    operator_control_client::OperatorControlClient, operator_to_server, server_to_operator,
};
use midgard_protocol::{CommandType, DesiredState, json_to_struct, struct_to_json};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Request;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};
use uuid::Uuid;

use crate::api::{ClusterState, ValkeyCluster, ValkeyClusterSpec};
use crate::controller::apply;
use crate::error::{Error, Result};
use crate::runtime::ValkeyOperatorConfig;

pub const VALKEY_MIDDLEWARE_KIND: &str = "valkey";
pub const LABEL_WORKSPACE_ID: &str = "midgard.io/workspace-id";
pub const LABEL_MIDDLEWARE_ID: &str = "midgard.io/middleware-id";
pub const LABEL_MIDDLEWARE_KIND: &str = "midgard.io/middleware-kind";

type OutboundSender = mpsc::Sender<OperatorToServer>;

pub async fn run_channel(config: ValkeyOperatorConfig, client: Client) -> Result<()> {
    let channel = connect_channel(&config).await?;
    let mut control = OperatorControlClient::new(channel);
    let (sender, receiver) = mpsc::channel(64);

    sender
        .send(registration_message(&config))
        .await
        .map_err(|_| Error::InvalidState("operator registration stream closed".to_string()))?;
    sender
        .send(capability_message(&config))
        .await
        .map_err(|_| Error::InvalidState("operator capability stream closed".to_string()))?;

    let request = Request::new(ReceiverStream::new(receiver));
    let mut inbound = control.open_channel(request).await?.into_inner();
    let heartbeat_sender = sender.clone();
    let heartbeat_config = config.clone();
    tokio::spawn(async move {
        loop {
            sleep(heartbeat_config.heartbeat_interval).await;
            if heartbeat_sender
                .send(heartbeat_message(&heartbeat_config))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    while let Some(message) = inbound.message().await? {
        handle_server_message(&config, client.clone(), sender.clone(), message).await?;
    }

    Ok(())
}

async fn connect_channel(config: &ValkeyOperatorConfig) -> Result<Channel> {
    let mut endpoint = Endpoint::from_shared(config.server_endpoint.clone())
        .map_err(|err| Error::InvalidConfig(format!("invalid operator endpoint: {err}")))?;
    if !config.allow_insecure_without_tls {
        let mut tls = ClientTlsConfig::new();
        if let Some(ca_path) = &config.tls_ca_path {
            let ca = fs::read(ca_path)
                .map_err(|err| Error::InvalidConfig(format!("read TLS CA bundle: {err}")))?;
            tls = tls.ca_certificate(Certificate::from_pem(ca));
        }
        endpoint = endpoint.tls_config(tls)?;
    }

    Ok(endpoint.connect().await?)
}

async fn handle_server_message(
    config: &ValkeyOperatorConfig,
    client: Client,
    sender: OutboundSender,
    message: ServerToOperator,
) -> Result<()> {
    let Some(payload) = message.payload else {
        return Ok(());
    };

    match payload {
        server_to_operator::Payload::Ack(ServerAck { accepted, message }) => {
            if !accepted {
                return Err(Error::InvalidState(format!(
                    "operator registration rejected: {message}"
                )));
            }
            tracing::info!(%message, "midgard operator registration acknowledged");
        }
        server_to_operator::Payload::Command(command) => {
            handle_command(config, client, sender, command).await?;
        }
    }

    Ok(())
}

async fn handle_command(
    config: &ValkeyOperatorConfig,
    client: Client,
    sender: OutboundSender,
    command: ServerCommand,
) -> Result<()> {
    let command_type =
        CommandType::try_from(command.command_type).unwrap_or(CommandType::UnknownCommandType);
    match command_type {
        CommandType::Create | CommandType::Update => {
            if let Some(instance) = command.instance {
                apply_instance(client, sender, command.operation_id, instance).await?;
            }
        }
        CommandType::Delete => {
            if let Some(instance) = command.instance {
                delete_instance(client, sender, command.operation_id, instance).await?;
            }
        }
        CommandType::Query => {
            if let Some(instance) = command.instance {
                query_instance(client, sender, command.operation_id, instance).await?;
            } else {
                refresh_inventory(config, client, sender, command.operation_id).await?;
            }
        }
        CommandType::Refresh => {
            refresh_inventory(config, client, sender, command.operation_id).await?;
        }
        CommandType::Reconcile => {
            for instance in command.instances {
                if is_delete_intent(&instance) {
                    delete_instance(
                        client.clone(),
                        sender.clone(),
                        command.operation_id.clone(),
                        instance,
                    )
                    .await?;
                } else {
                    apply_instance(
                        client.clone(),
                        sender.clone(),
                        command.operation_id.clone(),
                        instance,
                    )
                    .await?;
                }
            }
            refresh_inventory(config, client, sender, command.operation_id).await?;
        }
        CommandType::UnknownCommandType => {}
    }

    Ok(())
}

async fn apply_instance(
    client: Client,
    sender: OutboundSender,
    operation_id: String,
    instance: MiddlewareResource,
) -> Result<()> {
    let result = middleware_resource_to_cluster(instance.clone());
    let cluster = match result {
        Ok(cluster) => cluster,
        Err(err) => {
            send_operation_status(
                &sender,
                &operation_id,
                &instance.id,
                MiddlewareStatus::Degraded,
                err.to_string(),
            )
            .await?;
            send_operation_result(
                &sender,
                operation_id,
                false,
                err.to_string(),
                Some(instance),
            )
            .await?;
            return Ok(());
        }
    };
    let namespace = cluster.namespace().unwrap_or_default();
    let name = cluster.name_any();
    let api = Api::<ValkeyCluster>::namespaced(client, &namespace);
    let applied = apply(&api, &name, &cluster).await?;
    let resource = cluster_to_middleware_resource(&applied, Some(&instance))?;
    send_operation_result(
        &sender,
        operation_id,
        true,
        format!("ValkeyCluster {namespace}/{name} applied"),
        Some(resource),
    )
    .await
}

async fn delete_instance(
    client: Client,
    sender: OutboundSender,
    operation_id: String,
    mut instance: MiddlewareResource,
) -> Result<()> {
    let namespace = defaulted_namespace(&instance);
    let name = instance.name.clone();
    let api = Api::<ValkeyCluster>::namespaced(client, &namespace);
    if api.get_opt(&name).await?.is_some() {
        api.delete(&name, &DeleteParams::default()).await?;
    }
    instance.status = MiddlewareStatus::Stopped as i32;
    send_operation_result(
        &sender,
        operation_id,
        true,
        format!("ValkeyCluster {namespace}/{name} deleted"),
        Some(instance),
    )
    .await
}

async fn query_instance(
    client: Client,
    sender: OutboundSender,
    operation_id: String,
    instance: MiddlewareResource,
) -> Result<()> {
    let namespace = defaulted_namespace(&instance);
    let api = Api::<ValkeyCluster>::namespaced(client, &namespace);
    match api.get_opt(&instance.name).await? {
        Some(cluster) => {
            let resource = cluster_to_middleware_resource(&cluster, Some(&instance))?;
            send_operation_result(
                &sender,
                operation_id,
                true,
                format!("ValkeyCluster {namespace}/{} found", instance.name),
                Some(resource),
            )
            .await
        }
        None => {
            send_operation_result(
                &sender,
                operation_id,
                false,
                format!("ValkeyCluster {namespace}/{} not found", instance.name),
                Some(instance),
            )
            .await
        }
    }
}

async fn refresh_inventory(
    config: &ValkeyOperatorConfig,
    client: Client,
    sender: OutboundSender,
    operation_id: String,
) -> Result<()> {
    let mut instances = Vec::new();
    if config.watch_namespaces.is_empty() {
        let api = Api::<ValkeyCluster>::all(client);
        for cluster in api.list(&ListParams::default()).await?.items {
            if let Some(resource) = owned_cluster_to_resource(&cluster, &config.workspace_id)? {
                instances.push(resource);
            }
        }
    } else {
        for namespace in &config.watch_namespaces {
            let api = Api::<ValkeyCluster>::namespaced(client.clone(), namespace);
            for cluster in api.list(&ListParams::default()).await?.items {
                if let Some(resource) = owned_cluster_to_resource(&cluster, &config.workspace_id)? {
                    instances.push(resource);
                }
            }
        }
    }

    sender
        .send(OperatorToServer {
            request_id: operation_id,
            payload: Some(operator_to_server::Payload::InventorySnapshot(
                InventorySnapshot {
                    operator_id: config.operator_id(),
                    instances,
                },
            )),
        })
        .await
        .map_err(|_| Error::InvalidState("operator inventory stream closed".to_string()))
}

fn middleware_resource_to_cluster(resource: MiddlewareResource) -> Result<ValkeyCluster> {
    if resource.kind != VALKEY_MIDDLEWARE_KIND {
        return Err(Error::InvalidConfig(format!(
            "unsupported middleware kind {}; expected {VALKEY_MIDDLEWARE_KIND}",
            resource.kind
        )));
    }
    if resource.name.trim().is_empty() {
        return Err(Error::InvalidConfig(
            "middleware resource name is required".to_string(),
        ));
    }
    if resource.id.trim().is_empty() || Uuid::parse_str(&resource.id).is_err() {
        return Err(Error::InvalidConfig(
            "middleware resource id must be a UUID".to_string(),
        ));
    }
    if resource.workspace_id.trim().is_empty() || Uuid::parse_str(&resource.workspace_id).is_err() {
        return Err(Error::InvalidConfig(
            "middleware workspace id must be a UUID".to_string(),
        ));
    }

    let config =
        resource.config.clone().map(struct_to_json).ok_or_else(|| {
            Error::InvalidConfig("ValkeyCluster spec config is required".to_string())
        })?;
    let spec: ValkeyClusterSpec = serde_json::from_value(config)?;
    validate_valkey_spec(&spec)?;

    Ok(ValkeyCluster {
        metadata: ObjectMeta {
            name: Some(resource.name.clone()),
            namespace: Some(defaulted_namespace(&resource)),
            labels: Some(midgard_labels(&resource)),
            ..ObjectMeta::default()
        },
        spec,
        status: None,
    })
}

fn validate_valkey_spec(spec: &ValkeyClusterSpec) -> Result<()> {
    if spec.shards < 1 {
        return Err(Error::InvalidConfig(
            "ValkeyCluster spec.shards must be at least 1".to_string(),
        ));
    }
    if spec.replicas < 0 {
        return Err(Error::InvalidConfig(
            "ValkeyCluster spec.replicas cannot be negative".to_string(),
        ));
    }
    Ok(())
}

fn cluster_to_middleware_resource(
    cluster: &ValkeyCluster,
    fallback: Option<&MiddlewareResource>,
) -> Result<MiddlewareResource> {
    let labels = cluster.metadata.labels.clone().unwrap_or_default();
    let id = labels
        .get(LABEL_MIDDLEWARE_ID)
        .cloned()
        .or_else(|| fallback.map(|resource| resource.id.clone()))
        .unwrap_or_default();
    let workspace_id = labels
        .get(LABEL_WORKSPACE_ID)
        .cloned()
        .or_else(|| fallback.map(|resource| resource.workspace_id.clone()))
        .unwrap_or_default();
    let desired_state = fallback
        .map(|resource| resource.desired_state)
        .unwrap_or(DesiredState::Enabled as i32);
    let archived_at = fallback
        .map(|resource| resource.archived_at.clone())
        .unwrap_or_default();
    let created_at = fallback
        .map(|resource| resource.created_at.clone())
        .unwrap_or_default();
    let updated_at = fallback
        .map(|resource| resource.updated_at.clone())
        .unwrap_or_default();

    Ok(MiddlewareResource {
        id,
        workspace_id,
        kind: VALKEY_MIDDLEWARE_KIND.to_string(),
        name: cluster.name_any(),
        namespace: cluster.namespace().unwrap_or_default(),
        desired_state,
        status: cluster_status(cluster) as i32,
        config: Some(json_to_struct(&serde_json::to_value(&cluster.spec)?)),
        archived_at,
        created_at,
        updated_at,
    })
}

fn owned_cluster_to_resource(
    cluster: &ValkeyCluster,
    workspace_id: &str,
) -> Result<Option<MiddlewareResource>> {
    let labels = cluster.metadata.labels.clone().unwrap_or_default();
    if labels.get(LABEL_MIDDLEWARE_KIND).map(String::as_str) != Some(VALKEY_MIDDLEWARE_KIND) {
        return Ok(None);
    }
    let (Some(middleware_id), Some(resource_workspace_id)) = (
        labels.get(LABEL_MIDDLEWARE_ID),
        labels.get(LABEL_WORKSPACE_ID),
    ) else {
        return Ok(None);
    };
    if resource_workspace_id != workspace_id {
        return Ok(None);
    }
    if Uuid::parse_str(middleware_id).is_err() || Uuid::parse_str(resource_workspace_id).is_err() {
        return Ok(None);
    }

    cluster_to_middleware_resource(cluster, None).map(Some)
}

fn cluster_status(cluster: &ValkeyCluster) -> MiddlewareStatus {
    if cluster.metadata.deletion_timestamp.is_some() {
        return MiddlewareStatus::Stopped;
    }
    match cluster.status.as_ref().map(|status| &status.state) {
        Some(ClusterState::Ready) => MiddlewareStatus::Running,
        Some(ClusterState::Degraded | ClusterState::Failed) => MiddlewareStatus::Degraded,
        Some(ClusterState::Initializing | ClusterState::Reconciling) | None => {
            MiddlewareStatus::Pending
        }
    }
}

fn midgard_labels(resource: &MiddlewareResource) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            LABEL_WORKSPACE_ID.to_string(),
            resource.workspace_id.clone(),
        ),
        (LABEL_MIDDLEWARE_ID.to_string(), resource.id.clone()),
        (
            LABEL_MIDDLEWARE_KIND.to_string(),
            VALKEY_MIDDLEWARE_KIND.to_string(),
        ),
    ])
}

fn is_delete_intent(resource: &MiddlewareResource) -> bool {
    !resource.archived_at.is_empty()
        || DesiredState::try_from(resource.desired_state).unwrap_or(DesiredState::Enabled)
            == DesiredState::Disabled
}

fn defaulted_namespace(resource: &MiddlewareResource) -> String {
    if resource.namespace.trim().is_empty() {
        "default".to_string()
    } else {
        resource.namespace.clone()
    }
}

pub struct ValkeyResourceAdapter;

impl OperatorResourceAdapter for ValkeyResourceAdapter {
    type Resource = ValkeyCluster;
    type Error = Error;

    fn middleware_kind(&self) -> &str {
        VALKEY_MIDDLEWARE_KIND
    }

    fn resource_from_middleware(&self, resource: MiddlewareResource) -> Result<Self::Resource> {
        middleware_resource_to_cluster(resource)
    }

    fn middleware_from_resource(
        &self,
        resource: &Self::Resource,
        fallback: Option<&MiddlewareResource>,
    ) -> Result<MiddlewareResource> {
        cluster_to_middleware_resource(resource, fallback)
    }
}

async fn send_operation_status(
    sender: &OutboundSender,
    operation_id: &str,
    instance_id: &str,
    status: MiddlewareStatus,
    message: String,
) -> Result<()> {
    sender
        .send(OperatorToServer {
            request_id: operation_id.to_string(),
            payload: Some(operator_to_server::Payload::OperationStatus(
                OperationStatus {
                    operation_id: operation_id.to_string(),
                    instance_id: instance_id.to_string(),
                    status: status as i32,
                    message,
                },
            )),
        })
        .await
        .map_err(|_| Error::InvalidState("operator status stream closed".to_string()))
}

async fn send_operation_result(
    sender: &OutboundSender,
    operation_id: String,
    success: bool,
    message: String,
    instance: Option<MiddlewareResource>,
) -> Result<()> {
    sender
        .send(OperatorToServer {
            request_id: operation_id.clone(),
            payload: Some(operator_to_server::Payload::OperationResult(
                OperationResult {
                    operation_id,
                    success,
                    message,
                    instance,
                },
            )),
        })
        .await
        .map_err(|_| Error::InvalidState("operator result stream closed".to_string()))
}

#[cfg(test)]
mod tests {
    use midgard_protocol::json_to_struct;
    use serde_json::json;

    use super::*;

    fn resource(config: serde_json::Value) -> MiddlewareResource {
        MiddlewareResource {
            id: Uuid::new_v4().to_string(),
            workspace_id: Uuid::new_v4().to_string(),
            kind: VALKEY_MIDDLEWARE_KIND.to_string(),
            name: "cache".to_string(),
            namespace: "data".to_string(),
            desired_state: DesiredState::Enabled as i32,
            status: MiddlewareStatus::Pending as i32,
            config: Some(json_to_struct(&config)),
            archived_at: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn middleware_resource_maps_to_valkey_cluster() {
        let resource = resource(json!({"shards": 3, "replicas": 1}));

        let cluster = middleware_resource_to_cluster(resource.clone()).unwrap();

        assert_eq!(cluster.name_any(), "cache");
        assert_eq!(cluster.namespace().as_deref(), Some("data"));
        assert_eq!(cluster.spec.shards, 3);
        assert_eq!(cluster.spec.replicas, 1);
        assert_eq!(
            cluster
                .metadata
                .labels
                .as_ref()
                .unwrap()
                .get(LABEL_MIDDLEWARE_ID),
            Some(&resource.id)
        );
    }

    #[test]
    fn valkey_cluster_requires_at_least_one_shard() {
        let err = middleware_resource_to_cluster(resource(json!({"shards": 0, "replicas": 1})))
            .unwrap_err();

        assert!(err.to_string().contains("spec.shards"));
    }

    #[test]
    fn cluster_state_maps_to_middleware_status() {
        let mut cluster = ValkeyCluster::new(
            "cache",
            ValkeyClusterSpec {
                shards: 1,
                ..ValkeyClusterSpec::default()
            },
        );

        assert_eq!(cluster_status(&cluster), MiddlewareStatus::Pending);

        cluster.status = Some(crate::api::ValkeyClusterStatus {
            state: ClusterState::Ready,
            ..crate::api::ValkeyClusterStatus::default()
        });
        assert_eq!(cluster_status(&cluster), MiddlewareStatus::Running);

        cluster.status = Some(crate::api::ValkeyClusterStatus {
            state: ClusterState::Failed,
            ..crate::api::ValkeyClusterStatus::default()
        });
        assert_eq!(cluster_status(&cluster), MiddlewareStatus::Degraded);
    }

    #[test]
    fn owned_cluster_requires_current_workspace_label() {
        let workspace_id = Uuid::new_v4().to_string();
        let other_workspace_id = Uuid::new_v4().to_string();
        let mut cluster = ValkeyCluster::new(
            "cache",
            ValkeyClusterSpec {
                shards: 1,
                ..ValkeyClusterSpec::default()
            },
        );
        cluster.metadata.labels = Some(BTreeMap::from([
            (
                LABEL_MIDDLEWARE_KIND.to_string(),
                VALKEY_MIDDLEWARE_KIND.to_string(),
            ),
            (LABEL_MIDDLEWARE_ID.to_string(), Uuid::new_v4().to_string()),
            (LABEL_WORKSPACE_ID.to_string(), other_workspace_id),
        ]));

        assert!(
            owned_cluster_to_resource(&cluster, &workspace_id)
                .unwrap()
                .is_none()
        );
    }
}
