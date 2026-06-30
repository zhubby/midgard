pub mod cluster;
pub mod config;
pub mod node;
pub mod resources;
pub mod users;

use std::collections::BTreeMap;
use std::fmt::Debug;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Condition, ObjectMeta, OwnerReference, Time};
use kube::api::{Api, Patch, PatchParams, Resource};
use kube::{Client, ResourceExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::api::{ValkeyCluster, ValkeyNode};
use crate::error::Result;

pub const FIELD_MANAGER: &str = "valkey-operator-rust";
pub const APP_NAME: &str = "valkey";
pub const RESOURCE_PREFIX: &str = "valkey-";

pub const DEFAULT_PORT: i32 = 6379;
pub const DEFAULT_CLUSTER_BUS_PORT: i32 = 16379;
pub const DEFAULT_IMAGE: &str = "valkey/valkey:9.0.0";
pub const DEFAULT_EXPORTER_IMAGE: &str = "oliver006/redis_exporter:v1.80.0";
pub const DEFAULT_EXPORTER_PORT: i32 = 9121;

pub const ACL_SECRET_TYPE: &str = "valkey.io/acl";
pub const LABEL_CLUSTER: &str = "valkey.io/cluster";
pub const LABEL_SHARD_INDEX: &str = "valkey.io/shard-index";
pub const LABEL_NODE_INDEX: &str = "valkey.io/node-index";

pub const ROLE_PRIMARY: &str = "primary";
pub const ROLE_REPLICA: &str = "replica";
pub const ROLE_MASTER: &str = "master";
pub const ROLE_SLAVE: &str = "slave";

pub const TLS_VOLUME_NAME: &str = "tls-certs";
pub const TLS_CERT_MOUNT_PATH: &str = "/tls";
pub const TLS_SECRET_KEY_CA: &str = "ca.crt";
pub const TLS_SECRET_KEY_CERT: &str = "tls.crt";
pub const TLS_SECRET_KEY_KEY: &str = "tls.key";
pub const DATA_VOLUME_NAME: &str = "data";
pub const DATA_MOUNT_PATH: &str = "/data";

pub const HASH_ANNOTATION_KEY: &str = "valkey.io/internal-acl-hash";
pub const CONFIG_HASH_KEY: &str = "valkey.io/config-hash";
pub const SCRIPTS_HASH_KEY: &str = "valkey.io/script-hash";
pub const CONFIG_FILE_KEY: &str = "valkey.conf";
pub const READINESS_SCRIPT_KEY: &str = "readiness-check.sh";
pub const LIVENESS_SCRIPT_KEY: &str = "liveness-check.sh";

#[derive(Clone)]
pub struct Context {
    pub client: Client,
    pub watch_namespaces: Vec<String>,
}

pub fn base_labels(name: &str, component: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("app.kubernetes.io/name".to_string(), APP_NAME.to_string()),
        ("app.kubernetes.io/instance".to_string(), name.to_string()),
        (
            "app.kubernetes.io/component".to_string(),
            component.to_string(),
        ),
        (
            "app.kubernetes.io/part-of".to_string(),
            APP_NAME.to_string(),
        ),
        (
            "app.kubernetes.io/managed-by".to_string(),
            "valkey-operator".to_string(),
        ),
    ])
}

pub fn cluster_labels(cluster: &ValkeyCluster) -> BTreeMap<String, String> {
    let mut labels = cluster.metadata.labels.clone().unwrap_or_default();
    labels.extend(base_labels(&cluster.name_any(), "valkey-cluster"));
    labels
}

pub fn cluster_annotations(cluster: &ValkeyCluster) -> BTreeMap<String, String> {
    cluster.metadata.annotations.clone().unwrap_or_default()
}

pub fn headless_service_name(cluster_name: &str) -> String {
    format!("{RESOURCE_PREFIX}{cluster_name}")
}

pub fn server_config_map_name(name: &str) -> String {
    format!("{RESOURCE_PREFIX}{name}")
}

pub fn valkey_node_name(cluster_name: &str, shard_index: i32, node_index: i32) -> String {
    format!("{cluster_name}-{shard_index}-{node_index}")
}

pub fn valkey_node_resource_name(node: &ValkeyNode) -> String {
    format!("{RESOURCE_PREFIX}{}", node.name_any())
}

pub fn valkey_node_pvc_name(node: &ValkeyNode) -> String {
    format!("{}-data", valkey_node_resource_name(node))
}

pub fn valkey_node_labels(node: &ValkeyNode) -> BTreeMap<String, String> {
    let mut labels = base_labels(&node.name_any(), "valkey-node");
    if let Some(existing) = &node.metadata.labels {
        for key in [LABEL_CLUSTER, LABEL_SHARD_INDEX, LABEL_NODE_INDEX] {
            if let Some(value) = existing.get(key) {
                labels.insert(key.to_string(), value.clone());
            }
        }
    }
    labels
}

pub fn owner_reference<K>(owner: &K) -> Option<OwnerReference>
where
    K: Resource<DynamicType = ()>,
{
    owner.controller_owner_ref(&())
}

