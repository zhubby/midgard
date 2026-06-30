use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{
    Deployment, DeploymentSpec, DeploymentStrategy, StatefulSet, StatefulSetSpec,
};
use k8s_openapi::api::core::v1::{
    ConfigMap, ConfigMapVolumeSource, Container, ContainerPort, EnvVar, EnvVarSource, ExecAction,
    HTTPGetAction, LocalObjectReference, ObjectFieldSelector, PersistentVolumeClaim,
    PersistentVolumeClaimSpec, PersistentVolumeClaimVolumeSource, Pod, PodSpec, PodTemplateSpec,
    Probe, Secret, SecretKeySelector, SecretVolumeSource, Service, ServicePort, ServiceSpec,
    Volume, VolumeMount, VolumeResourceRequirements,
};
use k8s_openapi::api::policy::v1::{PodDisruptionBudget, PodDisruptionBudgetSpec};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{Api, ResourceExt};
use serde_json::Value;

use crate::api::{ExporterSpec, PdbPolicy, TlsConfig, ValkeyCluster, ValkeyNode, WorkloadType};
use crate::controller::config::build_node_config_map;
use crate::controller::users::{
    EXPORTER_USER, internal_acl_secret_name, system_password_secret_name,
};
use crate::controller::{
    CONFIG_HASH_KEY, DATA_MOUNT_PATH, DATA_VOLUME_NAME, DEFAULT_CLUSTER_BUS_PORT,
    DEFAULT_EXPORTER_IMAGE, DEFAULT_EXPORTER_PORT, DEFAULT_IMAGE, DEFAULT_PORT, LABEL_CLUSTER,
    LABEL_NODE_INDEX, LABEL_SHARD_INDEX, TLS_CERT_MOUNT_PATH, TLS_SECRET_KEY_CA,
    TLS_SECRET_KEY_CERT, TLS_SECRET_KEY_KEY, TLS_VOLUME_NAME, apply, cluster_labels,
    headless_service_name, object_meta, owner_reference, server_config_map_name,
    valkey_node_labels, valkey_node_pvc_name, valkey_node_resource_name,
};
use crate::error::Result;

pub async fn upsert_service(client: kube::Client, cluster: &ValkeyCluster) -> Result<()> {
    let namespace = cluster.namespace().unwrap_or_default();
    let name = headless_service_name(&cluster.name_any());
    let service = Service {
        metadata: object_meta(
            name.clone(),
            namespace.clone(),
            cluster_labels(cluster),
            BTreeMap::new(),
            owner_reference(cluster),
        ),
        spec: Some(ServiceSpec {
            type_: Some("ClusterIP".to_string()),
            cluster_ip: Some("None".to_string()),
            selector: Some(BTreeMap::from([(
                LABEL_CLUSTER.to_string(),
                cluster.name_any(),
            )])),
            ports: Some(vec![ServicePort {
                name: Some("valkey".to_string()),
                port: DEFAULT_PORT,
                ..ServicePort::default()
            }]),
            ..ServiceSpec::default()
        }),
        ..Service::default()
    };
    let api = Api::<Service>::namespaced(client, &namespace);
    apply(&api, &name, &service).await?;
    Ok(())
}

pub async fn reconcile_pdb(client: kube::Client, cluster: &ValkeyCluster) -> Result<()> {
    let namespace = cluster.namespace().unwrap_or_default();
    let name = headless_service_name(&cluster.name_any());
    let api = Api::<PodDisruptionBudget>::namespaced(client, &namespace);
    if cluster.spec.pod_disruption_budget == PdbPolicy::Disabled {
        if let Some(pdb) = api.get_opt(&name).await? {
            api.delete(&pdb.name_any(), &Default::default()).await?;
        }
        return Ok(());
    }
    let pdb = PodDisruptionBudget {
        metadata: object_meta(
            name.clone(),
            namespace.clone(),
            cluster_labels(cluster),
            BTreeMap::new(),
            owner_reference(cluster),
        ),
        spec: Some(PodDisruptionBudgetSpec {
            max_unavailable: Some(IntOrString::Int(1)),
            selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    LABEL_CLUSTER.to_string(),
                    cluster.name_any(),
                )])),
                ..LabelSelector::default()
            }),
            ..PodDisruptionBudgetSpec::default()
        }),
        ..PodDisruptionBudget::default()
    };
    apply(&api, &name, &pdb).await?;
    Ok(())
}

