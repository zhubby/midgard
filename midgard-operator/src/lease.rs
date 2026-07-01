use std::time::Duration;

use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{MicroTime, ObjectMeta};
use kube::Client;
use kube::api::{Api, PostParams};
use tokio::time::sleep;

use crate::{OperatorError, OperatorResult};

pub const DEFAULT_LOCK_NAMESPACE: &str = "midgard-operator-system";
pub const DEFAULT_LOCK_NAME: &str = "midgard-operator";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaseConfig {
    pub namespace: String,
    pub name: String,
    pub lease_duration: Duration,
    pub renew_interval: Duration,
    pub retry_interval: Duration,
}

impl Default for LeaseConfig {
    fn default() -> Self {
        Self {
            namespace: DEFAULT_LOCK_NAMESPACE.to_string(),
            name: DEFAULT_LOCK_NAME.to_string(),
            lease_duration: Duration::from_secs(15),
            renew_interval: Duration::from_secs(5),
            retry_interval: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaseDecision {
    Acquire { transitions: i32 },
    Renew,
    Wait { holder: String },
}

pub fn decide_lease(
    holder_identity: &str,
    current_holder: Option<&str>,
    renew_time_unix_seconds: Option<i64>,
    lease_duration_seconds: i64,
    now_unix_seconds: i64,
    current_transitions: i32,
) -> LeaseDecision {
    let Some(current_holder) = current_holder.filter(|holder| !holder.is_empty()) else {
        return LeaseDecision::Acquire {
            transitions: current_transitions + 1,
        };
    };

    if current_holder == holder_identity {
        return LeaseDecision::Renew;
    }

    let expired = renew_time_unix_seconds
        .map(|renewed_at| now_unix_seconds.saturating_sub(renewed_at) > lease_duration_seconds)
        .unwrap_or(true);
    if expired {
        return LeaseDecision::Acquire {
            transitions: current_transitions + 1,
        };
    }

    LeaseDecision::Wait {
        holder: current_holder.to_string(),
    }
}

#[derive(Clone)]
pub struct LeaseGuard {
    client: Client,
    config: LeaseConfig,
    holder_identity: String,
}

impl LeaseGuard {
    pub async fn acquire(
        client: Client,
        config: LeaseConfig,
        holder_identity: impl Into<String>,
    ) -> OperatorResult<Self> {
        let holder_identity = holder_identity.into();
        if holder_identity.trim().is_empty() {
            return Err(OperatorError::InvalidConfig(
                "lease holder identity is required".to_string(),
            ));
        }

        let guard = Self {
            client,
            config,
            holder_identity,
        };
        guard.acquire_once().await?;
        Ok(guard)
    }

    pub fn holder_identity(&self) -> &str {
        &self.holder_identity
    }

    pub async fn renew_until_lost(&self) -> OperatorResult<()> {
        loop {
            sleep(self.config.renew_interval).await;
            self.renew_once().await?;
        }
    }

    pub async fn release(&self) -> OperatorResult<()> {
        let api = self.api();
        let Some(mut lease) = api.get_opt(&self.config.name).await? else {
            return Ok(());
        };
        let spec = lease.spec.clone().unwrap_or_default();
        if !matches!(self.decision(&spec), LeaseDecision::Renew) {
            return Ok(());
        }

        lease.spec = Some(released_lease_spec(spec));
        match api
            .replace(&self.config.name, &PostParams::default(), &lease)
            .await
        {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(err)) if err.code == 409 => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    async fn acquire_once(&self) -> OperatorResult<()> {
        let api = self.api();
        let Some(mut lease) = api.get_opt(&self.config.name).await? else {
            let lease = self.build_lease(0);
            match api.create(&PostParams::default(), &lease).await {
                Ok(_) => return Ok(()),
                Err(kube::Error::Api(err)) if err.code == 409 => {
                    return Err(OperatorError::LeaseHeld("unknown".to_string()));
                }
                Err(err) => return Err(err.into()),
            }
        };

        let spec = lease.spec.clone().unwrap_or_default();
        match self.decision(&spec) {
            LeaseDecision::Acquire { transitions } => {
                lease.spec = Some(self.lease_spec(transitions, true));
                match api
                    .replace(&self.config.name, &PostParams::default(), &lease)
                    .await
                {
                    Ok(_) => Ok(()),
                    Err(kube::Error::Api(err)) if err.code == 409 => {
                        Err(OperatorError::LeaseHeld("unknown".to_string()))
                    }
                    Err(err) => Err(err.into()),
                }
            }
            LeaseDecision::Renew => {
                lease.spec =
                    Some(self.lease_spec(spec.lease_transitions.unwrap_or_default(), false));
                match api
                    .replace(&self.config.name, &PostParams::default(), &lease)
                    .await
                {
                    Ok(_) => Ok(()),
                    Err(kube::Error::Api(err)) if err.code == 409 => {
                        Err(OperatorError::LeaseHeld("unknown".to_string()))
                    }
                    Err(err) => Err(err.into()),
                }
            }
            LeaseDecision::Wait { holder } => Err(OperatorError::LeaseHeld(holder)),
        }
    }

    async fn renew_once(&self) -> OperatorResult<()> {
        let api = self.api();
        let mut lease = api
            .get_opt(&self.config.name)
            .await?
            .ok_or_else(|| OperatorError::LeaseLost("lease object no longer exists".to_string()))?;
        let spec = lease.spec.clone().unwrap_or_default();
        match self.decision(&spec) {
            LeaseDecision::Renew => {
                lease.spec =
                    Some(self.lease_spec(spec.lease_transitions.unwrap_or_default(), false));
                match api
                    .replace(&self.config.name, &PostParams::default(), &lease)
                    .await
                {
                    Ok(_) => Ok(()),
                    Err(kube::Error::Api(err)) if err.code == 409 => Err(OperatorError::LeaseLost(
                        "lease changed during renewal".to_string(),
                    )),
                    Err(err) => Err(err.into()),
                }
            }
            LeaseDecision::Acquire { .. } => Err(OperatorError::LeaseLost(
                "lease expired before renewal completed".to_string(),
            )),
            LeaseDecision::Wait { holder } => Err(OperatorError::LeaseLost(format!(
                "lease is now held by {holder}"
            ))),
        }
    }

    fn decision(&self, spec: &LeaseSpec) -> LeaseDecision {
        decide_lease(
            &self.holder_identity,
            spec.holder_identity.as_deref(),
            spec.renew_time.as_ref().map(micro_time_unix_seconds),
            self.config.lease_duration.as_secs() as i64,
            now_unix_seconds(),
            spec.lease_transitions.unwrap_or_default(),
        )
    }

    fn build_lease(&self, transitions: i32) -> Lease {
        Lease {
            metadata: ObjectMeta {
                name: Some(self.config.name.clone()),
                namespace: Some(self.config.namespace.clone()),
                ..ObjectMeta::default()
            },
            spec: Some(self.lease_spec(transitions, true)),
        }
    }

    fn lease_spec(&self, transitions: i32, acquired: bool) -> LeaseSpec {
        let now = now_micro_time();
        LeaseSpec {
            acquire_time: acquired.then_some(now.clone()),
            holder_identity: Some(self.holder_identity.clone()),
            lease_duration_seconds: Some(self.config.lease_duration.as_secs() as i32),
            lease_transitions: Some(transitions),
            renew_time: Some(now),
            ..LeaseSpec::default()
        }
    }

    fn api(&self) -> Api<Lease> {
        Api::namespaced(self.client.clone(), &self.config.namespace)
    }
}

fn released_lease_spec(mut spec: LeaseSpec) -> LeaseSpec {
    spec.holder_identity = None;
    spec.acquire_time = None;
    spec.renew_time = None;
    spec
}

fn now_micro_time() -> MicroTime {
    MicroTime(k8s_openapi::jiff::Timestamp::now())
}

fn now_unix_seconds() -> i64 {
    k8s_openapi::jiff::Timestamp::now().as_second()
}

fn micro_time_unix_seconds(value: &MicroTime) -> i64 {
    value.0.as_second()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_lease_spec_clears_holder_identity_and_timestamps() {
        let now = now_micro_time();
        let released = released_lease_spec(LeaseSpec {
            acquire_time: Some(now.clone()),
            holder_identity: Some("midgard-valkey-operator".to_string()),
            lease_duration_seconds: Some(15),
            lease_transitions: Some(3),
            renew_time: Some(now),
            ..LeaseSpec::default()
        });

        assert_eq!(released.holder_identity, None);
        assert_eq!(released.acquire_time, None);
        assert_eq!(released.renew_time, None);
        assert_eq!(released.lease_duration_seconds, Some(15));
        assert_eq!(released.lease_transitions, Some(3));
    }
}