pub fn object_meta(
    name: impl Into<String>,
    namespace: impl Into<String>,
    labels: BTreeMap<String, String>,
    annotations: BTreeMap<String, String>,
    owner: Option<OwnerReference>,
) -> ObjectMeta {
    ObjectMeta {
        name: Some(name.into()),
        namespace: Some(namespace.into()),
        labels: (!labels.is_empty()).then_some(labels),
        annotations: (!annotations.is_empty()).then_some(annotations),
        owner_references: owner.map(|owner| vec![owner]),
        ..ObjectMeta::default()
    }
}

pub async fn apply<K>(api: &Api<K>, name: &str, obj: &K) -> Result<K>
where
    K: Clone + Debug + DeserializeOwned + Serialize,
{
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    let mut patch = serde_json::to_value(obj)?;
    sanitize_apply_patch(&mut patch);
    Ok(api.patch(name, &pp, &Patch::Apply(&patch)).await?)
}

fn sanitize_apply_patch(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.remove("status");
    let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    for field in [
        "creationTimestamp",
        "deletionGracePeriodSeconds",
        "deletionTimestamp",
        "generation",
        "managedFields",
        "resourceVersion",
        "selfLink",
        "uid",
    ] {
        metadata.remove(field);
    }
}

pub async fn patch_status<K, S>(api: &Api<K>, name: &str, status: &S) -> Result<K>
where
    K: Clone + Debug + DeserializeOwned,
    S: Serialize + Debug,
{
    let pp = PatchParams::default();
    let patch = serde_json::json!({ "status": status });
    Ok(api.patch_status(name, &pp, &Patch::Merge(&patch)).await?)
}

pub fn set_condition(
    conditions: &mut Vec<Condition>,
    generation: i64,
    cond_type: &str,
    reason: &str,
    message: &str,
    status: &str,
) {
    let now = Time(k8s_openapi::jiff::Timestamp::now());
    if let Some(existing) = conditions
        .iter_mut()
        .find(|condition| condition.type_ == cond_type)
    {
        if existing.status != status || existing.reason != reason || existing.message != message {
            existing.last_transition_time = now;
        }
        existing.status = status.to_string();
        existing.reason = reason.to_string();
        existing.message = message.to_string();
        existing.observed_generation = Some(generation);
        return;
    }
    conditions.push(Condition {
        type_: cond_type.to_string(),
        status: status.to_string(),
        reason: reason.to_string(),
        message: message.to_string(),
        observed_generation: Some(generation),
        last_transition_time: now,
    });
}

pub fn remove_condition(conditions: &mut Vec<Condition>, cond_type: &str) {
    conditions.retain(|condition| condition.type_ != cond_type);
}

pub fn remove_condition_if_reason(conditions: &mut Vec<Condition>, cond_type: &str, reason: &str) {
    conditions.retain(|condition| !(condition.type_ == cond_type && condition.reason == reason));
}

pub fn find_condition<'a>(conditions: &'a [Condition], cond_type: &str) -> Option<&'a Condition> {
    conditions
        .iter()
        .find(|condition| condition.type_ == cond_type)
}

pub fn label_selector(labels: &BTreeMap<String, String>) -> String {
    labels
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn node_role_and_shard(address: &str, nodes: &[ValkeyNode]) -> (String, i32) {
    let Some(node) = nodes.iter().find(|node| {
        node.status
            .as_ref()
            .is_some_and(|status| status.pod_ip == address)
    }) else {
        return (String::new(), -1);
    };
    let labels = node.metadata.labels.clone().unwrap_or_default();
    let shard_index = labels
        .get(LABEL_SHARD_INDEX)
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(-1);
    let node_index = labels
        .get(LABEL_NODE_INDEX)
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(-1);
    if shard_index < 0 || node_index < 0 {
        return (String::new(), -1);
    }
    if node_index == 0 {
        (ROLE_PRIMARY.to_string(), shard_index)
    } else {
        (ROLE_REPLICA.to_string(), shard_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitize_apply_patch_removes_server_owned_fields() {
        let mut value = json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": "example",
                "namespace": "default",
                "managedFields": [],
                "resourceVersion": "42",
                "uid": "abc",
                "creationTimestamp": "2026-06-11T00:00:00Z",
                "generation": 2
            },
            "status": { "ready": true },
            "data": {}
        });

        sanitize_apply_patch(&mut value);

        assert_eq!(value.get("status"), None);
        let metadata = value
            .get("metadata")
            .and_then(Value::as_object)
            .expect("metadata should remain");
        assert_eq!(
            metadata.get("name").and_then(Value::as_str),
            Some("example")
        );
        assert!(!metadata.contains_key("managedFields"));
        assert!(!metadata.contains_key("resourceVersion"));
        assert!(!metadata.contains_key("uid"));
        assert!(!metadata.contains_key("creationTimestamp"));
        assert!(!metadata.contains_key("generation"));
    }
}
