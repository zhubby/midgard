use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_LOG_TAIL: u32 = 100;
pub(crate) const MAX_LOG_TAIL: u32 = 1000;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum DockerOperation {
    Info,
    Version,
    ListContainers {
        all: bool,
    },
    InspectContainer {
        container: String,
        size: bool,
    },
    ContainerLogs {
        container: String,
        tail: u32,
        timestamps: bool,
        stdout: bool,
        stderr: bool,
    },
    CreateContainer(ContainerCreateRequest),
    StartContainer {
        container: String,
    },
    StopContainer {
        container: String,
        timeout_seconds: Option<i32>,
    },
    RestartContainer {
        container: String,
        timeout_seconds: Option<i32>,
    },
    RemoveContainer {
        container: String,
        force: bool,
        volumes: bool,
    },
    PruneContainers,
    ListImages {
        all: bool,
    },
    InspectImage {
        image: String,
    },
    PullImage {
        image: String,
        tag: Option<String>,
    },
    RemoveImage {
        image: String,
        force: bool,
        no_prune: bool,
    },
    PruneImages,
    ListNetworks,
    InspectNetwork {
        network: String,
    },
    CreateNetwork(NetworkCreateToolRequest),
    ConnectNetwork {
        network: String,
        container: String,
    },
    DisconnectNetwork {
        network: String,
        container: String,
        force: bool,
    },
    RemoveNetwork {
        network: String,
    },
    PruneNetworks,
    ListVolumes,
    InspectVolume {
        volume: String,
    },
    CreateVolume(VolumeCreateToolRequest),
    RemoveVolume {
        volume: String,
        force: bool,
    },
    PruneVolumes,
    SystemPrune,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerCreateRequest {
    #[serde(default)]
    pub name: Option<String>,
    pub image: String,
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub ports: Vec<ContainerPortBinding>,
    #[serde(default)]
    pub binds: Vec<String>,
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub memory_bytes: Option<i64>,
    #[serde(default)]
    pub publish_all_ports: bool,
    #[serde(default)]
    pub restart_policy: Option<String>,
    #[serde(default)]
    pub auto_remove: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerPortBinding {
    pub container_port: u16,
    pub host_port: u16,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub host_ip: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkCreateToolRequest {
    pub name: String,
    #[serde(default)]
    pub driver: Option<String>,
    #[serde(default)]
    pub internal: Option<bool>,
    #[serde(default)]
    pub attachable: Option<bool>,
    #[serde(default)]
    pub enable_ipv6: Option<bool>,
    #[serde(default)]
    pub options: Option<HashMap<String, String>>,
    #[serde(default)]
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeCreateToolRequest {
    pub name: String,
    #[serde(default)]
    pub driver: Option<String>,
    #[serde(default)]
    pub driver_options: Option<HashMap<String, String>>,
    #[serde(default)]
    pub labels: Option<HashMap<String, String>>,
}

fn default_protocol() -> String {
    "tcp".to_string()
}
