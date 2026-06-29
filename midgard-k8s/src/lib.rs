use async_trait::async_trait;
use midgard_core::MidgardResult;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClusterHealth {
    pub ready: bool,
    pub context: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkloadSummary {
    pub namespace: String,
    pub name: String,
    pub kind: String,
    pub ready: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodSummary {
    pub namespace: String,
    pub name: String,
    pub phase: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KubernetesEvent {
    pub namespace: String,
    pub involved_object: String,
    pub reason: String,
    pub message: String,
}

#[async_trait]
pub trait KubernetesClient: Send + Sync {
    async fn cluster_health(&self) -> MidgardResult<ClusterHealth>;
    async fn list_namespaces(&self) -> MidgardResult<Vec<String>>;
    async fn list_workloads(&self, namespace: &str) -> MidgardResult<Vec<WorkloadSummary>>;
    async fn list_pods(&self, namespace: &str) -> MidgardResult<Vec<PodSummary>>;
    async fn read_events(&self, namespace: &str) -> MidgardResult<Vec<KubernetesEvent>>;
}

#[derive(Default)]
pub struct MockKubernetesClient;

#[async_trait]
impl KubernetesClient for MockKubernetesClient {
    async fn cluster_health(&self) -> MidgardResult<ClusterHealth> {
        Ok(ClusterHealth {
            ready: true,
            context: "mock".to_string(),
            message: "mock cluster is reachable".to_string(),
        })
    }

    async fn list_namespaces(&self) -> MidgardResult<Vec<String>> {
        Ok(vec!["default".to_string(), "midgard-system".to_string()])
    }

    async fn list_workloads(&self, namespace: &str) -> MidgardResult<Vec<WorkloadSummary>> {
        Ok(vec![WorkloadSummary {
            namespace: namespace.to_string(),
            name: "redis".to_string(),
            kind: "StatefulSet".to_string(),
            ready: "1/1".to_string(),
        }])
    }

    async fn list_pods(&self, namespace: &str) -> MidgardResult<Vec<PodSummary>> {
        Ok(vec![PodSummary {
            namespace: namespace.to_string(),
            name: "redis-0".to_string(),
            phase: "Running".to_string(),
        }])
    }

    async fn read_events(&self, namespace: &str) -> MidgardResult<Vec<KubernetesEvent>> {
        Ok(vec![KubernetesEvent {
            namespace: namespace.to_string(),
            involved_object: "statefulset/redis".to_string(),
            reason: "SuccessfulCreate".to_string(),
            message: "created pod redis-0".to_string(),
        }])
    }
}
