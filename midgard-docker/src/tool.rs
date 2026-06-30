use std::sync::Arc;

use async_trait::async_trait;
use midgard_core::RiskLevel;
use midgard_tools::{Tool, ToolCallContext, ToolDefinition, ToolResult};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{
    client::DockerRuntimeResolver,
    error::DockerToolError,
    operation::{
        ContainerCreateRequest, DEFAULT_LOG_TAIL, DockerOperation, MAX_LOG_TAIL,
        NetworkCreateToolRequest, VolumeCreateToolRequest,
    },
};

#[derive(Clone, Copy)]
pub(crate) enum DockerToolKind {
    Info,
    Version,
    ContainerList,
    ContainerInspect,
    ContainerLogs,
    ContainerCreate,
    ContainerStart,
    ContainerStop,
    ContainerRestart,
    ContainerRemove,
    ContainerPrune,
    ImageList,
    ImageInspect,
    ImagePull,
    ImageRemove,
    ImagePrune,
    NetworkList,
    NetworkInspect,
    NetworkCreate,
    NetworkConnect,
    NetworkDisconnect,
    NetworkRemove,
    NetworkPrune,
    VolumeList,
    VolumeInspect,
    VolumeCreate,
    VolumeRemove,
    VolumePrune,
    SystemPrune,
}

