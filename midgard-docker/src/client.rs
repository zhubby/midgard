use std::{fmt, sync::Arc};

use async_trait::async_trait;
use bollard::{
    Docker,
    models::{
        ContainerCreateBody, EndpointSettings, HostConfig, NetworkConnectRequest,
        NetworkCreateRequest, NetworkDisconnectRequest, PortBinding, PortMap, RestartPolicy,
        RestartPolicyNameEnum, VolumeCreateRequest,
    },
    query_parameters::{
        CreateContainerOptionsBuilder, CreateImageOptionsBuilder, InspectContainerOptionsBuilder,
        ListContainersOptionsBuilder, ListImagesOptionsBuilder, ListVolumesOptions,
        LogsOptionsBuilder, PruneContainersOptions, PruneImagesOptions, PruneNetworksOptions,
        PruneVolumesOptions, RemoveContainerOptionsBuilder, RemoveImageOptionsBuilder,
        RemoveVolumeOptionsBuilder, RestartContainerOptionsBuilder, StopContainerOptionsBuilder,
    },
};
use futures_util::{StreamExt, TryStreamExt};
use midgard_tools::ToolCallContext;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    DockerToolError,
    operation::{ContainerCreateRequest, ContainerPortBinding, DockerOperation, MAX_LOG_TAIL},
};

#[async_trait]
pub trait DockerRuntimeResolver: Send + Sync {
    async fn resolve(
        &self,
        context: &ToolCallContext,
    ) -> Result<Arc<dyn DockerClient>, DockerToolError>;
}

#[async_trait]
pub trait DockerClient: Send + Sync {
    async fn execute(&self, operation: DockerOperation) -> Result<Value, DockerToolError>;
}

#[derive(Clone)]
pub struct BollardDockerClient {
    docker: Docker,
}

impl BollardDockerClient {
    pub fn connect(endpoint: &str) -> Result<Self, DockerToolError> {
        let docker = Docker::connect_with_host(endpoint).map_err(|_| {
            DockerToolError::Runtime(
                "failed to create Docker client for configured endpoint".to_string(),
            )
        })?;

        Ok(Self { docker })
    }
}

#[async_trait]
impl DockerClient for BollardDockerClient {
    async fn execute(&self, operation: DockerOperation) -> Result<Value, DockerToolError> {
        self.execute_operation(operation).await
    }
}

