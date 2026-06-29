use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use midgard_config::{default_config_path, ensure_default_config, load_or_create};
use midgard_server::app_with_storage;
use midgard_storage::{connect_database, PostgresAgentSessionStore};
use std::{
    ffi::OsString,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use toasty_cli::{Config as ToastyConfig, ToastyCli};

const STORAGE_TOASTY_CONFIG: &str = "midgard-storage/Toasty.toml";

#[derive(Debug, Parser)]
#[command(name = "midgard")]
#[command(about = "Midgard operations platform")]
#[command(version)]
pub struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[arg(long, global = true)]
    project_root: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the Midgard HTTP API server
    Server,

    /// Manage Midgard configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },

    /// Manage Toasty database migrations
    Migrate {
        #[command(subcommand)]
        command: MigrateCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Create the default configuration file
    Init {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
enum MigrateCommand {
    /// Apply pending migrations
    Apply,

    /// Generate a migration from the current Toasty schema
    Generate {
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Drop a migration from Toasty history
    Drop {
        #[arg(short, long)]
        name: Option<String>,

        #[arg(short, long)]
        latest: bool,
    },

    /// Reset the database and optionally re-apply migrations
    Reset {
        #[arg(long)]
        skip_migrations: bool,
    },

    /// Print the current Toasty schema snapshot
    Snapshot,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    run_cli(cli).await
}

pub async fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;
    run_cli(cli).await
}

async fn run_cli(cli: Cli) -> Result<()> {
    match cli.command.unwrap_or(Command::Server) {
        Command::Server => run_server(cli.config.as_deref()).await,
        Command::Config {
            command: ConfigCommand::Init { force },
        } => {
            let path = match cli.config {
                Some(path) => path,
                None => default_config_path()?,
            };
            let created = ensure_default_config(&path, force)?;
            if created {
                println!("created Midgard config at {}", path.display());
            } else {
                println!("Midgard config already exists at {}", path.display());
            }
            Ok(())
        }
        Command::Migrate { command } => {
            run_migration(cli.config.as_deref(), cli.project_root.as_deref(), command).await
        }
    }
}

async fn run_server(config_path: Option<&Path>) -> Result<()> {
    init_tracing();
    let loaded = load_or_create(config_path)?;
    let database_url = loaded.config.require_database_url()?;
    let address: SocketAddr = loaded
        .config
        .server
        .bind_address
        .parse()
        .with_context(|| format!("invalid server.bind_address in {}", loaded.path.display()))?;

    let store = PostgresAgentSessionStore::connect(database_url).await?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("bind Midgard server to {address}"))?;

    tracing::info!(%address, config = %loaded.path.display(), "midgard server listening");
    axum::serve(listener, app_with_storage(Arc::new(store)))
        .await
        .context("serve Midgard API")
}

async fn run_migration(
    config_path: Option<&Path>,
    project_root: Option<&Path>,
    command: MigrateCommand,
) -> Result<()> {
    let loaded = load_or_create(config_path)?;
    let database_url = loaded.config.require_database_url()?;
    let project_root = absolute_project_root(project_root)?;
    let config = load_toasty_config(&project_root)?;
    let db = connect_database(database_url).await?;
    let cli = ToastyCli::with_config(db, config);

    cli.parse_from(toasty_args(command)).await
}

fn absolute_project_root(project_root: Option<&Path>) -> Result<PathBuf> {
    let path = match project_root {
        Some(path) => path.to_path_buf(),
        None => return std::env::current_dir().context("determine current directory"),
    };

    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()
            .context("determine current directory")?
            .join(path))
    }
}

fn load_toasty_config(project_root: &Path) -> Result<ToastyConfig> {
    let config_path = project_root.join(STORAGE_TOASTY_CONFIG);
    let config_dir = config_path
        .parent()
        .context("determine Toasty config directory")?;
    let mut config = ToastyConfig::load_from(&config_path)
        .with_context(|| format!("load Toasty.toml from {}", config_path.display()))?;

    if config.migration.path.is_relative() {
        config.migration.path = config_dir.join(&config.migration.path);
    }

    Ok(config)
}

