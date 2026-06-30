use std::collections::{BTreeMap, BTreeSet};

use k8s_openapi::api::core::v1::ConfigMap;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::{Api, ResourceExt};
use sha2::{Digest, Sha256};

use crate::api::{PersistenceSpec, TlsConfig, ValkeyCluster, ValkeyNode};
use crate::controller::{
    CONFIG_FILE_KEY, CONFIG_HASH_KEY, DATA_MOUNT_PATH, LIVENESS_SCRIPT_KEY, READINESS_SCRIPT_KEY,
    SCRIPTS_HASH_KEY, TLS_CERT_MOUNT_PATH, TLS_SECRET_KEY_CA, TLS_SECRET_KEY_CERT,
    TLS_SECRET_KEY_KEY, apply, cluster_labels, object_meta, owner_reference,
    server_config_map_name,
};
use crate::error::Result;

pub const READINESS_SCRIPT: &str = include_str!("../../assets/scripts/readiness-check.sh");
pub const LIVENESS_SCRIPT: &str = include_str!("../../assets/scripts/liveness-check.sh");

pub fn scripts_hash() -> String {
    let mut hasher = Sha256::new();
    hasher.update(READINESS_SCRIPT.as_bytes());
    hasher.update(LIVENESS_SCRIPT.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn build_managed_config(
    include_acl: bool,
    persistence: Option<&PersistenceSpec>,
    tls: Option<&TlsConfig>,
) -> BTreeMap<String, String> {
    let mut config = BTreeMap::new();
    if include_acl {
        config.insert("aclfile".to_string(), "/config/users/users.acl".to_string());
    }
    if persistence.is_some() {
        config.insert("dir".to_string(), DATA_MOUNT_PATH.to_string());
        config.insert(
            "cluster-config-file".to_string(),
            format!("{DATA_MOUNT_PATH}/nodes.conf"),
        );
    }
    if tls.is_some() {
        config.insert(
            "tls-port".to_string(),
            crate::controller::DEFAULT_PORT.to_string(),
        );
        config.insert("port".to_string(), "0".to_string());
        config.insert("tls-cluster".to_string(), "yes".to_string());
        config.insert("tls-replication".to_string(), "yes".to_string());
        config.insert(
            "tls-cert-file".to_string(),
            format!("{TLS_CERT_MOUNT_PATH}/{TLS_SECRET_KEY_CERT}"),
        );
        config.insert(
            "tls-key-file".to_string(),
            format!("{TLS_CERT_MOUNT_PATH}/{TLS_SECRET_KEY_KEY}"),
        );
        config.insert(
            "tls-ca-cert-file".to_string(),
            format!("{TLS_CERT_MOUNT_PATH}/{TLS_SECRET_KEY_CA}"),
        );
        config.insert("tls-auth-clients".to_string(), "optional".to_string());
    }
    config
}

pub fn generate_valkey_node_config(node: &ValkeyNode) -> String {
    render_config(&build_managed_config(
        !node.spec.users_acl_secret_name.is_empty(),
        node.spec.persistence.as_ref(),
        node.spec.tls.as_ref(),
    ))
}

pub fn build_base_config(cluster: &ValkeyCluster) -> BTreeMap<String, String> {
    let mut config = build_managed_config(
        true,
        cluster.spec.persistence.as_ref(),
        cluster.spec.tls.as_ref(),
    );
    config.extend(BTreeMap::from([
        ("cluster-enabled".to_string(), "yes".to_string()),
        ("protected-mode".to_string(), "no".to_string()),
        ("cluster-node-timeout".to_string(), "2000".to_string()),
        (
            "cluster-allow-replica-migration".to_string(),
            "no".to_string(),
        ),
        (
            "cluster-replica-validity-factor".to_string(),
            "0".to_string(),
        ),
    ]));
    config
}

pub fn live_config_allowlist() -> BTreeSet<String> {
    ["maxmemory-policy", "maxmemory", "maxclients"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub fn live_config_to_apply(config: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let allow = live_config_allowlist();
    config
        .iter()
        .filter(|(key, _)| allow.contains(*key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub fn render_server_config(
    cluster: &ValkeyCluster,
    exclude_user_keys: &BTreeSet<String>,
) -> String {
    let base = build_base_config(cluster);
    let mut output = String::new();
    let user_keys = cluster
        .spec
        .config
        .keys()
        .filter(|key| !exclude_user_keys.contains(*key))
        .cloned()
        .collect::<Vec<_>>();
    if !user_keys.is_empty() {
        write_config_line(&mut output, "#", "User Config");
        for key in user_keys {
            if let Some(value) = cluster.spec.config.get(&key) {
                write_config_line(&mut output, &key, value);
            }
        }
    }
    write_config_line(&mut output, "#", "Base Config");
    for (key, value) in base {
        write_config_line(&mut output, &key, &value);
    }
    output
}

pub fn build_server_config(cluster: &ValkeyCluster) -> String {
    render_server_config(cluster, &BTreeSet::new())
}

pub fn build_roll_server_config(cluster: &ValkeyCluster) -> String {
    render_server_config(cluster, &live_config_allowlist())
}

pub fn server_config_roll_hash(cluster: &ValkeyCluster) -> String {
    sha256_hex(build_roll_server_config(cluster).as_bytes())
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

pub async fn upsert_cluster_config_map(
    client: kube::Client,
    cluster: &ValkeyCluster,
) -> Result<()> {
    let namespace = cluster.namespace().unwrap_or_default();
    let name = server_config_map_name(&cluster.name_any());
    let server_config = build_server_config(cluster);
    let mut annotations = BTreeMap::new();
    annotations.insert(
        CONFIG_HASH_KEY.to_string(),
        sha256_hex(server_config.as_bytes()),
    );
    annotations.insert(SCRIPTS_HASH_KEY.to_string(), scripts_hash());
    let cm = ConfigMap {
        metadata: object_meta(
            name.clone(),
            namespace.clone(),
            cluster_labels(cluster),
            annotations,
            owner_reference(cluster),
        ),
        data: Some(BTreeMap::from([
            (
                READINESS_SCRIPT_KEY.to_string(),
                READINESS_SCRIPT.to_string(),
            ),
            (LIVENESS_SCRIPT_KEY.to_string(), LIVENESS_SCRIPT.to_string()),
            (CONFIG_FILE_KEY.to_string(), server_config),
        ])),
        ..ConfigMap::default()
    };
    let api = Api::<ConfigMap>::namespaced(client, &namespace);
    apply(&api, &name, &cm).await?;
    Ok(())
}

pub fn build_node_config_map(node: &ValkeyNode, owner: Option<OwnerReference>) -> ConfigMap {
    let name = server_config_map_name(&node.name_any());
    let namespace = node.namespace().unwrap_or_default();
    ConfigMap {
        metadata: object_meta(
            name,
            namespace,
            crate::controller::valkey_node_labels(node),
            BTreeMap::new(),
            owner,
        ),
        data: Some(BTreeMap::from([
            (
                READINESS_SCRIPT_KEY.to_string(),
                READINESS_SCRIPT.to_string(),
            ),
            (LIVENESS_SCRIPT_KEY.to_string(), LIVENESS_SCRIPT.to_string()),
            (
                CONFIG_FILE_KEY.to_string(),
                generate_valkey_node_config(node),
            ),
        ])),
        ..ConfigMap::default()
    }
}

fn render_config(config: &BTreeMap<String, String>) -> String {
    let mut output = String::new();
    for (key, value) in config {
        write_config_line(&mut output, key, value);
    }
    output
}

fn write_config_line(output: &mut String, name: &str, value: &str) {
    output.push_str(name);
    output.push(' ');
    output.push_str(value);
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ValkeyClusterSpec;

    fn cluster_with_config(config: BTreeMap<String, String>) -> ValkeyCluster {
        ValkeyCluster::new(
            "example",
            ValkeyClusterSpec {
                config,
                ..ValkeyClusterSpec::default()
            },
        )
    }

    #[test]
    fn server_config_includes_user_and_base_configuration() {
        let cluster = cluster_with_config(BTreeMap::from([
            ("maxmemory".to_string(), "50mb".to_string()),
            ("maxmemory-policy".to_string(), "allkeys-lfu".to_string()),
        ]));

        let config = build_server_config(&cluster);

        assert!(config.contains("maxmemory-policy allkeys-lfu"));
        assert!(config.contains("cluster-enabled yes"));
    }

    #[test]
    fn roll_config_excludes_live_keys_but_keeps_other_user_and_base_config() {
        let cluster = cluster_with_config(BTreeMap::from([
            ("maxmemory-policy".to_string(), "allkeys-lru".to_string()),
            ("appendonly".to_string(), "yes".to_string()),
        ]));

        let roll_config = build_roll_server_config(&cluster);

        assert!(!roll_config.contains("maxmemory-policy"));
        assert!(roll_config.contains("appendonly yes"));
        assert!(roll_config.contains("cluster-enabled yes"));
    }

    #[test]
    fn roll_hash_is_stable_when_only_live_config_changes() {
        let before = server_config_roll_hash(&cluster_with_config(BTreeMap::from([
            ("maxmemory-policy".to_string(), "allkeys-lru".to_string()),
            ("appendonly".to_string(), "yes".to_string()),
        ])));
        let after = server_config_roll_hash(&cluster_with_config(BTreeMap::from([
            ("maxmemory-policy".to_string(), "volatile-lru".to_string()),
            ("appendonly".to_string(), "yes".to_string()),
        ])));

        assert_eq!(after, before);
    }

    #[test]
    fn roll_hash_changes_when_non_live_config_changes() {
        let before = server_config_roll_hash(&cluster_with_config(BTreeMap::from([(
            "appendonly".to_string(),
            "yes".to_string(),
        )])));
        let after = server_config_roll_hash(&cluster_with_config(BTreeMap::from([(
            "appendonly".to_string(),
            "no".to_string(),
        )])));

        assert_ne!(after, before);
    }

    #[test]
    fn live_config_to_apply_keeps_only_allowlisted_keys() {
        let config = BTreeMap::from([
            ("maxmemory-policy".to_string(), "allkeys-lru".to_string()),
            ("appendonly".to_string(), "yes".to_string()),
        ]);

        assert_eq!(
            live_config_to_apply(&config),
            BTreeMap::from([("maxmemory-policy".to_string(), "allkeys-lru".to_string())])
        );
    }
}