impl BollardDockerClient {
    async fn execute_operation(
        &self,
        operation: DockerOperation,
    ) -> Result<Value, DockerToolError> {
        match operation {
            DockerOperation::Info => self.to_value("read Docker info", self.docker.info().await),
            DockerOperation::Version => {
                self.to_value("read Docker version", self.docker.version().await)
            }
            DockerOperation::ListContainers { all } => {
                let options = ListContainersOptionsBuilder::new().all(all).build();
                self.to_value(
                    "list Docker containers",
                    self.docker.list_containers(Some(options)).await,
                )
            }
            DockerOperation::InspectContainer { container, size } => {
                let options = InspectContainerOptionsBuilder::new().size(size).build();
                self.to_value(
                    "inspect Docker container",
                    self.docker
                        .inspect_container(&container, Some(options))
                        .await,
                )
            }
            DockerOperation::ContainerLogs {
                container,
                tail,
                timestamps,
                stdout,
                stderr,
            } => {
                let tail = tail.clamp(1, MAX_LOG_TAIL).to_string();
                let options = LogsOptionsBuilder::new()
                    .stdout(stdout)
                    .stderr(stderr)
                    .timestamps(timestamps)
                    .tail(&tail)
                    .build();
                let mut stream = self.docker.logs(&container, Some(options));
                let mut output = String::new();
                while let Some(chunk) = stream.next().await {
                    let chunk =
                        chunk.map_err(|err| api_error("read Docker container logs", err))?;
                    output.push_str(&chunk.to_string());
                }
                Ok(Value::String(output))
            }
            DockerOperation::CreateContainer(request) => {
                let options = request
                    .name
                    .as_ref()
                    .map(|name| CreateContainerOptionsBuilder::new().name(name).build());
                let body = container_create_body(request)?;
                self.to_value(
                    "create Docker container",
                    self.docker.create_container(options, body).await,
                )
            }
            DockerOperation::StartContainer { container } => self.to_value(
                "start Docker container",
                self.docker.start_container(&container, None).await,
            ),
            DockerOperation::StopContainer {
                container,
                timeout_seconds,
            } => {
                let options = timeout_seconds
                    .map(|timeout| StopContainerOptionsBuilder::new().t(timeout).build());
                self.to_value(
                    "stop Docker container",
                    self.docker.stop_container(&container, options).await,
                )
            }
            DockerOperation::RestartContainer {
                container,
                timeout_seconds,
            } => {
                let options = timeout_seconds
                    .map(|timeout| RestartContainerOptionsBuilder::new().t(timeout).build());
                self.to_value(
                    "restart Docker container",
                    self.docker.restart_container(&container, options).await,
                )
            }
            DockerOperation::RemoveContainer {
                container,
                force,
                volumes,
            } => {
                let options = RemoveContainerOptionsBuilder::new()
                    .force(force)
                    .v(volumes)
                    .build();
                self.to_value(
                    "remove Docker container",
                    self.docker
                        .remove_container(&container, Some(options))
                        .await,
                )
            }
            DockerOperation::PruneContainers => self.to_value(
                "prune Docker containers",
                self.docker
                    .prune_containers(None::<PruneContainersOptions>)
                    .await,
            ),
            DockerOperation::ListImages { all } => {
                let options = ListImagesOptionsBuilder::new().all(all).build();
                self.to_value(
                    "list Docker images",
                    self.docker.list_images(Some(options)).await,
                )
            }
            DockerOperation::InspectImage { image } => self.to_value(
                "inspect Docker image",
                self.docker.inspect_image(&image).await,
            ),
            DockerOperation::PullImage { image, tag } => {
                let mut builder = CreateImageOptionsBuilder::new().from_image(&image);
                if let Some(tag) = tag.as_deref() {
                    builder = builder.tag(tag);
                }
                let events = self
                    .docker
                    .create_image(Some(builder.build()), None, None)
                    .try_collect::<Vec<_>>()
                    .await
                    .map_err(|err| api_error("pull Docker image", err))?;
                json_value(events)
            }
            DockerOperation::RemoveImage {
                image,
                force,
                no_prune,
            } => {
                let options = RemoveImageOptionsBuilder::new()
                    .force(force)
                    .noprune(no_prune)
                    .build();
                self.to_value(
                    "remove Docker image",
                    self.docker.remove_image(&image, Some(options), None).await,
                )
            }
            DockerOperation::PruneImages => self.to_value(
                "prune Docker images",
                self.docker.prune_images(None::<PruneImagesOptions>).await,
            ),
            DockerOperation::ListNetworks => self.to_value(
                "list Docker networks",
                self.docker.list_networks(None).await,
            ),
            DockerOperation::InspectNetwork { network } => self.to_value(
                "inspect Docker network",
                self.docker.inspect_network(&network, None).await,
            ),
            DockerOperation::CreateNetwork(request) => {
                let body = NetworkCreateRequest {
                    name: request.name,
                    driver: request.driver,
                    scope: None,
                    internal: request.internal,
                    attachable: request.attachable,
                    ingress: None,
                    config_only: None,
                    config_from: None,
                    ipam: None,
                    enable_ipv4: None,
                    enable_ipv6: request.enable_ipv6,
                    options: request.options,
                    labels: request.labels,
                };
                self.to_value(
                    "create Docker network",
                    self.docker.create_network(body).await,
                )
            }
            DockerOperation::ConnectNetwork { network, container } => self.to_value(
                "connect Docker network",
                self.docker
                    .connect_network(
                        &network,
                        NetworkConnectRequest {
                            container,
                            endpoint_config: Some(EndpointSettings::default()),
                        },
                    )
                    .await,
            ),
            DockerOperation::DisconnectNetwork {
                network,
                container,
                force,
            } => self.to_value(
                "disconnect Docker network",
                self.docker
                    .disconnect_network(
                        &network,
                        NetworkDisconnectRequest {
                            container,
                            force: Some(force),
                        },
                    )
                    .await,
            ),
            DockerOperation::RemoveNetwork { network } => self.to_value(
                "remove Docker network",
                self.docker.remove_network(&network).await,
            ),
            DockerOperation::PruneNetworks => self.to_value(
                "prune Docker networks",
                self.docker
                    .prune_networks(None::<PruneNetworksOptions>)
                    .await,
            ),
            DockerOperation::ListVolumes => self.to_value(
                "list Docker volumes",
                self.docker.list_volumes(None::<ListVolumesOptions>).await,
            ),
            DockerOperation::InspectVolume { volume } => self.to_value(
                "inspect Docker volume",
                self.docker.inspect_volume(&volume).await,
            ),
            DockerOperation::CreateVolume(request) => {
                let body = VolumeCreateRequest {
                    name: Some(request.name),
                    driver: request.driver,
                    driver_opts: request.driver_options,
                    labels: request.labels,
                    cluster_volume_spec: None,
                };
                self.to_value(
                    "create Docker volume",
                    self.docker.create_volume(body).await,
                )
            }
            DockerOperation::RemoveVolume { volume, force } => {
                let options = RemoveVolumeOptionsBuilder::new().force(force).build();
                self.to_value(
                    "remove Docker volume",
                    self.docker.remove_volume(&volume, Some(options)).await,
                )
            }
            DockerOperation::PruneVolumes => self.to_value(
                "prune Docker volumes",
                self.docker.prune_volumes(None::<PruneVolumesOptions>).await,
            ),
            DockerOperation::SystemPrune => {
                let containers = self
                    .docker
                    .prune_containers(None::<PruneContainersOptions>)
                    .await
                    .map_err(|err| api_error("prune Docker containers", err))?;
                let images = self
                    .docker
                    .prune_images(None::<PruneImagesOptions>)
                    .await
                    .map_err(|err| api_error("prune Docker images", err))?;
                let networks = self
                    .docker
                    .prune_networks(None::<PruneNetworksOptions>)
                    .await
                    .map_err(|err| api_error("prune Docker networks", err))?;
                let volumes = self
                    .docker
                    .prune_volumes(None::<PruneVolumesOptions>)
                    .await
                    .map_err(|err| api_error("prune Docker volumes", err))?;
                Ok(json!({
                    "containers": containers,
                    "images": images,
                    "networks": networks,
                    "volumes": volumes,
                }))
            }
        }
    }