fn toasty_args(command: MigrateCommand) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("toasty"),
        OsString::from("migration"),
        OsString::from(match &command {
            MigrateCommand::Apply => "apply",
            MigrateCommand::Generate { .. } => "generate",
            MigrateCommand::Drop { .. } => "drop",
            MigrateCommand::Reset { .. } => "reset",
            MigrateCommand::Snapshot => "snapshot",
        }),
    ];

    match command {
        MigrateCommand::Generate { name: Some(name) } => {
            args.push(OsString::from("--name"));
            args.push(OsString::from(name));
        }
        MigrateCommand::Drop {
            name: Some(name),
            latest,
        } => {
            args.push(OsString::from("--name"));
            args.push(OsString::from(name));
            if latest {
                args.push(OsString::from("--latest"));
            }
        }
        MigrateCommand::Drop { name: None, latest } => {
            if latest {
                args.push(OsString::from("--latest"));
            }
        }
        MigrateCommand::Reset { skip_migrations } => {
            if skip_migrations {
                args.push(OsString::from("--skip-migrations"));
            }
        }
        MigrateCommand::Apply
        | MigrateCommand::Generate { name: None }
        | MigrateCommand::Snapshot => {}
    }

    args
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use midgard_config::{ensure_default_config, MidgardConfig};
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn config_init_creates_default_config_at_explicit_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        run_from([
            "midgard",
            "--config",
            path.to_str().unwrap(),
            "config",
            "init",
        ])
        .await
        .unwrap();

        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.contains("[database]"));
        assert!(contents.contains("url = \"\""));
    }

    #[tokio::test]
    async fn config_init_force_rewrites_existing_config() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "custom = true\n").unwrap();

        run_from([
            "midgard",
            "--config",
            path.to_str().unwrap(),
            "config",
            "init",
            "--force",
        ])
        .await
        .unwrap();

        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.contains("[server]"));
        assert!(!contents.contains("custom = true"));
    }

    #[tokio::test]
    async fn server_fails_before_binding_when_database_url_is_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        ensure_default_config(&path, true).unwrap();

        let err = run_from(["midgard", "--config", path.to_str().unwrap(), "server"])
            .await
            .unwrap_err();

        assert!(err.to_string().contains("database.url is empty"));
    }

    #[tokio::test]
    async fn migrate_apply_fails_when_database_url_is_empty() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        ensure_default_config(&config_path, true).unwrap();

        let err = run_from([
            "midgard",
            "--config",
            config_path.to_str().unwrap(),
            "migrate",
            "apply",
            "--project-root",
            dir.path().to_str().unwrap(),
        ])
        .await
        .unwrap_err();

        assert!(err.to_string().contains("database.url is empty"));
    }

    #[test]
    fn migrate_generate_translates_to_toasty_args() {
        let args = toasty_args(MigrateCommand::Generate {
            name: Some("init".to_string()),
        });

        assert_eq!(
            args,
            vec![
                OsString::from("toasty"),
                OsString::from("migration"),
                OsString::from("generate"),
                OsString::from("--name"),
                OsString::from("init"),
            ]
        );
    }

    #[test]
    fn toasty_config_migration_path_is_resolved_from_project_root() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("midgard-storage")).unwrap();
        fs::write(
            dir.path().join("midgard-storage/Toasty.toml"),
            r#"[migration]
path = "toasty"
prefix_style = "Sequential"
checksums = false
statement_breakpoints = true
"#,
        )
        .unwrap();

        let config = load_toasty_config(dir.path()).unwrap();

        assert_eq!(
            config.migration.path,
            dir.path().join("midgard-storage/toasty")
        );
    }

    #[tokio::test]
    async fn migrate_apply_can_run_against_test_database_url_when_available() {
        let Ok(database_url) = std::env::var("MIDGARD_TEST_DATABASE_URL") else {
            return;
        };
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let mut config = MidgardConfig::default_for_new_file();
        config.database.url = database_url;
        fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();
        fs::create_dir_all(dir.path().join("midgard-storage")).unwrap();
        fs::write(
            dir.path().join("midgard-storage/Toasty.toml"),
            r#"[migration]
path = "toasty"
prefix_style = "Sequential"
checksums = false
statement_breakpoints = true
"#,
        )
        .unwrap();

        run_from([
            "midgard",
            "--config",
            config_path.to_str().unwrap(),
            "migrate",
            "apply",
            "--project-root",
            dir.path().to_str().unwrap(),
        ])
        .await
        .unwrap();
    }
}
