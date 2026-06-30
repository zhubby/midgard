use anyhow::{Context, Result};
use clap::{Args as ClapArgs, Parser, Subcommand};
use midgard_agent::OpenAiCompatibleProvider;
use midgard_config::{
    OperatorControlConfig, default_config_path, ensure_default_config, load_or_create,
};
use midgard_server::{
    AuthSettings, OperatorControlService, OperatorRegistrationToken, OperatorRegistry,
    WorkspaceCredentialSettings,
    app_state_with_provider_auth_orgs_credentials_and_operator_registry, app_with_state,
};
use midgard_storage::{
    AuthStore, NewAuthAuditEvent, NewUser, PostgresAgentSessionStore, UserRole, connect_database,
    hash_password, normalize_email,
};
use std::{
    ffi::OsString,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use toasty_cli::{Config as ToastyConfig, ToastyCli};
use tonic::transport::{Identity, Server as GrpcServer, ServerTlsConfig};

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

    /// Manage Midgard authentication
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },

    /// Run Midgard-native middleware operators
    Operator {
        #[command(subcommand)]
        command: Box<OperatorCommand>,
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

#[derive(Debug, Subcommand)]
enum AuthCommand {
    /// Create the initial administrator account
    SeedAdmin {
        #[arg(long)]
        email: String,

        #[arg(long)]
        password: String,

        #[arg(long)]
        display_name: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum OperatorCommand {
    /// Run the Midgard Valkey Kubernetes operator
    Valkey(ValkeyOperatorArgs),
}

#[derive(Debug, ClapArgs)]
struct ValkeyOperatorArgs {
    #[arg(
        long,
        env = "MIDGARD_OPERATOR_SERVER_ENDPOINT",
        default_value = "https://127.0.0.1:8081"
    )]
    server_endpoint: String,

    #[arg(long, env = "MIDGARD_WORKSPACE_ID")]
    workspace_id: String,

    #[arg(long, env = "MIDGARD_OPERATOR_TOKEN")]
    registration_token: String,

    #[arg(long, env = "MIDGARD_OPERATOR_ID")]
    operator_id: Option<String>,

    #[arg(
        long = "watch-namespace",
        env = "MIDGARD_VALKEY_WATCH_NAMESPACES",
        value_delimiter = ','
    )]
    watch_namespace: Vec<String>,

    #[arg(long, env = "MIDGARD_OPERATOR_TLS_CA_PATH")]
    tls_ca_path: Option<PathBuf>,

    #[arg(long, env = "MIDGARD_OPERATOR_INSECURE", default_value_t = false)]
    allow_insecure_without_tls: bool,

    #[arg(long, env = "MIDGARD_VALKEY_LOCK_NAMESPACE", default_value = midgard_valkey_operator::lease::DEFAULT_LOCK_NAMESPACE)]
    lock_namespace: String,

    #[arg(long, env = "MIDGARD_VALKEY_LOCK_NAME", default_value = midgard_valkey_operator::lease::DEFAULT_LOCK_NAME)]
    lock_name: String,

    #[arg(
        long,
        env = "MIDGARD_VALKEY_LEASE_DURATION_SECONDS",
        default_value_t = 15
    )]
    lease_duration_seconds: u64,

    #[arg(long, env = "MIDGARD_VALKEY_LEASE_RENEW_SECONDS", default_value_t = 5)]
    lease_renew_seconds: u64,

    #[arg(long, env = "MIDGARD_VALKEY_LEASE_RETRY_SECONDS", default_value_t = 5)]
    lease_retry_seconds: u64,

    #[arg(long, env = "MIDGARD_OPERATOR_HEARTBEAT_SECONDS", default_value_t = 10)]
    heartbeat_seconds: u64,

    #[arg(long, env = "MIDGARD_VALKEY_HEALTH_PROBE_BIND_ADDRESS")]
    health_probe_bind_address: Option<String>,

    #[arg(long, env = "MIDGARD_VALKEY_METRICS_BIND_ADDRESS")]
    metrics_bind_address: Option<String>,
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
        Command::Auth { command } => run_auth(cli.config.as_deref(), command).await,
        Command::Operator { command } => match *command {
            OperatorCommand::Valkey(args) => run_valkey_operator(args).await,
        },
    }
}