impl DockerToolKind {
    pub(crate) fn all() -> &'static [DockerToolKind] {
        &[
            DockerToolKind::Info,
            DockerToolKind::Version,
            DockerToolKind::ContainerList,
            DockerToolKind::ContainerInspect,
            DockerToolKind::ContainerLogs,
            DockerToolKind::ContainerCreate,
            DockerToolKind::ContainerStart,
            DockerToolKind::ContainerStop,
            DockerToolKind::ContainerRestart,
            DockerToolKind::ContainerRemove,
            DockerToolKind::ContainerPrune,
            DockerToolKind::ImageList,
            DockerToolKind::ImageInspect,
            DockerToolKind::ImagePull,
            DockerToolKind::ImageRemove,
            DockerToolKind::ImagePrune,
            DockerToolKind::NetworkList,
            DockerToolKind::NetworkInspect,
            DockerToolKind::NetworkCreate,
            DockerToolKind::NetworkConnect,
            DockerToolKind::NetworkDisconnect,
            DockerToolKind::NetworkRemove,
            DockerToolKind::NetworkPrune,
            DockerToolKind::VolumeList,
            DockerToolKind::VolumeInspect,
            DockerToolKind::VolumeCreate,
            DockerToolKind::VolumeRemove,
            DockerToolKind::VolumePrune,
            DockerToolKind::SystemPrune,
        ]
    }

    fn name(self) -> &'static str {
        match self {
            DockerToolKind::Info => "docker_info",
            DockerToolKind::Version => "docker_version",
            DockerToolKind::ContainerList => "docker_container_list",
            DockerToolKind::ContainerInspect => "docker_container_inspect",
            DockerToolKind::ContainerLogs => "docker_container_logs",
            DockerToolKind::ContainerCreate => "docker_container_create",
            DockerToolKind::ContainerStart => "docker_container_start",
            DockerToolKind::ContainerStop => "docker_container_stop",
            DockerToolKind::ContainerRestart => "docker_container_restart",
            DockerToolKind::ContainerRemove => "docker_container_remove",
            DockerToolKind::ContainerPrune => "docker_container_prune",
            DockerToolKind::ImageList => "docker_image_list",
            DockerToolKind::ImageInspect => "docker_image_inspect",
            DockerToolKind::ImagePull => "docker_image_pull",
            DockerToolKind::ImageRemove => "docker_image_remove",
            DockerToolKind::ImagePrune => "docker_image_prune",
            DockerToolKind::NetworkList => "docker_network_list",
            DockerToolKind::NetworkInspect => "docker_network_inspect",
            DockerToolKind::NetworkCreate => "docker_network_create",
            DockerToolKind::NetworkConnect => "docker_network_connect",
            DockerToolKind::NetworkDisconnect => "docker_network_disconnect",
            DockerToolKind::NetworkRemove => "docker_network_remove",
            DockerToolKind::NetworkPrune => "docker_network_prune",
            DockerToolKind::VolumeList => "docker_volume_list",
            DockerToolKind::VolumeInspect => "docker_volume_inspect",
            DockerToolKind::VolumeCreate => "docker_volume_create",
            DockerToolKind::VolumeRemove => "docker_volume_remove",
            DockerToolKind::VolumePrune => "docker_volume_prune",
            DockerToolKind::SystemPrune => "docker_system_prune",
        }
    }

    fn description(self) -> &'static str {
        match self {
            DockerToolKind::Info => {
                "Inspect Docker daemon information for the current workspace runtime."
            }
            DockerToolKind::Version => "Inspect Docker daemon and API version details.",
            DockerToolKind::ContainerList => {
                "List Docker containers in the current workspace runtime."
            }
            DockerToolKind::ContainerInspect => "Inspect one Docker container by name or ID.",
            DockerToolKind::ContainerLogs => {
                "Read bounded stdout/stderr logs from one Docker container."
            }
            DockerToolKind::ContainerCreate => {
                "Create a Docker container from a structured subset of container settings."
            }
            DockerToolKind::ContainerStart => "Start one Docker container by name or ID.",
            DockerToolKind::ContainerStop => "Stop one Docker container by name or ID.",
            DockerToolKind::ContainerRestart => "Restart one Docker container by name or ID.",
            DockerToolKind::ContainerRemove => "Remove one Docker container by name or ID.",
            DockerToolKind::ContainerPrune => "Prune stopped Docker containers.",
            DockerToolKind::ImageList => "List Docker images in the current workspace runtime.",
            DockerToolKind::ImageInspect => "Inspect one Docker image by name, tag, or ID.",
            DockerToolKind::ImagePull => "Pull a Docker image by reference.",
            DockerToolKind::ImageRemove => "Remove a Docker image by name, tag, or ID.",
            DockerToolKind::ImagePrune => "Prune unused Docker images.",
            DockerToolKind::NetworkList => "List Docker networks in the current workspace runtime.",
            DockerToolKind::NetworkInspect => "Inspect one Docker network by name or ID.",
            DockerToolKind::NetworkCreate => {
                "Create a Docker network from a structured subset of network settings."
            }
            DockerToolKind::NetworkConnect => "Connect one container to a Docker network.",
            DockerToolKind::NetworkDisconnect => "Disconnect one container from a Docker network.",
            DockerToolKind::NetworkRemove => "Remove one Docker network by name or ID.",
            DockerToolKind::NetworkPrune => "Prune unused Docker networks.",
            DockerToolKind::VolumeList => "List Docker volumes in the current workspace runtime.",
            DockerToolKind::VolumeInspect => "Inspect one Docker volume by name.",
            DockerToolKind::VolumeCreate => {
                "Create a Docker volume from a structured subset of volume settings."
            }
            DockerToolKind::VolumeRemove => "Remove one Docker volume by name.",
            DockerToolKind::VolumePrune => "Prune unused Docker volumes.",
            DockerToolKind::SystemPrune => {
                "Prune unused Docker containers, images, networks, and volumes."
            }
        }
    }

    pub(crate) fn capability_id(self) -> String {
        self.name().replace('_', ".")
    }

    pub(crate) fn capability_name(self) -> &'static str {
        self.description()
    }

    pub(crate) fn risk_level(self) -> RiskLevel {
        match self {
            DockerToolKind::ContainerLogs => RiskLevel::Medium,
            DockerToolKind::ContainerCreate
            | DockerToolKind::ContainerStart
            | DockerToolKind::ContainerStop
            | DockerToolKind::ContainerRestart
            | DockerToolKind::ImagePull
            | DockerToolKind::ImageRemove
            | DockerToolKind::NetworkCreate
            | DockerToolKind::NetworkConnect
            | DockerToolKind::NetworkDisconnect
            | DockerToolKind::VolumeCreate => RiskLevel::High,
            DockerToolKind::ContainerRemove
            | DockerToolKind::ContainerPrune
            | DockerToolKind::ImagePrune
            | DockerToolKind::NetworkRemove
            | DockerToolKind::NetworkPrune
            | DockerToolKind::VolumeRemove
            | DockerToolKind::VolumePrune
            | DockerToolKind::SystemPrune => RiskLevel::Critical,
            _ => RiskLevel::Low,
        }
    }

    fn parameters_schema(self) -> Value {
        match self {
            DockerToolKind::Info
            | DockerToolKind::Version
            | DockerToolKind::NetworkList
            | DockerToolKind::NetworkPrune
            | DockerToolKind::VolumeList
            | DockerToolKind::VolumePrune
            | DockerToolKind::ContainerPrune
            | DockerToolKind::ImagePrune
            | DockerToolKind::SystemPrune => empty_schema(),
            DockerToolKind::ContainerList | DockerToolKind::ImageList => json!({
                "type": "object",
                "properties": {
                    "all": {"type": "boolean", "default": false}
                },
                "additionalProperties": false
            }),
            DockerToolKind::ContainerInspect => json!({
                "type": "object",
                "properties": {
                    "container": {"type": "string", "minLength": 1},
                    "size": {"type": "boolean", "default": false}
                },
                "required": ["container"],
                "additionalProperties": false
            }),
            DockerToolKind::ContainerLogs => json!({
                "type": "object",
                "properties": {
                    "container": {"type": "string", "minLength": 1},
                    "tail": {"type": "integer", "minimum": 1, "maximum": MAX_LOG_TAIL, "default": DEFAULT_LOG_TAIL},
                    "timestamps": {"type": "boolean", "default": false},
                    "stdout": {"type": "boolean", "default": true},
                    "stderr": {"type": "boolean", "default": true}
                },
                "required": ["container"],
                "additionalProperties": false
            }),
            DockerToolKind::ContainerCreate => json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "image": {"type": "string", "minLength": 1},
                    "cmd": {"type": "array", "items": {"type": "string"}},
                    "env": {"type": "object", "additionalProperties": {"type": "string"}},
                    "labels": {"type": "object", "additionalProperties": {"type": "string"}},
                    "ports": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "container_port": {"type": "integer", "minimum": 1, "maximum": 65535},
                                "host_port": {"type": "integer", "minimum": 1, "maximum": 65535},
                                "protocol": {"type": "string", "enum": ["tcp", "udp", "sctp"], "default": "tcp"},
                                "host_ip": {"type": "string"}
                            },
                            "required": ["container_port", "host_port"],
                            "additionalProperties": false
                        }
                    },
                    "binds": {"type": "array", "items": {"type": "string"}},
                    "volumes": {"type": "array", "items": {"type": "string"}},
                    "network": {"type": "string", "minLength": 1},
                    "memory_bytes": {"type": "integer", "minimum": 1},
                    "publish_all_ports": {"type": "boolean", "default": false},
                    "restart_policy": {"type": "string", "enum": ["no", "always", "unless-stopped", "on-failure"]},
                    "auto_remove": {"type": "boolean"}
                },
                "required": ["image"],
                "additionalProperties": false
            }),
            DockerToolKind::ContainerStart => id_schema("container"),
            DockerToolKind::ContainerStop | DockerToolKind::ContainerRestart => json!({
                "type": "object",
                "properties": {
                    "container": {"type": "string", "minLength": 1},
                    "timeout_seconds": {"type": "integer", "minimum": 0}
                },
                "required": ["container"],
                "additionalProperties": false
            }),
            DockerToolKind::ContainerRemove => json!({
                "type": "object",
                "properties": {
                    "container": {"type": "string", "minLength": 1},
                    "force": {"type": "boolean", "default": false},
                    "volumes": {"type": "boolean", "default": false}
                },
                "required": ["container"],
                "additionalProperties": false
            }),
            DockerToolKind::ImageInspect => id_schema("image"),
            DockerToolKind::ImagePull => json!({
                "type": "object",
                "properties": {
                    "image": {"type": "string", "minLength": 1},
                    "tag": {"type": "string", "minLength": 1}
                },
                "required": ["image"],
                "additionalProperties": false
            }),
            DockerToolKind::ImageRemove => json!({
                "type": "object",
                "properties": {
                    "image": {"type": "string", "minLength": 1},
                    "force": {"type": "boolean", "default": false},
                    "no_prune": {"type": "boolean", "default": false}
                },
                "required": ["image"],
                "additionalProperties": false
            }),
            DockerToolKind::NetworkInspect | DockerToolKind::NetworkRemove => id_schema("network"),
            DockerToolKind::NetworkCreate => json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "driver": {"type": "string", "minLength": 1},
                    "internal": {"type": "boolean"},
                    "attachable": {"type": "boolean"},
                    "enable_ipv6": {"type": "boolean"},
                    "options": {"type": "object", "additionalProperties": {"type": "string"}},
                    "labels": {"type": "object", "additionalProperties": {"type": "string"}}
                },
                "required": ["name"],
                "additionalProperties": false
            }),
            DockerToolKind::NetworkConnect => network_container_schema(false),
            DockerToolKind::NetworkDisconnect => network_container_schema(true),
            DockerToolKind::VolumeInspect | DockerToolKind::VolumeRemove => id_schema("volume"),
            DockerToolKind::VolumeCreate => json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "driver": {"type": "string", "minLength": 1},
                    "driver_options": {"type": "object", "additionalProperties": {"type": "string"}},
                    "labels": {"type": "object", "additionalProperties": {"type": "string"}}
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        }
    }

    fn operation(self, arguments: Value) -> Result<DockerOperation, DockerToolError> {
        match self {
            DockerToolKind::Info => {
                let _: EmptyArgs = parse_args(arguments)?;
                Ok(DockerOperation::Info)
            }
            DockerToolKind::Version => {
                let _: EmptyArgs = parse_args(arguments)?;
                Ok(DockerOperation::Version)
            }
            DockerToolKind::ContainerList => {
                let args: AllArgs = parse_args(arguments)?;
                Ok(DockerOperation::ListContainers { all: args.all })
            }
            DockerToolKind::ContainerInspect => {
                let args: ContainerInspectArgs = parse_args(arguments)?;
                Ok(DockerOperation::InspectContainer {
                    container: non_empty(args.container, "container")?,
                    size: args.size,
                })
            }
            DockerToolKind::ContainerLogs => {
                let args: ContainerLogsArgs = parse_args(arguments)?;
                Ok(DockerOperation::ContainerLogs {
                    container: non_empty(args.container, "container")?,
                    tail: args.tail.unwrap_or(DEFAULT_LOG_TAIL).clamp(1, MAX_LOG_TAIL),
                    timestamps: args.timestamps,
                    stdout: args.stdout,
                    stderr: args.stderr,
                })
            }
            DockerToolKind::ContainerCreate => {
                let mut request: ContainerCreateRequest = parse_args(arguments)?;
                request.image = non_empty(request.image, "image")?;
                request.name = optional_non_empty(request.name, "name")?;
                request.network = optional_non_empty(request.network, "network")?;
                Ok(DockerOperation::CreateContainer(request))
            }
            DockerToolKind::ContainerStart => {
                let args: ContainerArg = parse_args(arguments)?;
                Ok(DockerOperation::StartContainer {
                    container: non_empty(args.container, "container")?,
                })
            }
            DockerToolKind::ContainerStop => {
                let args: TimedContainerArg = parse_args(arguments)?;
                Ok(DockerOperation::StopContainer {
                    container: non_empty(args.container, "container")?,
                    timeout_seconds: args.timeout_seconds,
                })
            }
            DockerToolKind::ContainerRestart => {
                let args: TimedContainerArg = parse_args(arguments)?;
                Ok(DockerOperation::RestartContainer {
                    container: non_empty(args.container, "container")?,
                    timeout_seconds: args.timeout_seconds,
                })
            }
            DockerToolKind::ContainerRemove => {
                let args: RemoveContainerArgs = parse_args(arguments)?;
                Ok(DockerOperation::RemoveContainer {
                    container: non_empty(args.container, "container")?,
                    force: args.force,
                    volumes: args.volumes,
                })
            }
            DockerToolKind::ContainerPrune => {
                let _: EmptyArgs = parse_args(arguments)?;
                Ok(DockerOperation::PruneContainers)
            }
            DockerToolKind::ImageList => {
                let args: AllArgs = parse_args(arguments)?;
                Ok(DockerOperation::ListImages { all: args.all })
            }
            DockerToolKind::ImageInspect => {
                let args: ImageArg = parse_args(arguments)?;
                Ok(DockerOperation::InspectImage {
                    image: non_empty(args.image, "image")?,
                })
            }
            DockerToolKind::ImagePull => {
                let args: PullImageArgs = parse_args(arguments)?;
                Ok(DockerOperation::PullImage {
                    image: non_empty(args.image, "image")?,
                    tag: optional_non_empty(args.tag, "tag")?,
                })
            }
            DockerToolKind::ImageRemove => {
                let args: RemoveImageArgs = parse_args(arguments)?;
                Ok(DockerOperation::RemoveImage {
                    image: non_empty(args.image, "image")?,
                    force: args.force,
                    no_prune: args.no_prune,
                })
            }
            DockerToolKind::ImagePrune => {
                let _: EmptyArgs = parse_args(arguments)?;
                Ok(DockerOperation::PruneImages)
            }
            DockerToolKind::NetworkList => {
                let _: EmptyArgs = parse_args(arguments)?;
                Ok(DockerOperation::ListNetworks)
            }
            DockerToolKind::NetworkInspect => {
                let args: NetworkArg = parse_args(arguments)?;
                Ok(DockerOperation::InspectNetwork {
                    network: non_empty(args.network, "network")?,
                })
            }
            DockerToolKind::NetworkCreate => {
                let mut request: NetworkCreateToolRequest = parse_args(arguments)?;
                request.name = non_empty(request.name, "name")?;
                request.driver = optional_non_empty(request.driver, "driver")?;
                Ok(DockerOperation::CreateNetwork(request))
            }
            DockerToolKind::NetworkConnect => {
                let args: NetworkContainerArgs = parse_args(arguments)?;
                Ok(DockerOperation::ConnectNetwork {
                    network: non_empty(args.network, "network")?,
                    container: non_empty(args.container, "container")?,
                })
            }
            DockerToolKind::NetworkDisconnect => {
                let args: NetworkContainerArgs = parse_args(arguments)?;
                Ok(DockerOperation::DisconnectNetwork {
                    network: non_empty(args.network, "network")?,
                    container: non_empty(args.container, "container")?,
                    force: args.force,
                })
            }
            DockerToolKind::NetworkRemove => {
                let args: NetworkArg = parse_args(arguments)?;
                Ok(DockerOperation::RemoveNetwork {
                    network: non_empty(args.network, "network")?,
                })
            }
            DockerToolKind::NetworkPrune => {
                let _: EmptyArgs = parse_args(arguments)?;
                Ok(DockerOperation::PruneNetworks)
            }
            DockerToolKind::VolumeList => {
                let _: EmptyArgs = parse_args(arguments)?;
                Ok(DockerOperation::ListVolumes)
            }
            DockerToolKind::VolumeInspect => {
                let args: VolumeArg = parse_args(arguments)?;
                Ok(DockerOperation::InspectVolume {
                    volume: non_empty(args.volume, "volume")?,
                })
            }
            DockerToolKind::VolumeCreate => {
                let mut request: VolumeCreateToolRequest = parse_args(arguments)?;
                request.name = non_empty(request.name, "name")?;
                request.driver = optional_non_empty(request.driver, "driver")?;
                Ok(DockerOperation::CreateVolume(request))
            }
            DockerToolKind::VolumeRemove => {
                let args: RemoveVolumeArgs = parse_args(arguments)?;
                Ok(DockerOperation::RemoveVolume {
                    volume: non_empty(args.volume, "volume")?,
                    force: args.force,
                })
            }
            DockerToolKind::VolumePrune => {
                let _: EmptyArgs = parse_args(arguments)?;
                Ok(DockerOperation::PruneVolumes)
            }
            DockerToolKind::SystemPrune => {
                let _: EmptyArgs = parse_args(arguments)?;
                Ok(DockerOperation::SystemPrune)
            }
        }
    }
}

