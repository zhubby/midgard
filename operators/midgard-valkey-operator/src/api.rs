use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{
    Affinity, Container, LocalObjectReference, ResourceRequirements, Toleration,
    TopologySpreadConstraint,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition;
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
pub enum ClusterState {
    #[default]
    Initializing,
    Reconciling,
    Ready,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
pub enum PdbPolicy {
    #[serde(rename = "Managed")]
    #[default]
    Managed,
    #[serde(rename = "Disabled")]
    Disabled,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
pub enum WorkloadType {
    #[serde(rename = "StatefulSet")]
    #[default]
    StatefulSet,
    #[serde(rename = "Deployment")]
    Deployment,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
pub enum PersistenceReclaimPolicy {
    #[serde(rename = "Retain")]
    #[default]
    Retain,
    #[serde(rename = "Delete")]
    Delete,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersistenceSpec {
    pub size: Quantity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_class_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reclaim_policy: Option<PersistenceReclaimPolicy>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CertificateRef {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub secret_name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
pub struct TlsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate: Option<CertificateRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Default)]
pub struct ExporterSpec {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceRequirements>,
    #[serde(default)]
    pub enabled: bool,
}

impl ExporterSpec {
    pub fn enabled_default() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PasswordSecretSpec {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
pub struct CommandsAclSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct KeysAclSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_write: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_only: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write_only: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
pub struct ChannelsAclSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct UserAclSpec {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub password_secret: PasswordSecretSpec,
    #[serde(default, rename = "nopass")]
    pub no_password: bool,
    #[serde(default, rename = "resetpass")]
    pub reset_pass: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub commands: CommandsAclSpec,
    #[serde(default, skip_serializing_if = "is_default")]
    pub keys: KeysAclSpec,
    #[serde(default, skip_serializing_if = "is_default")]
    pub channels: ChannelsAclSpec,
    #[serde(
        default,
        rename = "permissions",
        skip_serializing_if = "String::is_empty"
    )]
    pub raw_acl: String,
}

impl Default for UserAclSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            password_secret: PasswordSecretSpec::default(),
            no_password: false,
            reset_pass: false,
            commands: CommandsAclSpec::default(),
            keys: KeysAclSpec::default(),
            channels: ChannelsAclSpec::default(),
            raw_acl: String::new(),
        }
    }
}

#[derive(CustomResource, Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Default)]
#[kube(
    group = "valkey.io",
    version = "v1alpha1",
    kind = "ValkeyCluster",
    plural = "valkeyclusters",
    namespaced,
    status = "ValkeyClusterStatus",
    derive = "PartialEq",
    printcolumn = r#"{"name":"State", "type":"string", "jsonPath":".status.state"}"#,
    printcolumn = r#"{"name":"Reason", "type":"string", "jsonPath":".status.reason"}"#,
    printcolumn = r#"{"name":"ReadyShards", "type":"integer", "jsonPath":".status.readyShards", "priority":1}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ValkeyClusterSpec {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_pull_secrets: Vec<LocalObjectReference>,
    #[serde(default)]
    pub shards: i32,
    #[serde(default)]
    pub replicas: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceRequirements>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tolerations: Vec<Toleration>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_selector: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affinity: Option<Affinity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topology_spread_constraints: Vec<TopologySpreadConstraint>,
    #[serde(
        default = "ExporterSpec::enabled_default",
        skip_serializing_if = "is_default"
    )]
    pub exporter: ExporterSpec,
    #[serde(default, skip_serializing_if = "is_default")]
    pub workload_type: WorkloadType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence: Option<PersistenceSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<UserAclSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<Container>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, String>,
    #[serde(default, rename = "tls", skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsConfig>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub pod_disruption_budget: PdbPolicy,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ValkeyClusterStatus {
    #[serde(default, skip_serializing_if = "is_default")]
    pub state: ClusterState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default)]
    pub shards: i32,
    #[serde(default)]
    pub ready_shards: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
}

#[derive(CustomResource, Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Default)]
#[kube(
    group = "valkey.io",
    version = "v1alpha1",
    kind = "ValkeyNode",
    plural = "valkeynodes",
    namespaced,
    status = "ValkeyNodeStatus",
    derive = "PartialEq",
    printcolumn = r#"{"name":"Ready", "type":"boolean", "jsonPath":".status.ready"}"#,
    printcolumn = r#"{"name":"Role", "type":"string", "jsonPath":".status.role"}"#,
    printcolumn = r#"{"name":"Pod", "type":"string", "jsonPath":".status.podName"}"#,
    printcolumn = r#"{"name":"IP", "type":"string", "jsonPath":".status.podIP", "priority":1}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ValkeyNodeSpec {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_pull_secrets: Vec<LocalObjectReference>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub workload_type: WorkloadType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence: Option<PersistenceSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceRequirements>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_selector: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affinity: Option<Affinity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tolerations: Vec<Toleration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topology_spread_constraints: Vec<TopologySpreadConstraint>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub exporter: ExporterSpec,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_config_map_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_config_hash: String,
    #[serde(
        default,
        rename = "usersACLSecretName",
        skip_serializing_if = "String::is_empty"
    )]
    pub users_acl_secret_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<Container>,
    #[serde(default, rename = "tls", skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ValkeyNodeStatus {
    #[serde(default)]
    pub observed_generation: i64,
    #[serde(default)]
    pub ready: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pod_name: String,
    #[serde(default, rename = "podIP", skip_serializing_if = "String::is_empty")]
    pub pod_ip: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
}

pub const CONDITION_READY: &str = "Ready";
pub const CONDITION_PROGRESSING: &str = "Progressing";
pub const CONDITION_DEGRADED: &str = "Degraded";
pub const CONDITION_CLUSTER_FORMED: &str = "ClusterFormed";
pub const CONDITION_SLOTS_ASSIGNED: &str = "SlotsAssigned";

pub const VALKEY_NODE_CONDITION_READY: &str = "Ready";
pub const VALKEY_NODE_CONDITION_PVC_READY: &str = "PersistentVolumeClaimReady";
pub const VALKEY_NODE_CONDITION_PVC_SIZE_READY: &str = "PersistentVolumeClaimSizeReady";
pub const VALKEY_NODE_CONDITION_LIVE_CONFIG_APPLIED: &str = "LiveConfigApplied";

pub fn default_true() -> bool {
    true
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}
