use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use midgard_core::{LlmApiMode, LlmConfig, MidgardError, MidgardResult};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const CONFIG_DIR: &str = ".midgard";
const CONFIG_FILE: &str = "config.toml";
const WORKSPACE_CREDENTIAL_KEY_BYTES: usize = 32;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MidgardConfig {
    pub server: ServerConfig,
    #[serde(default)]
    pub operator_control: OperatorControlConfig,
    pub database: DatabaseConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub secrets: SecretsConfig,
    pub llm: LlmFileConfig,
}

impl MidgardConfig {
    pub fn default_for_new_file() -> Self {
        Self {
            server: ServerConfig {
                bind_address: "0.0.0.0:8080".to_string(),
            },
            operator_control: OperatorControlConfig::default(),
            database: DatabaseConfig { url: String::new() },
            auth: AuthConfig::default(),
            secrets: SecretsConfig::generated(),
            llm: LlmFileConfig {
                base_url: "https://api.openai.com/v1/chat/completions".to_string(),
                model: "gpt-4o-mini".to_string(),
                api_mode: LlmApiMode::default(),
                api_key: String::new(),
            },
        }
    }

    pub fn llm_config(&self) -> LlmConfig {
        LlmConfig::new(self.llm.base_url.clone(), self.llm.model.clone())
            .with_api_mode(self.llm.api_mode.clone())
    }

    pub fn require_database_url(&self) -> MidgardResult<&str> {
        let url = self.database.url.trim();
        if url.is_empty() {
            return Err(MidgardError::Configuration(
                "database.url is empty; edit the Midgard config file before starting the server or running migrations".to_string(),
            ));
        }

        Ok(url)
    }
}

impl Default for MidgardConfig {
    fn default() -> Self {
        Self::default_for_new_file()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServerConfig {
    pub bind_address: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperatorControlConfig {
    pub enabled: bool,
    pub bind_address: String,
    pub tls_cert_path: String,
    pub tls_key_path: String,
    pub allow_insecure_without_tls: bool,
}

impl Default for OperatorControlConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_address: "0.0.0.0:8081".to_string(),
            tls_cert_path: String::new(),
            tls_key_path: String::new(),
            allow_insecure_without_tls: false,
        }
    }
}