    fn to_value<T, E>(
        &self,
        action: &'static str,
        result: Result<T, E>,
    ) -> Result<Value, DockerToolError>
    where
        T: Serialize,
        E: fmt::Display,
    {
        result
            .map_err(|err| api_error(action, err))
            .and_then(json_value)
    }
}

fn container_create_body(
    request: ContainerCreateRequest,
) -> Result<ContainerCreateBody, DockerToolError> {
    let env = (!request.env.is_empty()).then(|| {
        request
            .env
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
    });
    let labels = (!request.labels.is_empty()).then_some(request.labels);
    let exposed_ports = (!request.ports.is_empty()).then(|| {
        request
            .ports
            .iter()
            .map(container_port_key)
            .collect::<Vec<_>>()
    });
    let port_bindings = (!request.ports.is_empty()).then(|| {
        request
            .ports
            .iter()
            .map(|port| {
                (
                    container_port_key(port),
                    Some(vec![PortBinding {
                        host_ip: port.host_ip.clone(),
                        host_port: Some(port.host_port.to_string()),
                    }]),
                )
            })
            .collect::<PortMap>()
    });
    let restart_policy = restart_policy(request.restart_policy)?;
    let host_config = HostConfig {
        binds: (!request.binds.is_empty()).then_some(request.binds),
        memory: request.memory_bytes,
        network_mode: request.network,
        port_bindings,
        publish_all_ports: Some(request.publish_all_ports),
        restart_policy,
        auto_remove: request.auto_remove,
        ..Default::default()
    };

    Ok(ContainerCreateBody {
        image: Some(request.image),
        cmd: (!request.cmd.is_empty()).then_some(request.cmd),
        env,
        labels,
        exposed_ports,
        volumes: (!request.volumes.is_empty()).then_some(request.volumes),
        host_config: Some(host_config),
        ..Default::default()
    })
}

