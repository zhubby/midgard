use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::ByteString;
use kube::{Api, ResourceExt};
use rand::distributions::Alphanumeric;
use rand::Rng;
use sha2::{Digest, Sha256};

use crate::api::{UserAclSpec, ValkeyCluster};
use crate::controller::config::sha256_hex;
use crate::controller::{
    apply, cluster_labels, object_meta, owner_reference, ACL_SECRET_TYPE, HASH_ANNOTATION_KEY,
};
use crate::error::{Error, Result};

pub const OPERATOR_USER: &str = "_operator";
pub const EXPORTER_USER: &str = "_exporter";
pub const ACL_FILENAME: &str = "users.acl";
const PASSWORD_LENGTH: usize = 26;

pub fn internal_acl_secret_name(cluster_name: &str) -> String {
    format!("internal-{cluster_name}-acl")
}

pub fn default_user_secret_name(cluster_name: &str) -> String {
    format!("{cluster_name}-users")
}

pub fn system_password_secret_name(cluster_name: &str) -> String {
    format!("internal-{cluster_name}-system-passwords")
}

pub async fn reconcile_users_acl(client: kube::Client, cluster: &ValkeyCluster) -> Result<()> {
    let mut users = cluster.spec.users.clone();
    users.sort_by(|a, b| a.name.cmp(&b.name));
    let mut acl = String::new();
    for user in users {
        let passwords = fetch_user_passwords(
            client.clone(),
            &user,
            &cluster.name_any(),
            &cluster.namespace().unwrap_or_default(),
        )
        .await?;
        acl.push_str(&build_user_acl(&user, &passwords));
        acl.push('\n');
    }
    acl.push_str(&create_system_users_acl(client.clone(), cluster).await?);
    acl.push('\n');
    upsert_internal_acl_secret(client, cluster, acl.into_bytes()).await
}

pub async fn fetch_system_user_password(
    client: kube::Client,
    username: &str,
    cluster_name: &str,
    namespace: &str,
) -> Result<String> {
    let api = Api::<Secret>::namespaced(client, namespace);
    let secret = api.get(&system_password_secret_name(cluster_name)).await?;
    let password = secret
        .data
        .unwrap_or_default()
        .remove(username)
        .map(|bytes| String::from_utf8_lossy(&bytes.0).into_owned())
        .unwrap_or_default();
    Ok(password)
}

async fn create_system_users_acl(client: kube::Client, cluster: &ValkeyCluster) -> Result<String> {
    let secret = upsert_system_users_password_secret(client.clone(), cluster).await?;
    let mut out = String::new();
    for user in [OPERATOR_USER, EXPORTER_USER] {
        if user == EXPORTER_USER && !cluster.spec.exporter.enabled {
            continue;
        }
        let data = secret
            .data
            .as_ref()
            .and_then(|data| data.get(user))
            .ok_or_else(|| Error::InvalidState(format!("system password for {user} missing")))?;
        let password_hash = sha256_hex(&data.0);
        let user_acl = UserAclSpec {
            name: user.to_string(),
            enabled: true,
            raw_acl: system_user_acl(user).to_string(),
            password_secret: crate::api::PasswordSecretSpec {
                name: secret.name_any(),
                keys: vec![user.to_string()],
            },
            ..UserAclSpec::default()
        };
        out.push_str(&build_user_acl(&user_acl, &[password_hash]));
        out.push('\n');
    }
    Ok(out)
}

fn system_user_acl(user: &str) -> &'static str {
    match user {
        OPERATOR_USER => {
            "+@connection +cluster|myid +cluster|myshardid +cluster|info +cluster|nodes +cluster|meet +cluster|addslotsrange +cluster|replicate +cluster|forget +cluster|failover +cluster|getslotmigrations +cluster|migrateslots +cluster|set-config-epoch +config|set +info"
        }
        EXPORTER_USER => {
            "-@all +@connection +memory -readonly +strlen +config|get +xinfo +pfcount -quit +zcard +type +xlen -readwrite -command +client -wait +scard +llen +hlen +get +eval +slowlog +cluster|info +cluster|slots +cluster|nodes -hello -echo +info +latency +scan -reset -auth -asking"
        }
        _ => "",
    }
}

