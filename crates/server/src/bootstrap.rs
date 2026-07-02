use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use config::{Config, Environment, File, FileFormat};
use tower_http::catch_panic::CatchPanicLayer;

use oidc_exchange_core::config::{AppConfig, ProviderConfig};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{
    AuditLog, IdentityProvider, KeyManager, SessionRepository, UserRepository, UserSync,
};
use oidc_exchange_core::service::AppService;

use crate::middleware::audit_context::audit_context_layer;
use crate::middleware::error_handler::panic_handler;
use crate::middleware::request_id::request_id_layer;
use crate::routes;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Config loading
// ---------------------------------------------------------------------------

/// Name of the config directory relative to the process working directory.
const CONFIG_DIR: &str = "config";

/// Environment variable that selects the environment-specific overlay TOML
/// (`config/{OIDC_EXCHANGE_ENV}.toml`), e.g. `production`, `sqlite-only`.
const ENV_SELECTOR_VAR: &str = "OIDC_EXCHANGE_ENV";

/// Prefix required on environment variables that structurally override the
/// merged config (`OIDC_EXCHANGE__{section}__{key}`).
const ENV_OVERRIDE_PREFIX: &str = "OIDC_EXCHANGE";

/// Separator between path segments in `OIDC_EXCHANGE__{section}__{key}`
/// environment overrides. A double underscore separates segments; a single
/// underscore stays inside a segment (`OIDC_EXCHANGE__PROVIDERS__MY_IDP__…`
/// addresses `providers.my_idp`).
const ENV_OVERRIDE_SEPARATOR: &str = "__";

/// Load configuration from config files on disk, using the `OIDC_EXCHANGE_ENV`
/// environment variable to select the environment-specific config file, and
/// `OIDC_EXCHANGE__{section}__{key}` environment variables to override the
/// merged result afterward.
pub fn load_config() -> Result<AppConfig, Box<dyn std::error::Error>> {
    load_config_from_dir(CONFIG_DIR)
}

/// Core of [`load_config`], parameterized over the config directory so tests
/// can point it at a fixture directory instead of the process's `config/`.
///
/// Loading order:
/// 1. `{config_dir}/default.toml` — compiled-in defaults (missing file is not
///    an error).
/// 2. `{config_dir}/{OIDC_EXCHANGE_ENV}.toml` — deep-merged on top when
///    `OIDC_EXCHANGE_ENV` is set and non-empty (tables merge recursively,
///    scalars/arrays are replaced; missing/empty file is not an error).
/// 3. `OIDC_EXCHANGE__{section}__{key}` environment variables, applied on top
///    of the merged TOML and reaching every path including map-valued
///    sections.
///
/// With no files present and no overriding environment variables, the
/// deserialized result is `AppConfig::default()` (every field carries
/// `#[serde(default)]`).
fn load_config_from_dir(config_dir: &str) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let mut builder = Config::builder().add_source(
        File::with_name(&format!("{config_dir}/default"))
            .format(FileFormat::Toml)
            .required(false),
    );

    if let Ok(env) = std::env::var(ENV_SELECTOR_VAR) {
        if !env.is_empty() {
            builder = builder.add_source(
                File::with_name(&format!("{config_dir}/{env}"))
                    .format(FileFormat::Toml)
                    .required(false),
            );
        }
    }

    builder = builder.add_source(
        Environment::with_prefix(ENV_OVERRIDE_PREFIX)
            .separator(ENV_OVERRIDE_SEPARATOR)
            .try_parsing(true),
    );

    let merged = builder.build()?;
    let config: AppConfig = merged.try_deserialize()?;
    Ok(config)
}

/// Parse a TOML string directly into an `AppConfig`.
pub fn parse_config(toml_str: &str) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let config: AppConfig = toml::from_str(toml_str)?;
    Ok(config)
}

// ---------------------------------------------------------------------------
// Service builder
// ---------------------------------------------------------------------------