fn container_port_key(port: &ContainerPortBinding) -> String {
    format!("{}/{}", port.container_port, port.protocol)
}

fn restart_policy(value: Option<String>) -> Result<Option<RestartPolicy>, DockerToolError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let name = match value.as_str() {
        "no" => RestartPolicyNameEnum::NO,
        "always" => RestartPolicyNameEnum::ALWAYS,
        "unless-stopped" => RestartPolicyNameEnum::UNLESS_STOPPED,
        "on-failure" => RestartPolicyNameEnum::ON_FAILURE,
        other => {
            return Err(DockerToolError::InvalidArguments(format!(
                "unsupported restart_policy: {other}"
            )));
        }
    };

    Ok(Some(RestartPolicy {
        name: Some(name),
        maximum_retry_count: None,
    }))
}

fn json_value<T: Serialize>(value: T) -> Result<Value, DockerToolError> {
    serde_json::to_value(value)
        .map_err(|err| DockerToolError::Runtime(format!("serialize Docker response: {err}")))
}

fn api_error<E: fmt::Display>(action: &'static str, err: E) -> DockerToolError {
    DockerToolError::Api(action, err.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::operation::ContainerPortBinding;

    #[test]
    fn container_create_body_maps_structured_request() {
        let body = container_create_body(ContainerCreateRequest {
            name: Some("web".to_string()),
            image: "nginx:latest".to_string(),
            cmd: vec![
                "nginx".to_string(),
                "-g".to_string(),
                "daemon off;".to_string(),
            ],
            env: HashMap::from([("RUST_LOG".to_string(), "info".to_string())]),
            labels: HashMap::from([("app".to_string(), "web".to_string())]),
            ports: vec![ContainerPortBinding {
                container_port: 80,
                host_port: 8080,
                protocol: "tcp".to_string(),
                host_ip: Some("127.0.0.1".to_string()),
            }],
            binds: vec!["/host:/container:ro".to_string()],
            volumes: vec!["/data".to_string()],
            network: Some("frontend".to_string()),
            memory_bytes: Some(268_435_456),
            publish_all_ports: true,
            restart_policy: Some("always".to_string()),
            auto_remove: Some(false),
        })
        .unwrap();

        assert_eq!(body.image.as_deref(), Some("nginx:latest"));
        assert_eq!(body.cmd.as_ref().unwrap().len(), 3);
        assert_eq!(
            body.env.as_ref().unwrap(),
            &vec!["RUST_LOG=info".to_string()]
        );
        assert_eq!(
            body.labels.as_ref().unwrap().get("app").map(String::as_str),
            Some("web")
        );
        assert!(
            body.exposed_ports
                .as_ref()
                .unwrap()
                .iter()
                .any(|port| port == "80/tcp")
        );
        assert!(
            body.volumes
                .as_ref()
                .unwrap()
                .iter()
                .any(|volume| volume == "/data")
        );

        let host_config = body.host_config.unwrap();
        assert_eq!(host_config.network_mode.as_deref(), Some("frontend"));
        assert_eq!(host_config.memory, Some(268_435_456));
        assert_eq!(host_config.publish_all_ports, Some(true));
        assert_eq!(host_config.auto_remove, Some(false));
        assert_eq!(
            host_config.restart_policy.unwrap().name,
            Some(RestartPolicyNameEnum::ALWAYS)
        );
        let port_bindings = host_config.port_bindings.unwrap();
        let binding = &port_bindings["80/tcp"].as_ref().unwrap()[0];
        assert_eq!(binding.host_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(binding.host_port.as_deref(), Some("8080"));
    }

    #[test]
    fn container_create_body_rejects_unsupported_restart_policy() {
        let err = container_create_body(ContainerCreateRequest {
            image: "nginx:latest".to_string(),
            restart_policy: Some("sometimes".to_string()),
            ..Default::default()
        })
        .unwrap_err();

        assert!(err.to_string().contains("unsupported restart_policy"));
    }
}
