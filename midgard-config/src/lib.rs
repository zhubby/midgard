use midgard_core::{LlmApiMode, LlmConfig, MidgardError, MidgardResult};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const CONFIG_DIR: &str = ".midgard";
const CONFIG_FILE: &str = "config.toml";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MidgardConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub llm: LlmFileConfig,
}

impl MidgardConfig {
    pub fn default_for_new_file() -> Self {
        Self {
            server: ServerConfig {
                bind_address: "0.0.0.0:8080".to_string(),
            },
            database: DatabaseConfig { url: String::new() },
            llm: LlmFileConfig {
                base_url: "https://api.openai.com/v1".to_string(),
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
pub struct DatabaseConfig {
    pub url: String,
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
    let config = load_config(&path)?;

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

    let contents =
        toml::to_string_pretty(&MidgardConfig::default_for_new_file()).map_err(|err| {
            MidgardError::Configuration(format!("failed to serialize default config: {err}"))
        })?;

    fs::write(path, contents).map_err(|err| {
        MidgardError::Configuration(format!(
            "failed to write config file {}: {err}",
            path.display()
        ))
    })?;

    Ok(true)
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
        assert_eq!(loaded.config.server.bind_address, "0.0.0.0:8080");
        assert!(loaded.path.exists());
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

        assert_eq!(config.llm_config().base_url, "https://api.openai.com/v1");
        assert_eq!(config.llm_config().model, "gpt-4o-mini");
        assert_eq!(config.llm_config().api_mode, LlmApiMode::ChatCompletions);
    }
}