pub fn build_cluster_valkey_node(
    cluster: &ValkeyCluster,
    shard_index: i32,
    node_index: i32,
) -> ValkeyNode {
    let cluster_name = cluster.name_any();
    let namespace = cluster.namespace().unwrap_or_default();
    let mut labels = cluster.metadata.labels.clone().unwrap_or_default();
    labels.extend(crate::controller::base_labels(&cluster_name, "valkey-node"));
    labels.insert(LABEL_CLUSTER.to_string(), cluster_name.clone());
    labels.insert(LABEL_SHARD_INDEX.to_string(), shard_index.to_string());
    labels.insert(LABEL_NODE_INDEX.to_string(), node_index.to_string());

    ValkeyNode {
        metadata: ObjectMeta {
            name: Some(crate::controller::valkey_node_name(
                &cluster_name,
                shard_index,
                node_index,
            )),
            namespace: Some(namespace),
            labels: Some(labels),
            owner_references: owner_reference(cluster).map(|owner| vec![owner]),
            ..ObjectMeta::default()
        },
        spec: crate::api::ValkeyNodeSpec {
            image: cluster.spec.image.clone(),
            image_pull_secrets: cluster.spec.image_pull_secrets.clone(),
            workload_type: cluster.spec.workload_type.clone(),
            persistence: cluster.spec.persistence.clone(),
            resources: cluster.spec.resources.clone(),
            node_selector: cluster.spec.node_selector.clone(),
            affinity: cluster.spec.affinity.clone(),
            tolerations: cluster.spec.tolerations.clone(),
            topology_spread_constraints: cluster.spec.topology_spread_constraints.clone(),
            exporter: cluster.spec.exporter.clone(),
            containers: cluster.spec.containers.clone(),
            server_config_map_name: server_config_map_name(&cluster_name),
            users_acl_secret_name: internal_acl_secret_name(&cluster_name),
            tls: cluster.spec.tls.clone(),
            config: cluster.spec.config.clone(),
            ..crate::api::ValkeyNodeSpec::default()
        },
        status: None,
    }
}

pub async fn ensure_node_config_map(client: kube::Client, node: &ValkeyNode) -> Result<()> {
    if !node.spec.server_config_map_name.is_empty() {
        return Ok(());
    }
    let namespace = node.namespace().unwrap_or_default();
    let cm = build_node_config_map(node, owner_reference(node));
    let name = cm.name_any();
    let api = Api::<ConfigMap>::namespaced(client, &namespace);
    apply(&api, &name, &cm).await?;
    Ok(())
}

pub fn build_pvc(node: &ValkeyNode) -> Option<PersistentVolumeClaim> {
    let persistence = node.spec.persistence.as_ref()?;
    let namespace = node.namespace().unwrap_or_default();
    let mut requests = BTreeMap::new();
    requests.insert("storage".to_string(), persistence.size.clone());
    Some(PersistentVolumeClaim {
        metadata: object_meta(
            valkey_node_pvc_name(node),
            namespace,
            valkey_node_labels(node),
            BTreeMap::new(),
            owner_reference(node),
        ),
        spec: Some(PersistentVolumeClaimSpec {
            access_modes: Some(vec!["ReadWriteOnce".to_string()]),
            storage_class_name: persistence.storage_class_name.clone(),
            resources: Some(VolumeResourceRequirements {
                requests: Some(requests),
                ..VolumeResourceRequirements::default()
            }),
            ..PersistentVolumeClaimSpec::default()
        }),
        ..PersistentVolumeClaim::default()
    })
}

pub async fn ensure_pvc(client: kube::Client, node: &ValkeyNode) -> Result<()> {
    let Some(pvc) = build_pvc(node) else {
        return Ok(());
    };
    let namespace = node.namespace().unwrap_or_default();
    let name = pvc.name_any();
    let api = Api::<PersistentVolumeClaim>::namespaced(client, &namespace);
    apply(&api, &name, &pvc).await?;
    Ok(())
}