async fn fetch_user_passwords(
    client: kube::Client,
    user: &UserAclSpec,
    cluster_name: &str,
    namespace: &str,
) -> Result<Vec<String>> {
    if user.no_password || !user.enabled || user.reset_pass {
        return Ok(Vec::new());
    }
    let secret_name = if user.password_secret.name.is_empty() {
        default_user_secret_name(cluster_name)
    } else {
        user.password_secret.name.clone()
    };
    let api = Api::<Secret>::namespaced(client, namespace);
    let secret = api
        .get_opt(&secret_name)
        .await?
        .ok_or_else(|| Error::NotFound(format!("password secret {secret_name}")))?;
    let data = secret.data.unwrap_or_default();
    let mut keys = user.password_secret.keys.clone();
    if keys.is_empty() {
        keys.push(user.name.clone());
    }
    keys.sort();
    let mut passwords = Vec::with_capacity(keys.len());
    for key in keys {
        let password = data.get(&key).ok_or_else(|| {
            Error::InvalidState(format!(
                "missing password key {key} in secret {secret_name}"
            ))
        })?;
        if is_prehashed_password(&password.0) {
            passwords.push(String::from_utf8_lossy(&password.0[1..]).into_owned());
        } else {
            let mut hasher = Sha256::new();
            hasher.update(&password.0);
            passwords.push(hex::encode(hasher.finalize()));
        }
    }
    Ok(passwords)
}

pub fn build_user_acl(user: &UserAclSpec, passwords: &[String]) -> String {
    let mut acl = format!(
        "user {} {}",
        user.name,
        if user.enabled { "on" } else { "off" }
    );
    if !user.reset_pass {
        if user.no_password {
            acl.push_str(" nopass");
        } else {
            append_acl(&mut acl, passwords, "#");
        }
    }
    append_acl(&mut acl, &user.keys.read_write, "~");
    append_acl(&mut acl, &user.keys.read_only, "%R~");
    append_acl(&mut acl, &user.keys.write_only, "%W~");
    if !user.channels.patterns.is_empty() {
        acl.push_str(" resetchannels");
        append_acl(&mut acl, &user.channels.patterns, "&");
    }
    append_acl(&mut acl, &user.commands.allow, "+");
    append_acl(&mut acl, &user.commands.deny, "-");
    acl.push(' ');
    acl.push_str(&user.raw_acl);
    acl
}

fn append_acl(out: &mut String, permissions: &[String], prefix: &str) {
    for permission in permissions {
        out.push(' ');
        out.push_str(prefix);
        out.push_str(permission);
    }
}

fn is_prehashed_password(password: &[u8]) -> bool {
    password.first() == Some(&b'#') && password.len() == 65
}

async fn upsert_system_users_password_secret(
    client: kube::Client,
    cluster: &ValkeyCluster,
) -> Result<Secret> {
    let namespace = cluster.namespace().unwrap_or_default();
    let name = system_password_secret_name(&cluster.name_any());
    let api = Api::<Secret>::namespaced(client, &namespace);
    let mut secret = if let Some(existing) = api.get_opt(&name).await? {
        existing
    } else {
        Secret {
            metadata: object_meta(
                name.clone(),
                namespace.clone(),
                cluster_labels(cluster),
                BTreeMap::new(),
                owner_reference(cluster),
            ),
            type_: Some(ACL_SECRET_TYPE.to_string()),
            data: Some(BTreeMap::new()),
            ..Secret::default()
        }
    };
    let data = secret.data.get_or_insert_with(BTreeMap::new);
    for user in [OPERATOR_USER, EXPORTER_USER] {
        if user == EXPORTER_USER && !cluster.spec.exporter.enabled {
            continue;
        }
        data.entry(user.to_string())
            .or_insert_with(|| ByteString(generate_password(PASSWORD_LENGTH).into_bytes()));
    }
    secret.metadata.labels = Some(cluster_labels(cluster));
    secret.metadata.owner_references = owner_reference(cluster).map(|owner| vec![owner]);
    secret.type_ = Some(ACL_SECRET_TYPE.to_string());
    apply(&api, &name, &secret).await
}

async fn upsert_internal_acl_secret(
    client: kube::Client,
    cluster: &ValkeyCluster,
    acl_bytes: Vec<u8>,
) -> Result<()> {
    let namespace = cluster.namespace().unwrap_or_default();
    let name = internal_acl_secret_name(&cluster.name_any());
    let mut annotations = BTreeMap::new();
    annotations.insert(HASH_ANNOTATION_KEY.to_string(), sha256_hex(&acl_bytes));
    let secret = Secret {
        metadata: object_meta(
            name.clone(),
            namespace.clone(),
            cluster_labels(cluster),
            annotations,
            owner_reference(cluster),
        ),
        type_: Some(ACL_SECRET_TYPE.to_string()),
        data: Some(BTreeMap::from([(
            ACL_FILENAME.to_string(),
            ByteString(acl_bytes),
        )])),
        ..Secret::default()
    };
    let api = Api::<Secret>::namespaced(client, &namespace);
    apply(&api, &name, &secret).await?;
    Ok(())
}