impl OperatorControlConfig {
    pub fn validate_for_startup(&self) -> MidgardResult<()> {
        if !self.enabled {
            return Ok(());
        }
        if !self.allow_insecure_without_tls
            && (self.tls_cert_path.trim().is_empty() || self.tls_key_path.trim().is_empty())
        {
            return Err(MidgardError::Configuration(
                "operator_control.tls_cert_path and tls_key_path are required unless allow_insecure_without_tls is true".to_string(),
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthConfig {
    pub session_ttl_hours: u64,
    pub cookie_name: String,
    pub cookie_secure: bool,
    pub cookie_same_site: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            session_ttl_hours: 12,
            cookie_name: "midgard_session".to_string(),
            cookie_secure: false,
            cookie_same_site: "lax".to_string(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SecretsConfig {
    pub workspace_credentials_key: String,
}

impl SecretsConfig {
    pub fn generated() -> Self {
        let mut key = [0_u8; WORKSPACE_CREDENTIAL_KEY_BYTES];
        OsRng.fill_bytes(&mut key);
        Self {
            workspace_credentials_key: URL_SAFE_NO_PAD.encode(key),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LlmFileConfig {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_mode: LlmApiMode,
    pub api_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub config: MidgardConfig,
    pub created: bool,
}

pub fn default_config_path() -> MidgardResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| {
        MidgardError::Configuration("could not determine the user home directory".to_string())
    })?;

    Ok(default_config_path_from_home(home))
}

pub fn default_config_path_from_home(home: impl Into<PathBuf>) -> PathBuf {
    home.into().join(CONFIG_DIR).join(CONFIG_FILE)
}

pub fn load_or_create(path: Option<&Path>) -> MidgardResult<LoadedConfig> {
    let path = match path {
        Some(path) => path.to_path_buf(),
        None => default_config_path()?,
    };

    let created = ensure_default_config(&path, false)?;
    let mut config = load_config(&path)?;
    if config.secrets.workspace_credentials_key.trim().is_empty() {
        config.secrets = SecretsConfig::generated();
        write_config(&path, &config)?;
    }

    Ok(LoadedConfig {
        path,
        config,
        created,
    })
}

pub fn ensure_default_config(path: &Path, force: bool) -> MidgardResult<bool> {
    if path.exists() && !force {
        return Ok(false);
    }

    if let Some(parent) = non_empty_parent(path) {
        fs::create_dir_all(parent).map_err(|err| {
            MidgardError::Configuration(format!(
                "failed to create config directory {}: {err}",
                parent.display()
            ))
        })?;
    }

    write_config(path, &MidgardConfig::default_for_new_file())?;

    Ok(true)
}

fn write_config(path: &Path, config: &MidgardConfig) -> MidgardResult<()> {
    let contents = toml::to_string_pretty(config)
        .map_err(|err| MidgardError::Configuration(format!("failed to serialize config: {err}")))?;

    fs::write(path, contents).map_err(|err| {
        MidgardError::Configuration(format!(
            "failed to write config file {}: {err}",
            path.display()
        ))
    })
}

fn non_empty_parent(path: &Path) -> Option<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
}

pub fn load_config(path: &Path) -> MidgardResult<MidgardConfig> {
    let contents = fs::read_to_string(path).map_err(|err| {
        MidgardError::Configuration(format!(
            "failed to read config file {}: {err}",
            path.display()
        ))
    })?;

    toml::from_str(&contents).map_err(|err| {
        MidgardError::Configuration(format!(
            "failed to parse config file {}: {err}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn default_path_uses_midgard_config_under_home() {
        let path = default_config_path_from_home("/tmp/example-home");

        assert_eq!(
            path,
            PathBuf::from("/tmp/example-home/.midgard/config.toml")
        );
    }

    #[test]
    fn load_or_create_writes_default_config_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".midgard/config.toml");

        let loaded = load_or_create(Some(&path)).unwrap();

        assert!(loaded.created);
        assert_eq!(loaded.path, path);
        assert_eq!(loaded.config.database.url, "");
        assert_eq!(loaded.config.auth.cookie_name, "midgard_session");
        assert!(
            loaded.config.secrets.workspace_credentials_key.len() >= 32,
            "new config should include a generated workspace credential key"
        );
        assert_eq!(loaded.config.server.bind_address, "0.0.0.0:8080");
        assert!(!loaded.config.operator_control.enabled);
        assert_eq!(loaded.config.operator_control.bind_address, "0.0.0.0:8081");
        assert!(loaded.path.exists());
    }

    #[test]
    fn load_or_create_backfills_missing_workspace_credential_key() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".midgard/config.toml");
        let mut config = MidgardConfig::default_for_new_file();
        config.secrets.workspace_credentials_key.clear();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, toml::to_string_pretty(&config).unwrap()).unwrap();

        let loaded = load_or_create(Some(&path)).unwrap();
        let persisted = load_config(&path).unwrap();

        assert!(!loaded.created);
        assert!(!loaded.config.secrets.workspace_credentials_key.is_empty());
        assert_eq!(
            loaded.config.secrets.workspace_credentials_key,
            persisted.secrets.workspace_credentials_key
        );
    }

    #[test]
    fn ensure_default_config_does_not_overwrite_existing_file_without_force() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "custom = true\n").unwrap();

        let created = ensure_default_config(&path, false).unwrap();

        assert!(!created);
        assert_eq!(fs::read_to_string(path).unwrap(), "custom = true\n");
    }

    #[test]
    fn ensure_default_config_overwrites_existing_file_with_force() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "custom = true\n").unwrap();

        let created = ensure_default_config(&path, true).unwrap();

        assert!(created);
        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.contains("[database]"));
        assert!(contents.contains("[auth]"));
        assert!(contents.contains("url = \"\""));
    }

    #[test]
    fn current_directory_relative_file_has_no_parent_to_create() {
        assert_eq!(non_empty_parent(Path::new("config.toml")), None);
    }

    #[test]
    fn empty_database_url_returns_configuration_error() {
        let config = MidgardConfig::default_for_new_file();

        let err = config.require_database_url().unwrap_err();

        assert!(matches!(err, MidgardError::Configuration(_)));
        assert!(err.to_string().contains("database.url is empty"));
    }

    #[test]
    fn invalid_toml_error_mentions_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[server\n").unwrap();

        let err = load_config(&path).unwrap_err();

        assert!(matches!(err, MidgardError::Configuration(_)));
        assert!(err.to_string().contains(path.to_string_lossy().as_ref()));
    }

    #[test]
    fn llm_file_config_maps_to_core_llm_config() {
        let config = MidgardConfig::default_for_new_file();

        assert_eq!(
            config.llm_config().base_url,
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(config.llm_config().model, "gpt-4o-mini");
        assert_eq!(config.llm_config().api_mode, LlmApiMode::ChatCompletions);
    }

    #[test]
    fn enabled_operator_control_requires_tls_by_default() {
        let mut config = OperatorControlConfig {
            enabled: true,
            ..OperatorControlConfig::default()
        };

        assert!(config.validate_for_startup().is_err());

        config.allow_insecure_without_tls = true;
        assert!(config.validate_for_startup().is_ok());
    }
}