pub async fn ensure_workload(client: kube::Client, node: &ValkeyNode) -> Result<()> {
    match node.spec.workload_type {
        WorkloadType::StatefulSet => {
            let workload = build_stateful_set(node)?;
            let namespace = node.namespace().unwrap_or_default();
            let name = workload.name_any();
            let api = Api::<StatefulSet>::namespaced(client, &namespace);
            apply(&api, &name, &workload).await?;
        }
        WorkloadType::Deployment => {
            let workload = build_deployment(node)?;
            let namespace = node.namespace().unwrap_or_default();
            let name = workload.name_any();
            let api = Api::<Deployment>::namespaced(client, &namespace);
            apply(&api, &name, &workload).await?;
        }
    }
    Ok(())
}

pub async fn delete_workload(client: kube::Client, node: &ValkeyNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_default();
    let name = valkey_node_resource_name(node);
    match node.spec.workload_type {
        WorkloadType::StatefulSet => {
            let api = Api::<StatefulSet>::namespaced(client, &namespace);
            if api.get_opt(&name).await?.is_some() {
                api.delete(&name, &Default::default()).await?;
            }
        }
        WorkloadType::Deployment => {
            let api = Api::<Deployment>::namespaced(client, &namespace);
            if api.get_opt(&name).await?.is_some() {
                api.delete(&name, &Default::default()).await?;
            }
        }
    }
    Ok(())
}

pub fn build_stateful_set(node: &ValkeyNode) -> Result<StatefulSet> {
    let labels = valkey_node_labels(node);
    let template = build_pod_template(node, &labels)?;
    let namespace = node.namespace().unwrap_or_default();
    Ok(StatefulSet {
        metadata: object_meta(
            valkey_node_resource_name(node),
            namespace,
            labels.clone(),
            BTreeMap::new(),
            owner_reference(node),
        ),
        spec: Some(StatefulSetSpec {
            replicas: Some(1),
            service_name: Some(valkey_node_resource_name(node)),
            selector: LabelSelector {
                match_labels: Some(labels),
                ..LabelSelector::default()
            },
            template,
            ..StatefulSetSpec::default()
        }),
        ..StatefulSet::default()
    })
}

pub fn build_deployment(node: &ValkeyNode) -> Result<Deployment> {
    let labels = valkey_node_labels(node);
    let template = build_pod_template(node, &labels)?;
    let namespace = node.namespace().unwrap_or_default();
    Ok(Deployment {
        metadata: object_meta(
            valkey_node_resource_name(node),
            namespace,
            labels.clone(),
            BTreeMap::new(),
            owner_reference(node),
        ),
        spec: Some(DeploymentSpec {
            replicas: Some(1),
            strategy: Some(DeploymentStrategy {
                type_: Some("Recreate".to_string()),
                ..DeploymentStrategy::default()
            }),
            selector: LabelSelector {
                match_labels: Some(labels),
                ..LabelSelector::default()
            },
            template,
            ..DeploymentSpec::default()
        }),
        ..Deployment::default()
    })
}

fn build_pod_template(
    node: &ValkeyNode,
    labels: &BTreeMap<String, String>,
) -> Result<PodTemplateSpec> {
    let mut containers = build_containers(node)?;
    let config_map_name = if node.spec.server_config_map_name.is_empty() {
        server_config_map_name(&node.name_any())
    } else {
        node.spec.server_config_map_name.clone()
    };
    let mut volumes = vec![
        Volume {
            name: "scripts".to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: config_map_name.clone(),
                default_mode: Some(0o755),
                ..ConfigMapVolumeSource::default()
            }),
            ..Volume::default()
        },
        Volume {
            name: "valkey-conf".to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: config_map_name,
                ..ConfigMapVolumeSource::default()
            }),
            ..Volume::default()
        },
    ];

    if !node.spec.users_acl_secret_name.is_empty() {
        volumes.push(Volume {
            name: "users-acl".to_string(),
            secret: Some(SecretVolumeSource {
                secret_name: Some(node.spec.users_acl_secret_name.clone()),
                ..SecretVolumeSource::default()
            }),
            ..Volume::default()
        });
        if let Some(server) = containers.first_mut() {
            server
                .volume_mounts
                .get_or_insert_with(Vec::new)
                .push(VolumeMount {
                    name: "users-acl".to_string(),
                    mount_path: "/config/users".to_string(),
                    read_only: Some(true),
                    ..VolumeMount::default()
                });
        }
    }
    if let Some(tls) = &node.spec.tls
        && let Some(secret_name) = tls_secret_name(tls)
    {
        volumes.push(Volume {
            name: TLS_VOLUME_NAME.to_string(),
            secret: Some(SecretVolumeSource {
                secret_name: Some(secret_name),
                ..SecretVolumeSource::default()
            }),
            ..Volume::default()
        });
    }
    if node.spec.persistence.is_some() {
        volumes.push(Volume {
            name: DATA_VOLUME_NAME.to_string(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: valkey_node_pvc_name(node),
                ..PersistentVolumeClaimVolumeSource::default()
            }),
            ..Volume::default()
        });
    }

    Ok(PodTemplateSpec {
        metadata: Some(ObjectMeta {
            labels: Some(labels.clone()),
            annotations: Some(pod_template_annotations(node)),
            ..ObjectMeta::default()
        }),
        spec: Some(PodSpec {
            containers,
            image_pull_secrets: (!node.spec.image_pull_secrets.is_empty())
                .then_some(node.spec.image_pull_secrets.clone()),
            node_selector: (!node.spec.node_selector.is_empty())
                .then_some(node.spec.node_selector.clone()),
            affinity: node.spec.affinity.clone(),
            tolerations: (!node.spec.tolerations.is_empty())
                .then_some(node.spec.tolerations.clone()),
            topology_spread_constraints: build_topology_spread_constraints(node, labels),
            volumes: Some(volumes),
            ..PodSpec::default()
        }),
    })
}