async fn run_valkey_operator(args: ValkeyOperatorArgs) -> Result<()> {
    init_tracing();
    let config = midgard_valkey_operator::ValkeyOperatorConfig {
        server_endpoint: args.server_endpoint,
        workspace_id: args.workspace_id,
        registration_token: args.registration_token,
        operator_id: args.operator_id,
        watch_namespaces: args.watch_namespace,
        tls_ca_path: args.tls_ca_path,
        allow_insecure_without_tls: args.allow_insecure_without_tls,
        lease: midgard_valkey_operator::lease::LeaseConfig {
            namespace: args.lock_namespace,
            name: args.lock_name,
            lease_duration: Duration::from_secs(args.lease_duration_seconds),
            renew_interval: Duration::from_secs(args.lease_renew_seconds),
            retry_interval: Duration::from_secs(args.lease_retry_seconds),
        },
        heartbeat_interval: Duration::from_secs(args.heartbeat_seconds),
        health_probe_bind_address: args.health_probe_bind_address,
        metrics_bind_address: args.metrics_bind_address,
    };
    midgard_valkey_operator::run(config).await?;
    Ok(())
}

async fn run_server(config_path: Option<&Path>) -> Result<()> {
    init_tracing();
    let loaded = load_or_create(config_path)?;
    let database_url = loaded.config.require_database_url()?;
    loaded.config.operator_control.validate_for_startup()?;
    let address: SocketAddr = loaded
        .config
        .server
        .bind_address
        .parse()
        .with_context(|| format!("invalid server.bind_address in {}", loaded.path.display()))?;

    let store = Arc::new(PostgresAgentSessionStore::connect(database_url).await?);
    let provider = OpenAiCompatibleProvider::new(
        loaded.config.llm_config(),
        loaded.config.llm.api_key.clone(),
    );
    let auth_settings = AuthSettings::new(
        loaded.config.auth.session_ttl_hours,
        loaded.config.auth.cookie_name.clone(),
        loaded.config.auth.cookie_secure,
        loaded.config.auth.cookie_same_site.clone(),
    );
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("bind Midgard server to {address}"))?;

    tracing::info!(%address, config = %loaded.path.display(), "midgard server listening");
    let workspace_credentials = WorkspaceCredentialSettings::new(Some(
        loaded.config.secrets.workspace_credentials_key.clone(),
    ));
    let operator_registry = operator_registry_from_config(&loaded.config.operator_control);
    let app_state = app_state_with_provider_auth_orgs_credentials_and_operator_registry(
        store.clone(),
        store.clone(),
        store,
        Arc::new(provider),
        auth_settings,
        workspace_credentials,
        operator_registry,
    );
    let http_server = axum::serve(listener, app_with_state(app_state.clone()));

    if loaded.config.operator_control.enabled {
        let operator_address: SocketAddr = loaded
            .config
            .operator_control
            .bind_address
            .parse()
            .with_context(|| {
                format!(
                    "invalid operator_control.bind_address in {}",
                    loaded.path.display()
                )
            })?;
        tracing::info!(%operator_address, "midgard operator gRPC listening");
        let operator_server = operator_grpc_server(&loaded.config.operator_control)?
            .add_service(OperatorControlService::new(app_state).into_server())
            .serve(operator_address);

        tokio::select! {
            result = http_server => result.context("serve Midgard API"),
            result = operator_server => result.context("serve Midgard operator gRPC"),
        }
    } else {
        http_server.await.context("serve Midgard API")
    }
}

fn operator_registry_from_config(config: &OperatorControlConfig) -> OperatorRegistry {
    OperatorRegistry::new(
        config
            .registration_tokens
            .iter()
            .map(|token| {
                OperatorRegistrationToken::new(
                    token.workspace_id.trim().to_string(),
                    token.token.trim().to_string(),
                )
            })
            .collect(),
    )
}