/// Build the full `AppService` from a loaded config, instantiating all
/// adapters (repositories, key manager, audit log, user sync, providers)
/// according to the configured role.
pub async fn build_service(config: &AppConfig) -> Result<AppService, Box<dyn std::error::Error>> {
    let role = config.server.role.as_str();

    // Build adapters (skip unused ones based on role)
    let user_repo = build_user_repository(config).await?;
    let session_repo = build_session_repository(config).await?;

    // Key manager and providers only needed for exchange role
    let keys: Box<dyn KeyManager> = if role == "admin" {
        Box::new(oidc_exchange_adapters::noop::NoopKeyManager)
    } else {
        build_key_manager(config)?
    };

    let audit = build_audit_log(config).await?;

    // User sync only needed for admin role
    let user_sync: Box<dyn UserSync> = if role == "exchange" {
        Box::new(oidc_exchange_adapters::noop::NoopUserSync::new())
    } else {
        build_user_sync(config)?
    };

    // Providers only needed for exchange role
    let providers = if role == "admin" {
        HashMap::new()
    } else {
        build_providers(config).await?
    };

    Ok(AppService::new(
        user_repo,
        session_repo,
        keys,
        audit,
        user_sync,
        providers,
        config.clone(),
    ))
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

/// Build the Axum `Router` from a config and service, applying role-based
/// route merging and middleware layers.
pub fn build_router(config: &AppConfig, service: AppService) -> Router {
    let role = config.server.role.as_str();

    let state = AppState {
        service: Arc::new(service),
        config: Arc::new(config.clone()),
    };

    let mut app: Router<AppState> = Router::new();

    if role == "exchange" || role == "all" {
        app = app.merge(routes::public_routes());
    }
    if role == "admin" || role == "all" {
        app = app.merge(routes::internal_routes(state.clone()));
        // Ensure /health is available even in admin-only mode
        // (only add if not already present from public_routes)
        if role == "admin" {
            app = app.route(
                "/health",
                axum::routing::get(routes::health::health_handler),
            );
        }
    }

    app.layer(axum::middleware::from_fn(request_id_layer))
        .layer(axum::middleware::from_fn(audit_context_layer))
        .layer(CatchPanicLayer::custom(panic_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Adapter builders (private)
// ---------------------------------------------------------------------------

async fn build_dynamo_client(
    config: &AppConfig,
) -> Result<(aws_sdk_dynamodb::Client, String), Box<dyn std::error::Error>> {
    let dynamo_cfg = config
        .repository
        .dynamodb
        .as_ref()
        .ok_or_else(|| Error::ConfigError {
            detail: "repository.adapter is 'dynamodb' but [repository.dynamodb] section is missing"
                .into(),
        })?;

    let mut aws_loader = aws_config::defaults(aws_config::BehaviorVersion::latest());

    if let Some(ref region) = dynamo_cfg.region {
        aws_loader = aws_loader.region(aws_config::Region::new(region.clone()));
    }

    let sdk_config = aws_loader.load().await;
    let client = aws_sdk_dynamodb::Client::new(&sdk_config);
    Ok((client, dynamo_cfg.table_name.clone()))
}

async fn build_user_repository(
    config: &AppConfig,
) -> Result<Box<dyn UserRepository>, Box<dyn std::error::Error>> {
    match config.repository.adapter.as_str() {
        "dynamodb" => {
            let (client, table_name) = build_dynamo_client(config).await?;
            Ok(Box::new(
                oidc_exchange_adapters::dynamo::DynamoRepository::new(client, table_name),
            ))
        }
        "postgres" => {
            let pg_cfg = config.repository.postgres.as_ref().ok_or_else(|| {
                Error::ConfigError {
                    detail:
                        "repository.adapter is 'postgres' but [repository.postgres] section is missing"
                            .into(),
                }
            })?;
            let pool = oidc_exchange_adapters::postgres::create_pool(
                &pg_cfg.url,
                pg_cfg.max_connections.unwrap_or(5),
            )
            .await?;
            Ok(Box::new(
                oidc_exchange_adapters::postgres::PostgresRepository::new(pool),
            ))
        }
        "sqlite" => {
            let sq_cfg = config
                .repository
                .sqlite
                .as_ref()
                .ok_or_else(|| Error::ConfigError {
                    detail:
                        "repository.adapter is 'sqlite' but [repository.sqlite] section is missing"
                            .into(),
                })?;
            let pool = oidc_exchange_adapters::sqlite::create_pool(&sq_cfg.path).await?;
            Ok(Box::new(
                oidc_exchange_adapters::sqlite::SqliteRepository::new(pool),
            ))
        }
        "" => Err(Box::new(Error::ConfigError {
            detail: "repository.adapter is not configured".into(),
        })),
        other => Err(Box::new(Error::ConfigError {
            detail: format!("unknown repository adapter: {other}"),
        })),
    }
}

async fn build_session_repository(
    config: &AppConfig,
) -> Result<Box<dyn SessionRepository>, Box<dyn std::error::Error>> {
    // If a separate session_repository adapter is configured, use it.
    // Otherwise, fall back to the same adapter as the user repository.
    let adapter = config
        .session_repository
        .adapter
        .as_deref()
        .unwrap_or(config.repository.adapter.as_str());

    match adapter {
        "dynamodb" => {
            let (client, table_name) = build_dynamo_client(config).await?;
            Ok(Box::new(
                oidc_exchange_adapters::dynamo::DynamoRepository::new(client, table_name),
            ))
        }
        "postgres" => {
            let pg_cfg = config.repository.postgres.as_ref().ok_or_else(|| {
                Error::ConfigError {
                    detail:
                        "session_repository adapter is 'postgres' but [repository.postgres] section is missing"
                            .into(),
                }
            })?;
            let pool = oidc_exchange_adapters::postgres::create_pool(
                &pg_cfg.url,
                pg_cfg.max_connections.unwrap_or(5),
            )
            .await?;
            Ok(Box::new(
                oidc_exchange_adapters::postgres::PostgresRepository::new(pool),
            ))
        }
        "sqlite" => {
            let sq_cfg = config.repository.sqlite.as_ref().ok_or_else(|| {
                Error::ConfigError {
                    detail:
                        "session_repository adapter is 'sqlite' but [repository.sqlite] section is missing"
                            .into(),
                }
            })?;
            let pool = oidc_exchange_adapters::sqlite::create_pool(&sq_cfg.path).await?;
            Ok(Box::new(
                oidc_exchange_adapters::sqlite::SqliteRepository::new(pool),
            ))
        }
        "valkey" => {
            let vk_cfg = config.session_repository.valkey.as_ref().ok_or_else(|| {
                Error::ConfigError {
                    detail:
                        "session_repository adapter is 'valkey' but [session_repository.valkey] section is missing"
                            .into(),
                }
            })?;
            let client = oidc_exchange_adapters::valkey::ValkeySessionRepository::new(
                &vk_cfg.url,
                vk_cfg
                    .key_prefix
                    .clone()
                    .unwrap_or_else(|| "oidc:".to_string()),
            )
            .await?;
            Ok(Box::new(client))
        }
        "lmdb" => {
            let lm_cfg = config.session_repository.lmdb.as_ref().ok_or_else(|| {
                Error::ConfigError {
                    detail:
                        "session_repository adapter is 'lmdb' but [session_repository.lmdb] section is missing"
                            .into(),
                }
            })?;
            let repo = oidc_exchange_adapters::lmdb::LmdbSessionRepository::new(
                &lm_cfg.path,
                lm_cfg.max_size_mb.unwrap_or(256),
            )?;
            Ok(Box::new(repo))
        }
        "" => Err(Box::new(Error::ConfigError {
            detail: "repository.adapter is not configured".into(),
        })),
        other => Err(Box::new(Error::ConfigError {
            detail: format!("unknown session_repository adapter: {other}"),
        })),
    }
}

fn build_key_manager(
    config: &AppConfig,
) -> Result<Box<dyn KeyManager>, Box<dyn std::error::Error>> {
    match config.key_manager.adapter.as_str() {
        "local" => {
            let local_cfg =
                config
                    .key_manager
                    .local
                    .as_ref()
                    .ok_or_else(|| {
                        Error::ConfigError {
                    detail:
                        "key_manager.adapter is 'local' but [key_manager.local] section is missing"
                            .into(),
                }
                    })?;

            let mgr = oidc_exchange_adapters::local_keys::LocalKeyManager::from_file(
                &local_cfg.private_key_path,
                &local_cfg.algorithm,
                &local_cfg.kid,
            )?;
            Ok(Box::new(mgr))
        }
        "kms" => {
            let kms_cfg = config
                .key_manager
                .kms
                .as_ref()
                .ok_or_else(|| Error::ConfigError {
                    detail: "key_manager.adapter is 'kms' but [key_manager.kms] section is missing"
                        .into(),
                })?;

            // Build KMS client synchronously using a blocking load.
            let sdk_config = futures::executor::block_on(
                aws_config::defaults(aws_config::BehaviorVersion::latest()).load(),
            );
            let client = aws_sdk_kms::Client::new(&sdk_config);

            Ok(Box::new(oidc_exchange_adapters::kms::KmsKeyManager::new(
                client,
                kms_cfg.key_id.clone(),
                kms_cfg.algorithm.clone(),
                kms_cfg.kid.clone(),
            )))
        }
        "" => Err(Box::new(Error::ConfigError {
            detail: "key_manager.adapter is not configured".into(),
        })),
        other => Err(Box::new(Error::ConfigError {
            detail: format!("unknown key_manager adapter: {other}"),
        })),
    }
}

async fn build_audit_log(
    config: &AppConfig,
) -> Result<Box<dyn AuditLog>, Box<dyn std::error::Error>> {
    match config.audit.adapter.as_str() {
        "noop" | "" => Ok(Box::new(oidc_exchange_adapters::noop::NoopAuditLog::new())),
        "stdout" | "stderr" | "auto" => {
            use oidc_exchange_adapters::stdout_audit::{OutputTarget, StdoutAuditLog};
            let target = match config.audit.adapter.as_str() {
                "stdout" => OutputTarget::Stdout,
                "stderr" => OutputTarget::Stderr,
                _ => OutputTarget::Auto,
            };
            Ok(Box::new(StdoutAuditLog::new(target)))
        }
        "sqs" => {
            let sqs_cfg = config
                .audit
                .sqs
                .as_ref()
                .ok_or_else(|| Error::ConfigError {
                    detail: "audit.adapter is 'sqs' but [audit.sqs] section is missing".into(),
                })?;

            let mut aws_loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
            if let Some(ref region) = sqs_cfg.region {
                aws_loader = aws_loader.region(aws_config::Region::new(region.clone()));
            }
            let sdk_config = aws_loader.load().await;
            let client = aws_sdk_sqs::Client::new(&sdk_config);

            Ok(Box::new(
                oidc_exchange_adapters::sqs_audit::SqsAuditLog::new(
                    client,
                    sqs_cfg.queue_url.clone(),
                ),
            ))
        }
        other => Err(Box::new(Error::ConfigError {
            detail: format!("unknown audit adapter: {other}"),
        })),
    }
}

fn build_user_sync(config: &AppConfig) -> Result<Box<dyn UserSync>, Box<dyn std::error::Error>> {
    if !config.user_sync.enabled {
        return Ok(Box::new(oidc_exchange_adapters::noop::NoopUserSync::new()));
    }

    match config.user_sync.adapter.as_deref() {
        Some("webhook") => {
            let wh_cfg = config
                .user_sync
                .webhook
                .as_ref()
                .ok_or_else(|| Error::ConfigError {
                    detail:
                        "user_sync.adapter is 'webhook' but [user_sync.webhook] section is missing"
                            .into(),
                })?;

            let timeout_secs = wh_cfg
                .timeout
                .as_deref()
                .and_then(|s| {
                    let s = s.trim();
                    if let Some(stripped) = s.strip_suffix('s') {
                        stripped.parse::<u64>().ok()
                    } else {
                        s.parse::<u64>().ok()
                    }
                })
                .unwrap_or(5);
            let retries = wh_cfg.retries.unwrap_or(2);

            Ok(Box::new(
                oidc_exchange_adapters::webhook::WebhookUserSync::new(
                    wh_cfg.url.clone(),
                    wh_cfg.secret.clone(),
                    std::time::Duration::from_secs(timeout_secs),
                    retries,
                ),
            ))
        }
        Some(other) => Err(Box::new(Error::ConfigError {
            detail: format!("unknown user_sync adapter: {other}"),
        })),
        None => {
            // enabled=true but no adapter specified — default to noop
            Ok(Box::new(oidc_exchange_adapters::noop::NoopUserSync::new()))
        }
    }
}

async fn build_providers(
    config: &AppConfig,
) -> Result<HashMap<String, Box<dyn IdentityProvider>>, Box<dyn std::error::Error>> {
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();

    for (name, provider_cfg) in &config.providers {
        let provider = build_single_provider(name, provider_cfg).await?;
        providers.insert(name.clone(), provider);
    }

    Ok(providers)
}

async fn build_single_provider(
    name: &str,
    config: &ProviderConfig,
) -> Result<Box<dyn IdentityProvider>, Box<dyn std::error::Error>> {
    match config.adapter.as_str() {
        "oidc" => {
            let oidc_config = provider_config_to_oidc(name, config)?;
            let provider =
                oidc_exchange_adapters::oidc::OidcProvider::from_config(name, &oidc_config).await?;
            Ok(Box::new(provider))
        }
        "apple" => {
            let provider =
                oidc_exchange_providers::apple::AppleProvider::from_config(&config.extra).await?;
            Ok(Box::new(provider))
        }
        other => Err(Box::new(Error::ConfigError {
            detail: format!("unknown provider adapter for '{name}': {other}"),
        })),
    }
}

/// Convert the generic `ProviderConfig` (with its `extra` map) into the typed
/// `OidcProviderConfig` expected by the OIDC adapter.
fn provider_config_to_oidc(
    name: &str,
    config: &ProviderConfig,
) -> Result<oidc_exchange_core::domain::provider::OidcProviderConfig, Error> {
    use oidc_exchange_core::domain::provider::OidcProviderConfig;

    let get_str = |key: &str| -> Option<String> {
        config
            .extra
            .get(key)
            .and_then(|v| v.as_str())
            .map(String::from)
    };

    let issuer = get_str("issuer").ok_or_else(|| Error::ConfigError {
        detail: format!("provider '{name}': missing 'issuer'"),
    })?;

    let client_id = get_str("client_id").ok_or_else(|| Error::ConfigError {
        detail: format!("provider '{name}': missing 'client_id'"),
    })?;

    let scopes = config
        .extra
        .get("scopes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| vec!["openid".to_string()]);

    Ok(OidcProviderConfig {
        provider_id: name.to_string(),
        issuer,
        client_id,
        client_secret: get_str("client_secret"),
        jwks_uri: get_str("jwks_uri"),
        token_endpoint: get_str("token_endpoint"),
        revocation_endpoint: get_str("revocation_endpoint"),
        scopes,
        additional_params: HashMap::new(),
    })
}

// ---------------------------------------------------------------------------
// Config loading tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod load_config_tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Sets one or more environment variables for the duration of a test and
    /// removes them on drop (including when a later assertion panics), so
    /// config-loading tests never leak process-global env state.
    struct EnvVarGuard {
        keys: Vec<&'static str>,
    }

    impl EnvVarGuard {
        fn set(vars: &[(&'static str, &str)]) -> Self {
            let keys = vars.iter().map(|(key, _)| *key).collect();
            for (key, value) in vars {
                std::env::set_var(key, value);
            }
            Self { keys }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            for key in &self.keys {
                std::env::remove_var(key);
            }
        }
    }

    fn write_toml(dir: &Path, stem: &str, contents: &str) {
        fs::write(dir.join(format!("{stem}.toml")), contents).expect("write fixture toml");
    }

    fn dir_str(dir: &Path) -> &str {
        dir.to_str().expect("fixture dir path is valid UTF-8")
    }

    #[test]
    fn overlay_merges_over_default_rather_than_replacing_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [server]
                host = "0.0.0.0"
                port = 8080

                [registration]
                mode = "open"
            "#,
        );
        write_toml(
            dir.path(),
            "overlay-env",
            r#"
                [server]
                port = 9090
            "#,
        );
        let _guard = EnvVarGuard::set(&[("OIDC_EXCHANGE_ENV", "overlay-env")]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        // A key present only in default.toml survives the overlay...
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.registration.mode, "open");
        // ...while a key set in both takes the overlay's value.
        assert_eq!(config.server.port, 9090);
    }

    #[test]
    fn env_var_override_reaches_nested_and_map_valued_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [server]
                port = 8080

                [providers.google]
                adapter = "oidc"
                client_id = "default-client"
            "#,
        );
        let _guard = EnvVarGuard::set(&[
            (
                "OIDC_EXCHANGE__PROVIDERS__GOOGLE__CLIENT_ID",
                "overridden-client",
            ),
            ("OIDC_EXCHANGE__SERVER__PORT", "9999"),
        ]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        assert_eq!(config.server.port, 9999);
        let google = config.providers.get("google").expect("google provider");
        assert_eq!(google.adapter, "oidc", "unrelated fields survive the merge");
        assert_eq!(
            google.extra.get("client_id").and_then(|v| v.as_str()),
            Some("overridden-client")
        );
    }

    #[test]
    fn single_underscore_provider_name_is_addressable_not_split() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [providers.my_idp]
                adapter = "oidc"
                client_id = "a"
            "#,
        );
        let _guard = EnvVarGuard::set(&[("OIDC_EXCHANGE__PROVIDERS__MY_IDP__CLIENT_ID", "b")]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        assert_eq!(
            config.providers.len(),
            1,
            "my_idp must address a single provider entry, not split into extra segments"
        );
        let provider = config
            .providers
            .get("my_idp")
            .expect("my_idp provider present under its full, unsplit name");
        assert_eq!(
            provider.extra.get("client_id").and_then(|v| v.as_str()),
            Some("b")
        );
    }

    #[test]
    fn missing_config_files_fall_back_to_compiled_in_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No files written at all, and no OIDC_EXCHANGE_ENV set.

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");
        let defaults = AppConfig::default();

        assert_eq!(config.server.host, defaults.server.host);
        assert_eq!(config.server.port, defaults.server.port);
        assert_eq!(config.registration.mode, defaults.registration.mode);
        assert_eq!(
            config.token.access_token_ttl,
            defaults.token.access_token_ttl
        );
    }

    #[test]
    fn missing_or_empty_env_overlay_file_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [server]
                host = "10.0.0.1"
            "#,
        );
        // Present but empty overlay file.
        write_toml(dir.path(), "empty-env", "");

        {
            let _guard = EnvVarGuard::set(&[("OIDC_EXCHANGE_ENV", "empty-env")]);
            let config = load_config_from_dir(dir_str(dir.path())).expect("load config");
            assert_eq!(config.server.host, "10.0.0.1");
        }

        // Overlay file that does not exist on disk at all.
        let _guard = EnvVarGuard::set(&[("OIDC_EXCHANGE_ENV", "does-not-exist")]);
        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");
        assert_eq!(config.server.host, "10.0.0.1");
    }

    /// End-to-end reviewability case: all three layers (default TOML, env
    /// overlay TOML, and an `OIDC_EXCHANGE__…` var) are exercised together,
    /// and the merged `AppConfig` carries the untouched default alongside
    /// the overlaid and env-overridden values.
    #[test]
    fn default_overlay_and_env_var_all_apply_together() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [server]
                host = "0.0.0.0"
                port = 8080

                [registration]
                mode = "open"
            "#,
        );
        write_toml(
            dir.path(),
            "full-stack",
            r#"
                [server]
                port = 9090
            "#,
        );
        let _guard = EnvVarGuard::set(&[
            ("OIDC_EXCHANGE_ENV", "full-stack"),
            ("OIDC_EXCHANGE__SERVER__HOST", "203.0.113.5"),
        ]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        // Untouched by either overlay or env var — the compiled default TOML value.
        assert_eq!(config.registration.mode, "open");
        // Set by the env overlay TOML, not present in the env var.
        assert_eq!(config.server.port, 9090);
        // Set by the OIDC_EXCHANGE__ env var, on top of the overlay and default.
        assert_eq!(config.server.host, "203.0.113.5");
    }
}