fn pod_template_annotations(node: &ValkeyNode) -> BTreeMap<String, String> {
    let mut annotations = BTreeMap::new();
    if !node.spec.server_config_hash.is_empty() {
        annotations.insert(
            CONFIG_HASH_KEY.to_string(),
            node.spec.server_config_hash.clone(),
        );
    }
    annotations
}

fn build_containers(node: &ValkeyNode) -> Result<Vec<Container>> {
    let image = if node.spec.image.is_empty() {
        DEFAULT_IMAGE.to_string()
    } else {
        node.spec.image.clone()
    };
    let mut server = Container {
        name: "server".to_string(),
        image: Some(image),
        resources: node.spec.resources.clone(),
        command: Some(vec![
            "valkey-server".to_string(),
            "/config/valkey.conf".to_string(),
            "--cluster-announce-ip".to_string(),
            "$(POD_IP)".to_string(),
        ]),
        env: Some(vec![EnvVar {
            name: "POD_IP".to_string(),
            value_from: Some(EnvVarSource {
                field_ref: Some(ObjectFieldSelector {
                    field_path: "status.podIP".to_string(),
                    ..ObjectFieldSelector::default()
                }),
                ..EnvVarSource::default()
            }),
            ..EnvVar::default()
        }]),
        ports: Some(vec![
            ContainerPort {
                name: Some("client".to_string()),
                container_port: DEFAULT_PORT,
                ..ContainerPort::default()
            },
            ContainerPort {
                name: Some("cluster-bus".to_string()),
                container_port: DEFAULT_CLUSTER_BUS_PORT,
                ..ContainerPort::default()
            },
        ]),
        startup_probe: Some(exec_probe("/scripts/liveness-check.sh", 5, 5, 20, 5)),
        liveness_probe: Some(exec_probe("/scripts/liveness-check.sh", 5, 5, 5, 5)),
        readiness_probe: Some(exec_probe("/scripts/readiness-check.sh", 5, 5, 5, 2)),
        volume_mounts: Some(vec![
            VolumeMount {
                name: "scripts".to_string(),
                mount_path: "/scripts".to_string(),
                ..VolumeMount::default()
            },
            VolumeMount {
                name: "valkey-conf".to_string(),
                mount_path: "/config".to_string(),
                read_only: Some(true),
                ..VolumeMount::default()
            },
        ]),
        ..Container::default()
    };
    if node.spec.persistence.is_some() {
        server
            .volume_mounts
            .get_or_insert_with(Vec::new)
            .push(VolumeMount {
                name: DATA_VOLUME_NAME.to_string(),
                mount_path: DATA_MOUNT_PATH.to_string(),
                ..VolumeMount::default()
            });
    }
    if node.spec.tls.is_some() {
        server
            .volume_mounts
            .get_or_insert_with(Vec::new)
            .push(VolumeMount {
                name: TLS_VOLUME_NAME.to_string(),
                mount_path: TLS_CERT_MOUNT_PATH.to_string(),
                read_only: Some(true),
                ..VolumeMount::default()
            });
        server.env.get_or_insert_with(Vec::new).extend([
            EnvVar {
                name: "VALKEY_TLS_ENABLED".to_string(),
                value: Some("true".to_string()),
                ..EnvVar::default()
            },
            EnvVar {
                name: "VALKEY_TLS_CA_FILE".to_string(),
                value: Some(format!("{TLS_CERT_MOUNT_PATH}/{TLS_SECRET_KEY_CA}")),
                ..EnvVar::default()
            },
            EnvVar {
                name: "VALKEY_TLS_CERT_FILE".to_string(),
                value: Some(format!("{TLS_CERT_MOUNT_PATH}/{TLS_SECRET_KEY_CERT}")),
                ..EnvVar::default()
            },
            EnvVar {
                name: "VALKEY_TLS_KEY_FILE".to_string(),
                value: Some(format!("{TLS_CERT_MOUNT_PATH}/{TLS_SECRET_KEY_KEY}")),
                ..EnvVar::default()
            },
            EnvVar {
                name: "VALKEY_TLS_ARGS".to_string(),
                value: Some(format!(
                    "--tls --cacert {TLS_CERT_MOUNT_PATH}/{TLS_SECRET_KEY_CA} --cert {TLS_CERT_MOUNT_PATH}/{TLS_SECRET_KEY_CERT} --key {TLS_CERT_MOUNT_PATH}/{TLS_SECRET_KEY_KEY}"
                )),
                ..EnvVar::default()
            },
        ]);
    }
    let mut containers = vec![server];
    if node.spec.exporter.enabled {
        let cluster_name = node
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get(LABEL_CLUSTER))
            .cloned()
            .unwrap_or_default();
        containers.push(metrics_exporter_container(
            &node.spec.exporter,
            &cluster_name,
            node.spec.tls.as_ref(),
        ));
    }
    merge_patch_containers(containers, &node.spec.containers)
}