fn generate_password(length: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ChannelsAclSpec, CommandsAclSpec, KeysAclSpec, PasswordSecretSpec};

    #[test]
    fn build_user_acl_renders_passwords_keys_channels_commands_and_raw_acl() {
        let user = UserAclSpec {
            name: "alice".to_string(),
            enabled: true,
            password_secret: PasswordSecretSpec {
                name: "alice-secret".to_string(),
                keys: vec!["alice".to_string()],
            },
            commands: CommandsAclSpec {
                allow: vec![
                    "@read".to_string(),
                    "@write".to_string(),
                    "@connection".to_string(),
                ],
                deny: vec!["@admin".to_string(), "@dangerous".to_string()],
            },
            keys: KeysAclSpec {
                read_write: vec!["app:*".to_string(), "cache:*".to_string()],
                read_only: vec!["shared:*".to_string(), "config:*".to_string()],
                write_only: vec!["logs:*".to_string(), "metrics:*".to_string()],
            },
            channels: ChannelsAclSpec {
                patterns: vec!["notifications:*".to_string(), "events:*".to_string()],
            },
            raw_acl: "+client|setname +debug".to_string(),
            ..UserAclSpec::default()
        };

        let acl = build_user_acl(
            &user,
            &["a71153805265764af6f55b4e0ce38858cde64e6e24b9a9b14e32262760572137".to_string()],
        );

        assert_eq!(
            acl.trim(),
            "user alice on #a71153805265764af6f55b4e0ce38858cde64e6e24b9a9b14e32262760572137 ~app:* ~cache:* %R~shared:* %R~config:* %W~logs:* %W~metrics:* resetchannels &notifications:* &events:* +@read +@write +@connection -@admin -@dangerous +client|setname +debug"
        );
    }

    #[test]
    fn build_user_acl_renders_nopass_disabled_and_resetpass_cases() {
        let bob = UserAclSpec {
            name: "bob".to_string(),
            no_password: true,
            raw_acl: "+@all -@admin ~* &*".to_string(),
            ..UserAclSpec::default()
        };
        let charlie = UserAclSpec {
            name: "charlie".to_string(),
            enabled: false,
            no_password: true,
            commands: CommandsAclSpec {
                allow: vec!["@admin".to_string()],
                ..CommandsAclSpec::default()
            },
            ..UserAclSpec::default()
        };
        let edward = UserAclSpec {
            name: "edward".to_string(),
            no_password: true,
            reset_pass: true,
            commands: CommandsAclSpec {
                allow: vec!["@admin".to_string()],
                ..CommandsAclSpec::default()
            },
            ..UserAclSpec::default()
        };

        assert_eq!(
            build_user_acl(&bob, &[]).trim(),
            "user bob on nopass +@all -@admin ~* &*"
        );
        assert_eq!(
            build_user_acl(&charlie, &[]).trim(),
            "user charlie off nopass +@admin"
        );
        assert_eq!(
            build_user_acl(&edward, &[]).trim(),
            "user edward on +@admin"
        );
    }

    #[test]
    fn build_user_acl_renders_multiple_password_hashes() {
        let user = UserAclSpec {
            name: "david".to_string(),
            commands: CommandsAclSpec {
                allow: vec!["@admin".to_string()],
                ..CommandsAclSpec::default()
            },
            ..UserAclSpec::default()
        };

        let acl = build_user_acl(
            &user,
            &[
                "7447bd019c69af5975c54072b40f9c24d1105836cbd68408d6df7be76ac42ab1".to_string(),
                "4b31cf3c1347d94fe80efb0c848579c5730d63efef2f5eaf32f78a7ca251833b".to_string(),
            ],
        );

        assert_eq!(
            acl.trim(),
            "user david on #7447bd019c69af5975c54072b40f9c24d1105836cbd68408d6df7be76ac42ab1 #4b31cf3c1347d94fe80efb0c848579c5730d63efef2f5eaf32f78a7ca251833b +@admin"
        );
    }
}
