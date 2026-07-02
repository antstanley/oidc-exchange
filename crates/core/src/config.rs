use serde::Deserialize;
use std::collections::HashMap;

use crate::error::Error;

/// Top-level application configuration, matching the TOML structure.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub registration: RegistrationConfig,
    pub token: TokenConfig,
    pub audit: AuditConfig,
    pub key_manager: KeyManagerConfig,
    pub repository: RepositoryConfig,
    #[serde(default)]
    pub session_repository: SessionRepositoryConfig,
    pub user_sync: UserSyncConfig,
    pub telemetry: TelemetryConfig,
    pub internal_api: InternalApiConfig,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

/// The only values `server.role` may take: which parts of the API a process
/// serves. See `06-configuration.md` → Sections → `[server]`.
const ALLOWED_SERVER_ROLES: [&str; 3] = ["all", "exchange", "admin"];

impl AppConfig {
    /// Validate the loaded configuration once, at startup, so malformed
    /// config fails closed instead of being absorbed and discovered later
    /// (an unmounted router, a per-request panic, or an over-permissive
    /// allowlist/auth check).
    ///
    /// Checks, each returning a `ConfigError` naming the offending field:
    /// - `server.role` is one of [`ALLOWED_SERVER_ROLES`].
    /// - `token.access_token_ttl` and `token.refresh_token_ttl` parse via
    ///   [`crate::service::parse_duration_secs`].
    /// - Every `registration.domain_allowlist` entry is an exact domain or a
    ///   `*.`-prefixed wildcard.
    /// - When the internal API will be served (`server.role` is `admin` or
    ///   `all`, and `internal_api.enabled == true`),
    ///   `internal_api.shared_secret` is present and non-empty.
    pub fn validate(&self) -> Result<(), Error> {
        if !ALLOWED_SERVER_ROLES.contains(&self.server.role.as_str()) {
            return Err(Error::ConfigError {
                detail: format!(
                    "server.role {:?} is not one of {ALLOWED_SERVER_ROLES:?}",
                    self.server.role
                ),
            });
        }

        prefix_config_error(
            crate::service::parse_duration_secs(&self.token.access_token_ttl),
            "token.access_token_ttl",
        )?;
        prefix_config_error(
            crate::service::parse_duration_secs(&self.token.refresh_token_ttl),
            "token.refresh_token_ttl",
        )?;

        if let Some(allowlist) = &self.registration.domain_allowlist {
            for entry in allowlist {
                validate_allowlist_entry(entry)?;
            }
        }

        let internal_api_served =
            matches!(self.server.role.as_str(), "admin" | "all") && self.internal_api.enabled;
        if internal_api_served {
            let secret_is_present_and_non_empty = self
                .internal_api
                .shared_secret
                .as_deref()
                .is_some_and(|secret| !secret.is_empty());
            if !secret_is_present_and_non_empty {
                return Err(Error::ConfigError {
                    detail: "internal_api.shared_secret must be non-empty when the internal API \
                             is served (server.role is \"admin\" or \"all\" and \
                             internal_api.enabled = true)"
                        .to_string(),
                });
            }
        }

        Ok(())
    }
}

/// Rewrap a `parse_duration_secs` failure with the config field it came
/// from, so the reported `ConfigError` names the offending TOML key rather
/// than just the raw duration string.
fn prefix_config_error<T>(result: Result<T, Error>, field: &str) -> Result<T, Error> {
    result.map_err(|err| match err {
        Error::ConfigError { detail } => Error::ConfigError {
            detail: format!("{field}: {detail}"),
        },
        other => other,
    })
}