fn exec_probe(
    command: &str,
    initial_delay: i32,
    period: i32,
    failure_threshold: i32,
    timeout: i32,
) -> Probe {
    Probe {
        initial_delay_seconds: Some(initial_delay),
        period_seconds: Some(period),
        failure_threshold: Some(failure_threshold),
        timeout_seconds: Some(timeout),
        success_threshold: Some(1),
        exec: Some(ExecAction {
            command: Some(vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                command.to_string(),
            ]),
        }),
        ..Probe::default()
    }
}

fn metrics_exporter_container(
    exporter: &ExporterSpec,
    cluster_name: &str,
    tls: Option<&TlsConfig>,
) -> Container {
    let image = if exporter.image.is_empty() {
        DEFAULT_EXPORTER_IMAGE.to_string()
    } else {
        exporter.image.clone()
    };
    let scheme = if tls.is_some() { "rediss" } else { "redis" };
    let mut args = vec![format!("--redis.addr={scheme}://localhost:{DEFAULT_PORT}")];
    let mut volume_mounts = Vec::new();
    if tls.is_some() {
        args.push(format!(
            "--tls-ca-cert-file={TLS_CERT_MOUNT_PATH}/{TLS_SECRET_KEY_CA}"
        ));
        volume_mounts.push(VolumeMount {
            name: TLS_VOLUME_NAME.to_string(),
            mount_path: TLS_CERT_MOUNT_PATH.to_string(),
            read_only: Some(true),
            ..VolumeMount::default()
        });
    }
    Container {
        name: "metrics-exporter".to_string(),
        image: Some(image),
        args: Some(args),
        env: Some(vec![
            EnvVar {
                name: "REDIS_USER".to_string(),
                value: Some(EXPORTER_USER.to_string()),
                ..EnvVar::default()
            },
            EnvVar {
                name: "REDIS_PASSWORD".to_string(),
                value_from: Some(EnvVarSource {
                    secret_key_ref: Some(SecretKeySelector {
                        name: system_password_secret_name(cluster_name),
                        key: EXPORTER_USER.to_string(),
                        ..SecretKeySelector::default()
                    }),
                    ..EnvVarSource::default()
                }),
                ..EnvVar::default()
            },
        ]),
        ports: Some(vec![ContainerPort {
            name: Some("metrics".to_string()),
            container_port: DEFAULT_EXPORTER_PORT,
            protocol: Some("TCP".to_string()),
            ..ContainerPort::default()
        }]),
        volume_mounts: (!volume_mounts.is_empty()).then_some(volume_mounts),
        liveness_probe: Some(http_probe("/health", 10, 10, 3)),
        readiness_probe: Some(http_probe("/health", 5, 1, 3)),
        resources: exporter.resources.clone(),
        ..Container::default()
    }
}

