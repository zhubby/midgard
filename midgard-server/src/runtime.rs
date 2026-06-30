use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use http::Uri;
use midgard_core::{MidgardError, MidgardResult};
use midgard_storage::{
    DockerRuntimeConfigView, KubernetesRuntimeConfigView, WorkspaceRuntimeConfigRecord,
    WorkspaceRuntimeConfigStatus, WorkspaceRuntimeConfigView, WorkspaceRuntimeMode,
};
use rand::{RngCore, rngs::OsRng};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{AppError, WorkspaceRuntimeConfigInput};

#[derive(Clone, Debug, Default)]
pub struct WorkspaceCredentialSettings {
    pub encryption_key: Option<String>,
}

impl WorkspaceCredentialSettings {
    pub fn new(encryption_key: impl Into<Option<String>>) -> Self {
        Self {
            encryption_key: encryption_key
                .into()
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty()),
        }
    }
}

pub fn prepare_workspace_runtime_config(
    settings: &WorkspaceCredentialSettings,
    input: WorkspaceRuntimeConfigInput,
) -> Result<WorkspaceRuntimeConfigRecord, AppError> {
    let key = settings.encryption_key.as_deref().ok_or_else(|| {
        AppError::Midgard(MidgardError::Configuration(
            "secrets.workspace_credentials_key is required to save workspace runtime credentials"
                .to_string(),
        ))
    })?;

    match input {
        WorkspaceRuntimeConfigInput::Docker {
            docker_api_url,
            allow_insecure_local_endpoint,
        } => {
            let (endpoint_host, normalized) =
                validate_docker_endpoint(&docker_api_url, allow_insecure_local_endpoint)?;
            let view = WorkspaceRuntimeConfigView {
                mode: Some(WorkspaceRuntimeMode::Docker),
                status: WorkspaceRuntimeConfigStatus::Configured,
                updated_at: Some(midgard_storage::utc_now_rfc3339()),
                docker: Some(DockerRuntimeConfigView {
                    configured: true,
                    endpoint_host: Some(endpoint_host),
                    insecure_allowed: allow_insecure_local_endpoint,
                }),
                kubernetes: None,
            };
            Ok(WorkspaceRuntimeConfigRecord {
                view,
                ciphertext: encrypt_runtime_secret(key, normalized.as_bytes())?,
            })
        }
        WorkspaceRuntimeConfigInput::Kubernetes { kubeconfig } => {
            let summary = summarize_kubeconfig(&kubeconfig)?;
            let view = WorkspaceRuntimeConfigView {
                mode: Some(WorkspaceRuntimeMode::Kubernetes),
                status: WorkspaceRuntimeConfigStatus::Configured,
                updated_at: Some(midgard_storage::utc_now_rfc3339()),
                docker: None,
                kubernetes: Some(summary),
            };
            Ok(WorkspaceRuntimeConfigRecord {
                view,
                ciphertext: encrypt_runtime_secret(key, kubeconfig.as_bytes())?,
            })
        }
    }
}

fn validate_docker_endpoint(
    value: &str,
    allow_insecure_local_endpoint: bool,
) -> Result<(String, String), AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "docker_api_url is required".to_string(),
        ));
    }

    let uri = trimmed
        .parse::<Uri>()
        .map_err(|_| AppError::BadRequest("docker_api_url must be a valid URL".to_string()))?;
    let scheme = uri.scheme_str().ok_or_else(|| {
        AppError::BadRequest("docker_api_url must include http or https scheme".to_string())
    })?;
    if scheme != "https" && !(scheme == "http" && allow_insecure_local_endpoint) {
        return Err(AppError::BadRequest(
            "docker_api_url must use https unless allow_insecure_local_endpoint is true"
                .to_string(),
        ));
    }
    let host = uri
        .host()
        .ok_or_else(|| AppError::BadRequest("docker_api_url must include a host".to_string()))?
        .to_string();

    Ok((host, trimmed.to_string()))
}

fn summarize_kubeconfig(value: &str) -> Result<KubernetesRuntimeConfigView, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("kubeconfig is required".to_string()));
    }
    let config: KubeConfigSummary = serde_yaml::from_str(trimmed)
        .map_err(|_| AppError::BadRequest("kubeconfig must be valid YAML".to_string()))?;
    let context_name = config
        .current_context
        .or_else(|| config.contexts.first().map(|context| context.name.clone()));
    let cluster_name = context_name.as_ref().and_then(|context_name| {
        config
            .contexts
            .iter()
            .find(|context| &context.name == context_name)
            .map(|context| context.context.cluster.clone())
    });
    let cluster_server_host = cluster_name
        .as_ref()
        .and_then(|cluster_name| {
            config
                .clusters
                .iter()
                .find(|cluster| &cluster.name == cluster_name)
        })
        .and_then(|cluster| {
            cluster
                .cluster
                .server
                .parse::<Uri>()
                .ok()
                .and_then(|uri| uri.host().map(ToString::to_string))
        });

    Ok(KubernetesRuntimeConfigView {
        configured: true,
        context_name,
        cluster_server_host,
    })
}