/// Validate a single `registration.domain_allowlist` entry: only an exact
/// domain (`example.com`) or a `*.`-prefixed wildcard (`*.example.com`) is
/// accepted. A bare `*` or a dotless prefix (`*example.com`) is rejected —
/// both would let `matches_domain_allowlist` (`service::exchange`) match
/// domains the operator never intended to allow.
fn validate_allowlist_entry(entry: &str) -> Result<(), Error> {
    if entry.starts_with('*') && !entry.starts_with("*.") {
        return Err(Error::ConfigError {
            detail: format!(
                "registration.domain_allowlist entry {entry:?} must be an exact domain \
                 (\"example.com\") or a \"*.\"-prefixed wildcard (\"*.example.com\")"
            ),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub issuer: String,
    pub role: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            issuer: String::new(),
            role: "all".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RegistrationConfig {
    pub mode: String,
    pub domain_allowlist: Option<Vec<String>>,
}

impl Default for RegistrationConfig {
    fn default() -> Self {
        Self {
            mode: "open".to_string(),
            domain_allowlist: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TokenConfig {
    pub access_token_ttl: String,
    pub refresh_token_ttl: String,
    pub audience: Option<String>,
    pub custom_claims: Option<HashMap<String, String>>,
}

impl Default for TokenConfig {
    fn default() -> Self {
        Self {
            access_token_ttl: "15m".to_string(),
            refresh_token_ttl: "30d".to_string(),
            audience: None,
            custom_claims: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AuditConfig {
    pub adapter: String,
    pub blocking_threshold: String,
    pub sqs: Option<SqsAuditConfig>,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            adapter: "noop".to_string(),
            blocking_threshold: "warning".to_string(),
            sqs: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SqsAuditConfig {
    pub queue_url: String,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct KeyManagerConfig {
    pub adapter: String,
    pub kms: Option<KmsConfig>,
    pub local: Option<LocalKeyConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KmsConfig {
    pub key_id: String,
    pub algorithm: String,
    pub kid: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalKeyConfig {
    pub private_key_path: String,
    pub algorithm: String,
    pub kid: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RepositoryConfig {
    pub adapter: String,
    pub dynamodb: Option<DynamoConfig>,
    pub postgres: Option<PostgresConfig>,
    pub sqlite: Option<SqliteConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SessionRepositoryConfig {
    pub adapter: Option<String>,
    pub valkey: Option<ValkeyConfig>,
    pub lmdb: Option<LmdbConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DynamoConfig {
    pub table_name: String,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PostgresConfig {
    pub url: String,
    pub max_connections: Option<u32>,
    /// Whether `create_pool` should run the adapter's idempotent migration
    /// DDL before returning. Absent (`None`) resolves to `true` at the call
    /// site; set `false` for locked-down databases where the app role has
    /// no DDL rights and migrations are applied out-of-band.
    pub run_migrations: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SqliteConfig {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ValkeyConfig {
    pub url: String,
    pub key_prefix: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LmdbConfig {
    pub path: String,
    pub max_size_mb: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserSyncConfig {
    pub enabled: bool,
    pub adapter: Option<String>,
    pub webhook: Option<WebhookConfig>,
}

#[derive(Clone, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    pub secret: String,
    pub timeout: Option<String>,
    pub retries: Option<u32>,
}

impl std::fmt::Debug for WebhookConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookConfig")
            .field("url", &self.url)
            .field("secret", &"<redacted>")
            .field("timeout", &self.timeout)
            .field("retries", &self.retries)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub exporter: String,
    pub endpoint: Option<String>,
    pub service_name: Option<String>,
    pub sample_rate: Option<f64>,
    pub protocol: Option<String>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            exporter: "none".to_string(),
            endpoint: None,
            service_name: None,
            sample_rate: Some(1.0),
            protocol: None,
        }
    }
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct InternalApiConfig {
    pub enabled: bool,
    pub auth_method: Option<String>,
    pub shared_secret: Option<String>,
}

impl std::fmt::Debug for InternalApiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InternalApiConfig")
            .field("enabled", &self.enabled)
            .field("auth_method", &self.auth_method)
            .field(
                "shared_secret",
                &self.shared_secret.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Provider configuration. The `adapter` field selects the provider type, and
/// all remaining fields are captured into `extra` via `#[serde(flatten)]` so
/// that each adapter can define its own schema.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    pub adapter: String,
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_default_toml() {
        let toml_str = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/default.toml"
        ))
        .expect("failed to read config/default.toml");

        let config: AppConfig = toml::from_str(&toml_str).expect("failed to deserialize config");

        // Server defaults
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert!(config.server.issuer.is_empty());

        // Registration defaults
        assert_eq!(config.registration.mode, "open");
        assert!(config.registration.domain_allowlist.is_none());

        // Token defaults
        assert_eq!(config.token.access_token_ttl, "15m");
        assert_eq!(config.token.refresh_token_ttl, "30d");
        assert!(config.token.audience.is_none());
        assert!(config.token.custom_claims.is_none());

        // Audit defaults
        assert_eq!(config.audit.adapter, "noop");
        assert_eq!(config.audit.blocking_threshold, "warning");
        assert!(config.audit.sqs.is_none());

        // Telemetry defaults
        assert!(!config.telemetry.enabled);
        assert_eq!(config.telemetry.exporter, "none");

        // User sync defaults
        assert!(!config.user_sync.enabled);

        // Internal API defaults
        assert!(!config.internal_api.enabled);

        // No providers in default config
        assert!(config.providers.is_empty());
    }

    #[test]
    fn deserialize_full_config() {
        let toml_str = r#"
[server]
host = "127.0.0.1"
port = 9090
issuer = "https://auth.example.com"

[registration]
mode = "existing_users_only"
domain_allowlist = ["example.com", "*.acme.corp"]

[token]
access_token_ttl = "15m"
refresh_token_ttl = "30d"
audience = "https://api.example.com"

[token.custom_claims]
org = "example"
role = "admin"

[audit]
adapter = "sqs"
blocking_threshold = "warning"

[audit.sqs]
queue_url = "https://sqs.us-east-1.amazonaws.com/123456789012/audit-events"
region = "us-east-1"

[key_manager]
adapter = "kms"

[key_manager.kms]
key_id = "arn:aws:kms:us-east-1:123456:key/abc"
algorithm = "ECDSA_SHA_256"
kid = "key-2024-01"

[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "oidc-exchange"
region = "us-east-1"

[user_sync]
enabled = true
adapter = "webhook"

[user_sync.webhook]
url = "https://hooks.example.com/sync"
secret = "super-secret"
timeout = "5s"
retries = 2

[telemetry]
enabled = true
exporter = "otlp"
endpoint = "http://localhost:4317"
service_name = "oidc-exchange"
sample_rate = 0.5
protocol = "grpc"

[internal_api]
enabled = true
auth_method = "shared_secret"
shared_secret = "my-secret"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "google-client-id"
client_secret = "google-client-secret"
scopes = ["openid", "email", "profile"]
"#;

        let config: AppConfig =
            toml::from_str(toml_str).expect("failed to deserialize full config");

        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 9090);
        assert_eq!(config.server.issuer, "https://auth.example.com");

        assert_eq!(config.registration.mode, "existing_users_only");
        let allowlist = config.registration.domain_allowlist.unwrap();
        assert_eq!(allowlist.len(), 2);

        assert_eq!(
            config.token.audience.as_deref(),
            Some("https://api.example.com")
        );
        let claims = config.token.custom_claims.unwrap();
        assert_eq!(claims.get("org").unwrap(), "example");

        assert_eq!(config.audit.adapter, "sqs");
        let sqs_cfg = config.audit.sqs.unwrap();
        assert_eq!(
            sqs_cfg.queue_url,
            "https://sqs.us-east-1.amazonaws.com/123456789012/audit-events"
        );
        assert_eq!(sqs_cfg.region.as_deref(), Some("us-east-1"));

        let kms = config.key_manager.kms.unwrap();
        assert_eq!(kms.algorithm, "ECDSA_SHA_256");

        let dynamo = config.repository.dynamodb.unwrap();
        assert_eq!(dynamo.table_name, "oidc-exchange");
        assert_eq!(dynamo.region.as_deref(), Some("us-east-1"));

        assert!(config.user_sync.enabled);
        let webhook = config.user_sync.webhook.unwrap();
        assert_eq!(webhook.retries, Some(2));

        assert!(config.telemetry.enabled);
        assert_eq!(config.telemetry.exporter, "otlp");
        assert_eq!(config.telemetry.sample_rate, Some(0.5));

        assert!(config.internal_api.enabled);
        assert_eq!(
            config.internal_api.shared_secret.as_deref(),
            Some("my-secret")
        );

        let google = config.providers.get("google").unwrap();
        assert_eq!(google.adapter, "oidc");
        assert_eq!(
            google.extra.get("issuer").unwrap().as_str().unwrap(),
            "https://accounts.google.com"
        );
    }

    /// `[repository.postgres] run_migrations` deserializes to `Some(true|false)`
    /// when present and to `None` (later resolved as `true`) when absent, for
    /// all three cases an operator's TOML can express.
    #[test]
    fn postgres_run_migrations_deserializes_present_and_absent() {
        let with_false: AppConfig = toml::from_str(
            r#"
[repository]
adapter = "postgres"

[repository.postgres]
url = "postgres://localhost/oidc"
run_migrations = false
"#,
        )
        .expect("run_migrations = false must deserialize");
        assert_eq!(
            with_false.repository.postgres.unwrap().run_migrations,
            Some(false)
        );

        let with_true: AppConfig = toml::from_str(
            r#"
[repository]
adapter = "postgres"

[repository.postgres]
url = "postgres://localhost/oidc"
run_migrations = true
"#,
        )
        .expect("run_migrations = true must deserialize");
        assert_eq!(
            with_true.repository.postgres.unwrap().run_migrations,
            Some(true)
        );

        // Negative-space: omitting the key must still deserialize (not a
        // parse error), landing on `None` so the call site can resolve the
        // documented default of `true`.
        let absent: AppConfig = toml::from_str(
            r#"
[repository]
adapter = "postgres"

[repository.postgres]
url = "postgres://localhost/oidc"
"#,
        )
        .expect("omitting run_migrations must still deserialize");
        assert_eq!(absent.repository.postgres.unwrap().run_migrations, None);
    }

    #[test]
    fn validate_accepts_well_formed_default_config() {
        let config = AppConfig::default();

        let result = config.validate();

        assert!(
            result.is_ok(),
            "well-formed config must validate: {result:?}"
        );
        assert_eq!(config.server.role, "all");
    }

    #[test]
    fn validate_rejects_unknown_role() {
        let mut config = AppConfig::default();
        config.server.role = "exchang".to_string();

        let err = config.validate().expect_err("typo'd role must be rejected");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("server.role"),
                    "detail must name the field: {detail}"
                );
                assert!(
                    detail.contains("exchang"),
                    "detail must echo the bad value: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_unparseable_access_token_ttl() {
        let mut config = AppConfig::default();
        config.token.access_token_ttl = "not-a-duration".to_string();

        let err = config.validate().expect_err("bad TTL must be rejected");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("token.access_token_ttl"),
                    "detail must name the field: {detail}"
                );
                assert!(
                    detail.contains("not-a-duration"),
                    "detail must echo the bad value: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_overflowing_refresh_token_ttl() {
        let mut config = AppConfig::default();
        config.token.refresh_token_ttl = format!("{}d", u64::MAX);

        let err = config
            .validate()
            .expect_err("overflowing TTL must be rejected");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("token.refresh_token_ttl"),
                    "detail must name the field: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_bare_wildcard_allowlist_entry() {
        let mut config = AppConfig::default();
        config.registration.domain_allowlist = Some(vec!["*".to_string()]);

        let err = config.validate().expect_err("bare * must be rejected");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("domain_allowlist"),
                    "detail must name the field: {detail}"
                );
                assert!(
                    detail.contains('*'),
                    "detail must echo the offending entry: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_dotless_wildcard_allowlist_entry() {
        let mut config = AppConfig::default();
        config.registration.domain_allowlist = Some(vec!["*example.com".to_string()]);

        let err = config
            .validate()
            .expect_err("dotless *-prefix must be rejected");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("domain_allowlist"),
                    "detail must name the field: {detail}"
                );
                assert!(
                    detail.contains("*example.com"),
                    "detail must echo the offending entry: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    #[test]
    fn validate_accepts_exact_and_wildcard_allowlist_entries() {
        let mut config = AppConfig::default();
        config.registration.domain_allowlist =
            Some(vec!["example.com".to_string(), "*.example.com".to_string()]);

        let result = config.validate();

        assert!(
            result.is_ok(),
            "well-formed allowlist must pass: {result:?}"
        );
        assert_eq!(
            config.registration.domain_allowlist.as_ref().unwrap().len(),
            2
        );
    }

    #[test]
    fn validate_rejects_served_internal_api_with_missing_secret() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = None;

        let err = config
            .validate()
            .expect_err("missing secret on a served internal API must be rejected");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("internal_api.shared_secret"),
                    "detail must name the field: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_served_internal_api_with_empty_secret() {
        let mut config = AppConfig::default();
        config.server.role = "all".to_string();
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(String::new());

        let err = config
            .validate()
            .expect_err("empty secret on a served internal API must be rejected");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("internal_api.shared_secret"),
                    "detail must name the field: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    #[test]
    fn validate_does_not_require_secret_when_internal_api_not_served() {
        // Role excludes the internal API even though it is "enabled".
        let mut role_excludes = AppConfig::default();
        role_excludes.server.role = "exchange".to_string();
        role_excludes.internal_api.enabled = true;
        role_excludes.internal_api.shared_secret = None;
        assert!(
            role_excludes.validate().is_ok(),
            "role=exchange never serves the internal API, so no secret is required"
        );

        // Role admits the internal API but it is disabled.
        let mut disabled = AppConfig::default();
        disabled.server.role = "admin".to_string();
        disabled.internal_api.enabled = false;
        disabled.internal_api.shared_secret = None;
        assert!(
            disabled.validate().is_ok(),
            "internal_api.enabled = false never serves the internal API, so no secret is required"
        );
    }

    #[test]
    fn validate_accepts_served_internal_api_with_non_empty_secret() {
        let mut config = AppConfig::default();
        config.server.role = "all".to_string();
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some("shhh".to_string());

        let result = config.validate();

        assert!(result.is_ok(), "non-empty secret must pass: {result:?}");
        assert!(config.internal_api.enabled);
    }
}