fn http_probe(path: &str, initial_delay: i32, period: i32, timeout: i32) -> Probe {
    Probe {
        initial_delay_seconds: Some(initial_delay),
        period_seconds: Some(period),
        timeout_seconds: Some(timeout),
        http_get: Some(HTTPGetAction {
            path: Some(path.to_string()),
            port: IntOrString::Int(DEFAULT_EXPORTER_PORT),
            ..HTTPGetAction::default()
        }),
        ..Probe::default()
    }
}

fn build_topology_spread_constraints(
    node: &ValkeyNode,
    labels: &BTreeMap<String, String>,
) -> Option<Vec<k8s_openapi::api::core::v1::TopologySpreadConstraint>> {
    if node.spec.topology_spread_constraints.is_empty() {
        return None;
    }
    let cluster_name = labels.get(LABEL_CLUSTER).cloned().unwrap_or_default();
    let shard_index = labels.get(LABEL_SHARD_INDEX).cloned().unwrap_or_default();
    Some(
        node.spec
            .topology_spread_constraints
            .iter()
            .cloned()
            .map(|mut constraint| {
                let selector = constraint
                    .label_selector
                    .get_or_insert_with(LabelSelector::default);
                let uses_cluster_label = label_selector_uses_key(selector, LABEL_CLUSTER);
                let match_labels = selector.match_labels.get_or_insert_with(BTreeMap::new);
                if !cluster_name.is_empty() && !uses_cluster_label {
                    match_labels.insert(LABEL_CLUSTER.to_string(), cluster_name.clone());
                }
                if !shard_index.is_empty()
                    && !topology_spread_constraint_uses_key(&constraint, LABEL_SHARD_INDEX)
                {
                    constraint
                        .match_label_keys
                        .get_or_insert_with(Vec::new)
                        .push(LABEL_SHARD_INDEX.to_string());
                }
                constraint
            })
            .collect(),
    )
}

fn topology_spread_constraint_uses_key(
    constraint: &k8s_openapi::api::core::v1::TopologySpreadConstraint,
    key: &str,
) -> bool {
    constraint
        .label_selector
        .as_ref()
        .is_some_and(|selector| label_selector_uses_key(selector, key))
        || constraint
            .match_label_keys
            .as_ref()
            .is_some_and(|keys| keys.iter().any(|candidate| candidate == key))
}

fn label_selector_uses_key(selector: &LabelSelector, key: &str) -> bool {
    selector
        .match_labels
        .as_ref()
        .is_some_and(|labels| labels.contains_key(key))
        || selector
            .match_expressions
            .as_ref()
            .is_some_and(|expressions| expressions.iter().any(|expression| expression.key == key))
}

fn merge_patch_containers(base: Vec<Container>, patches: &[Container]) -> Result<Vec<Container>> {
    let mut output = Vec::new();
    let mut patch_by_name = patches
        .iter()
        .map(|container| (container.name.clone(), container.clone()))
        .collect::<BTreeMap<_, _>>();

    for container in base {
        if let Some(patch) = patch_by_name.remove(&container.name) {
            output.push(merge_container(container, patch)?);
        } else {
            output.push(container);
        }
    }
    for patch in patches {
        if patch_by_name.contains_key(&patch.name) {
            output.push(patch.clone());
        }
    }
    Ok(output)
}

fn merge_container(base: Container, patch: Container) -> Result<Container> {
    let mut base_value = serde_json::to_value(base)?;
    let patch_value = serde_json::to_value(patch)?;
    merge_json(&mut base_value, patch_value);
    Ok(serde_json::from_value(base_value)?)
}

fn merge_json(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                if value.is_null() {
                    continue;
                }
                merge_json(base.entry(key).or_insert(Value::Null), value);
            }
        }
        (base, patch) => *base = patch,
    }
}

pub fn tls_secret_name(tls: &TlsConfig) -> Option<String> {
    tls.certificate
        .as_ref()
        .map(|cert| cert.secret_name.clone())
        .filter(|name| !name.is_empty())
}

#[allow(dead_code)]
fn _keep_imports(_: LocalObjectReference, _: Secret, _: Pod, _: Quantity) {}