fn encrypt_runtime_secret(key: &str, plaintext: &[u8]) -> MidgardResult<String> {
    let key_hash = Sha256::digest(key.as_bytes());
    let cipher = Aes256Gcm::new_from_slice(&key_hash).map_err(|err| {
        MidgardError::Configuration(format!(
            "invalid workspace credential encryption key: {err}"
        ))
    })?;
    let mut nonce_bytes = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|err| {
            MidgardError::Configuration(format!("encrypt workspace credentials: {err}"))
        })?;

    Ok(format!(
        "v1:{}:{}",
        STANDARD_NO_PAD.encode(nonce_bytes),
        STANDARD_NO_PAD.encode(ciphertext)
    ))
}

pub(crate) fn decrypt_workspace_runtime_secret(
    settings: &WorkspaceCredentialSettings,
    ciphertext: &str,
) -> Result<String, AppError> {
    let key = settings.encryption_key.as_deref().ok_or_else(|| {
        AppError::Midgard(MidgardError::Configuration(
            "secrets.workspace_credentials_key is required to read workspace runtime credentials"
                .to_string(),
        ))
    })?;
    let mut parts = ciphertext.split(':');
    let version = parts.next();
    let nonce = parts.next();
    let encrypted = parts.next();
    if version != Some("v1") || nonce.is_none() || encrypted.is_none() || parts.next().is_some() {
        return Err(AppError::Midgard(MidgardError::Configuration(
            "workspace runtime credential ciphertext has unsupported format".to_string(),
        )));
    }
    let nonce = STANDARD_NO_PAD.decode(nonce.unwrap()).map_err(|err| {
        AppError::Midgard(MidgardError::Configuration(format!(
            "decode workspace credential nonce: {err}"
        )))
    })?;
    if nonce.len() != 12 {
        return Err(AppError::Midgard(MidgardError::Configuration(
            "workspace runtime credential nonce has invalid length".to_string(),
        )));
    }
    let encrypted = STANDARD_NO_PAD.decode(encrypted.unwrap()).map_err(|err| {
        AppError::Midgard(MidgardError::Configuration(format!(
            "decode workspace credential ciphertext: {err}"
        )))
    })?;
    let key_hash = Sha256::digest(key.as_bytes());
    let cipher = Aes256Gcm::new_from_slice(&key_hash).map_err(|err| {
        MidgardError::Configuration(format!(
            "invalid workspace credential encryption key: {err}"
        ))
    })?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), encrypted.as_ref())
        .map_err(|err| {
            AppError::Midgard(MidgardError::Configuration(format!(
                "decrypt workspace credentials: {err}"
            )))
        })?;

    String::from_utf8(plaintext).map_err(|err| {
        AppError::Midgard(MidgardError::Configuration(format!(
            "workspace runtime credential is not valid UTF-8: {err}"
        )))
    })
}

#[derive(Debug, Deserialize)]
struct KubeConfigSummary {
    #[serde(rename = "current-context")]
    current_context: Option<String>,
    #[serde(default)]
    contexts: Vec<NamedKubeContext>,
    #[serde(default)]
    clusters: Vec<NamedKubeCluster>,
}

#[derive(Debug, Deserialize)]
struct NamedKubeContext {
    name: String,
    context: KubeContextRef,
}

#[derive(Debug, Deserialize)]
struct KubeContextRef {
    cluster: String,
}

#[derive(Debug, Deserialize)]
struct NamedKubeCluster {
    name: String,
    cluster: KubeClusterRef,
}

#[derive(Debug, Deserialize)]
struct KubeClusterRef {
    server: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_runtime_config_encrypts_endpoint_and_can_be_decrypted_for_server_execution() {
        let settings = WorkspaceCredentialSettings::new(Some("test workspace key".to_string()));
        let record = prepare_workspace_runtime_config(
            &settings,
            WorkspaceRuntimeConfigInput::Docker {
                docker_api_url: "https://secret-docker.example.com:2376".to_string(),
                allow_insecure_local_endpoint: false,
            },
        )
        .unwrap();

        assert_eq!(record.view.mode, Some(WorkspaceRuntimeMode::Docker));
        assert_eq!(record.view.status, WorkspaceRuntimeConfigStatus::Configured);
        assert_eq!(
            record
                .view
                .docker
                .as_ref()
                .and_then(|docker| docker.endpoint_host.as_deref()),
            Some("secret-docker.example.com")
        );
        assert!(record.ciphertext.starts_with("v1:"));
        assert!(
            !record
                .ciphertext
                .contains("https://secret-docker.example.com:2376")
        );

        let plaintext = decrypt_workspace_runtime_secret(&settings, &record.ciphertext).unwrap();
        assert_eq!(plaintext, "https://secret-docker.example.com:2376");
    }
}