fn operator_grpc_server(config: &OperatorControlConfig) -> Result<GrpcServer> {
    let server = GrpcServer::builder();
    if config.allow_insecure_without_tls {
        tracing::warn!(
            "operator gRPC is running without TLS because allow_insecure_without_tls is true"
        );
        return Ok(server);
    }

    let certificate = fs::read(&config.tls_cert_path)
        .with_context(|| format!("read operator TLS certificate {}", config.tls_cert_path))?;
    let private_key = fs::read(&config.tls_key_path)
        .with_context(|| format!("read operator TLS key {}", config.tls_key_path))?;
    Ok(server.tls_config(
        ServerTlsConfig::new().identity(Identity::from_pem(certificate, private_key)),
    )?)
}

async fn run_auth(config_path: Option<&Path>, command: AuthCommand) -> Result<()> {
    let loaded = load_or_create(config_path)?;
    let database_url = loaded.config.require_database_url()?;
    let store = PostgresAgentSessionStore::connect(database_url).await?;

    match command {
        AuthCommand::SeedAdmin {
            email,
            password,
            display_name,
        } => {
            let email_lower = normalize_email(&email);
            if store.load_user_by_email(&email_lower).await?.is_some() {
                println!("admin user already exists for {email_lower}");
                return Ok(());
            }

            let user = store
                .create_user(NewUser {
                    email: email_lower.clone(),
                    display_name: display_name
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| email_lower.clone()),
                    role: UserRole::Admin,
                    system_role_id: None,
                    password_hash: hash_password(&password)?,
                    active: true,
                })
                .await?;
            store
                .record_auth_audit_event(NewAuthAuditEvent {
                    user_id: Some(user.id),
                    event_type: "seed_admin_created".to_string(),
                    email_lower: Some(user.email.clone()),
                    occurred_at: chrono::Utc::now().to_rfc3339(),
                    ip_address: None,
                    user_agent: None,
                    detail_json: Some(r#"{"actor":"midgard-cli"}"#.to_string()),
                })
                .await?;

            println!("created admin user {}", user.email);
            Ok(())
        }
    }
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
    use midgard_config::{MidgardConfig, ensure_default_config};
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

    #[test]
    fn operator_valkey_command_parses_startup_arguments() {
        let cli = Cli::try_parse_from([
            "midgard",
            "operator",
            "valkey",
            "--server-endpoint",
            "http://127.0.0.1:8081",
            "--workspace-id",
            "11111111-1111-1111-1111-111111111111",
            "--registration-token",
            "secret",
            "--allow-insecure-without-tls",
            "--watch-namespace",
            "data,cache",
            "--health-probe-bind-address",
            ":8081",
        ])
        .unwrap();

        let Some(Command::Operator { command }) = cli.command else {
            panic!("expected valkey operator command");
        };
        let OperatorCommand::Valkey(args) = *command;
        assert_eq!(args.server_endpoint, "http://127.0.0.1:8081");
        assert_eq!(args.workspace_id, "11111111-1111-1111-1111-111111111111");
        assert_eq!(args.registration_token, "secret");
        assert!(args.allow_insecure_without_tls);
        assert_eq!(args.watch_namespace, vec!["data", "cache"]);
        assert_eq!(args.health_probe_bind_address.as_deref(), Some(":8081"));
    }

    #[test]
    fn old_manager_startup_command_is_not_available() {
        let err = Cli::try_parse_from(["midgard", "manager"]).unwrap_err();

        assert!(err.to_string().contains("unrecognized subcommand"));
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

    #[tokio::test]
    async fn auth_seed_admin_fails_when_database_url_is_empty() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        ensure_default_config(&config_path, true).unwrap();

        let err = run_from([
            "midgard",
            "--config",
            config_path.to_str().unwrap(),
            "auth",
            "seed-admin",
            "--email",
            "admin@example.com",
            "--password",
            "valid-password",
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