pub(crate) struct DockerTool {
    kind: DockerToolKind,
    resolver: Arc<dyn DockerRuntimeResolver>,
}

impl DockerTool {
    pub(crate) fn new(kind: DockerToolKind, resolver: Arc<dyn DockerRuntimeResolver>) -> Self {
        Self { kind, resolver }
    }
}

#[async_trait]
impl Tool for DockerTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            self.kind.name(),
            self.kind.description(),
            self.kind.parameters_schema(),
            self.kind.risk_level(),
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        self.call_with_context(arguments, ToolCallContext::default())
            .await
    }

    async fn call_with_context(&self, arguments: Value, context: ToolCallContext) -> ToolResult {
        let operation = match self.kind.operation(arguments) {
            Ok(operation) => operation,
            Err(err) => return ToolResult::error(err.to_string()),
        };
        let client = match self.resolver.resolve(&context).await {
            Ok(client) => client,
            Err(err) => return ToolResult::error(err.to_string()),
        };

        match client.execute(operation).await {
            Ok(output) => ToolResult::success(tool_output(output)),
            Err(err) => ToolResult::error(err.to_string()),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AllArgs {
    #[serde(default)]
    all: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContainerArg {
    container: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContainerInspectArgs {
    container: String,
    #[serde(default)]
    size: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContainerLogsArgs {
    container: String,
    tail: Option<u32>,
    #[serde(default)]
    timestamps: bool,
    #[serde(default = "default_true")]
    stdout: bool,
    #[serde(default = "default_true")]
    stderr: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TimedContainerArg {
    container: String,
    #[serde(default)]
    timeout_seconds: Option<i32>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveContainerArgs {
    container: String,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    volumes: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageArg {
    image: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PullImageArgs {
    image: String,
    tag: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveImageArgs {
    image: String,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    no_prune: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NetworkArg {
    network: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct NetworkContainerArgs {
    network: String,
    container: String,
    #[serde(default)]
    force: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VolumeArg {
    volume: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveVolumeArgs {
    volume: String,
    #[serde(default)]
    force: bool,
}

fn parse_args<T: DeserializeOwned>(arguments: Value) -> Result<T, DockerToolError> {
    serde_json::from_value(arguments)
        .map_err(|err| DockerToolError::InvalidArguments(err.to_string()))
}

fn non_empty(value: String, field: &'static str) -> Result<String, DockerToolError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(DockerToolError::InvalidArguments(format!(
            "{field} is required"
        )));
    }

    Ok(value)
}

fn optional_non_empty(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<String>, DockerToolError> {
    value.map(|value| non_empty(value, field)).transpose()
}

fn tool_output(value: Value) -> String {
    match value {
        Value::String(output) => output,
        other => serde_json::to_string_pretty(&other).unwrap_or_else(|_| other.to_string()),
    }
}

fn empty_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

fn id_schema(field: &'static str) -> Value {
    json!({
        "type": "object",
        "properties": {
            field: {"type": "string", "minLength": 1}
        },
        "required": [field],
        "additionalProperties": false
    })
}

fn network_container_schema(include_force: bool) -> Value {
    let mut schema = json!({
        "type": "object",
        "properties": {
            "network": {"type": "string", "minLength": 1},
            "container": {"type": "string", "minLength": 1}
        },
        "required": ["network", "container"],
        "additionalProperties": false
    });
    if include_force {
        schema["properties"]["force"] = json!({"type": "boolean", "default": false});
    }
    schema
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, sync::Mutex};

    use crate::{
        client::{DockerClient, DockerRuntimeResolver},
        operation::{ContainerPortBinding, DockerOperation},
        plugin::DockerPlugin,
    };
    use midgard_controller::{MiddlewareController, MiddlewarePlugin};
    use midgard_tools::ToolRegistry;

    #[tokio::test]
    async fn docker_tool_calls_mock_client_with_parsed_operation() {
        let client = Arc::new(MockDockerClient::default());
        let plugin = DockerPlugin::new(Arc::new(MockResolver {
            client: Some(client.clone()),
            error: None,
        }));
        let controller = plugin.controller();
        let mut registry = ToolRegistry::default();
        controller.register_tools(&mut registry);

        let result = registry
            .call_with_context(
                "docker_container_create",
                json!({
                    "name": "web",
                    "image": "nginx:latest",
                    "cmd": ["nginx", "-g", "daemon off;"],
                    "env": {"RUST_LOG": "info"},
                    "ports": [{"container_port": 80, "host_port": 8080}]
                }),
                ToolCallContext {
                    workspace_id: Some("workspace-1".to_string()),
                },
            )
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(
            client.operations.lock().unwrap()[0],
            DockerOperation::CreateContainer(ContainerCreateRequest {
                name: Some("web".to_string()),
                image: "nginx:latest".to_string(),
                cmd: vec![
                    "nginx".to_string(),
                    "-g".to_string(),
                    "daemon off;".to_string()
                ],
                env: HashMap::from([("RUST_LOG".to_string(), "info".to_string())]),
                ports: vec![ContainerPortBinding {
                    container_port: 80,
                    host_port: 8080,
                    protocol: "tcp".to_string(),
                    host_ip: None,
                }],
                ..Default::default()
            })
        );
    }

    #[tokio::test]
    async fn docker_tool_rejects_missing_required_arguments() {
        let plugin = DockerPlugin::new(Arc::new(MockResolver::default()));
        let controller = plugin.controller();
        let mut registry = ToolRegistry::default();
        controller.register_tools(&mut registry);

        let result = registry
            .call("docker_container_inspect", json!({}))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.output.contains("invalid docker tool arguments"));
    }

    #[tokio::test]
    async fn docker_tool_rejects_workspace_id_argument() {
        let plugin = DockerPlugin::new(Arc::new(MockResolver::default()));
        let controller = plugin.controller();
        let mut registry = ToolRegistry::default();
        controller.register_tools(&mut registry);

        let result = registry
            .call(
                "docker_info",
                json!({"workspace_id": "00000000-0000-0000-0000-000000000000"}),
            )
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.output.contains("unknown field"));
    }

    #[tokio::test]
    async fn docker_tool_reports_resolver_errors_without_calling_client() {
        let plugin = DockerPlugin::new(Arc::new(MockResolver {
            client: None,
            error: Some("workspace runtime is not docker".to_string()),
        }));
        let controller = plugin.controller();
        let mut registry = ToolRegistry::default();
        controller.register_tools(&mut registry);

        let result = registry.call("docker_info", json!({})).await.unwrap();

        assert!(!result.success);
        assert_eq!(
            result.output,
            "docker runtime is not available: workspace runtime is not docker"
        );
    }

    #[derive(Default)]
    struct MockResolver {
        client: Option<Arc<MockDockerClient>>,
        error: Option<String>,
    }

    #[async_trait]
    impl DockerRuntimeResolver for MockResolver {
        async fn resolve(
            &self,
            _context: &ToolCallContext,
        ) -> Result<Arc<dyn DockerClient>, DockerToolError> {
            if let Some(error) = &self.error {
                return Err(DockerToolError::Runtime(error.clone()));
            }

            Ok(self
                .client
                .clone()
                .unwrap_or_else(|| Arc::new(MockDockerClient::default())))
        }
    }

    #[derive(Default)]
    struct MockDockerClient {
        operations: Mutex<Vec<DockerOperation>>,
    }

    #[async_trait]
    impl DockerClient for MockDockerClient {
        async fn execute(&self, operation: DockerOperation) -> Result<Value, DockerToolError> {
            self.operations.lock().unwrap().push(operation);
            Ok(json!({"ok": true}))
        }
    }
}
