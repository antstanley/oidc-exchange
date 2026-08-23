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
    /// Canonicalise every field with a documented normal form, in place,
    /// before [`AppConfig::validate`] runs. Called by both configuration
    /// entry points (`load_config_from_dir` and `parse_config`) immediately
    /// after deserialization, so every consumer downstream of config load —
    /// the router builder, adapters, and embedded hosts — sees the same
    /// canonical shapes and never re-implements tolerance for sloppy ones.
    ///
    /// Normalisations:
    /// - `server.base_path`: `""` and `"/"` resolve to `None` (unset), and a
    ///   single trailing `/` is trimmed (`"/prod/"` → `"/prod"`). A value
    ///   without a leading `/` is deliberately left alone here so
    ///   [`AppConfig::validate`] can reject it by name rather than silently
    ///   rewriting what the operator wrote.
    pub fn normalise(&mut self) {
        self.server.base_path = normalise_base_path(self.server.base_path.take());
    }

    /// Validate the loaded configuration once, at startup, so malformed
    /// config fails closed instead of being absorbed and discovered later
    /// (an unmounted router, a per-request panic, or an over-permissive
    /// allowlist/auth check).
    ///
    /// Checks, each returning a `ConfigError` naming the offending field:
    /// - `server.role` is one of [`ALLOWED_SERVER_ROLES`].
    /// - `server.base_path`, when set, is in its canonical form: a leading
    ///   `/` plus at least one further character, and no trailing `/`
    ///   (`""`, `"/"`, `"prod"`, and `"/prod/"` are all rejected — the
    ///   first two belong as unset, the third needs its leading slash, and
    ///   the last should have been trimmed by [`AppConfig::normalise`]).
    /// - `server.request_timeout`, `token.access_token_ttl`, and
    ///   `token.refresh_token_ttl` parse via
    ///   [`crate::service::parse_duration_secs`].
    /// - Every `registration.domain_allowlist` entry is an exact domain or a
    ///   `*.`-prefixed wildcard.
    /// - When the internal API will be served (`server.role` is `admin` or
    ///   `all`, and `internal_api.enabled == true`),
    ///   `internal_api.shared_secret` is present and non-empty.
    pub fn validate(&self) -> Result<(), Error> {
        if let Some(base_path) = &self.server.base_path {
            let is_canonical =
                base_path.len() > 1 && base_path.starts_with('/') && !base_path.ends_with('/');
            if !is_canonical {
                return Err(Error::ConfigError {
                    detail: format!(
                        "server.base_path {base_path:?} must start with '/' and carry at least \
                         one non-slash character (\"\" and \"/\" mean unset, a trailing \"/\" is \
                         trimmed at load)"
                    ),
                });
            }
        }

        if !ALLOWED_SERVER_ROLES.contains(&self.server.role.as_str()) {
            return Err(Error::ConfigError {
                detail: format!(
                    "server.role {:?} is not one of {ALLOWED_SERVER_ROLES:?}",
                    self.server.role
                ),
            });
        }

        prefix_config_error(
            crate::service::parse_duration_secs(&self.server.request_timeout),
            "server.request_timeout",
        )?;
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

/// Canonicalise a `[server] base_path` value from its deserialized form:
///
/// - `None` stays `None`.
/// - `Some("")` and `Some("/")` become `None` — both spell "no mount
///   prefix", and keeping them as `Some` would force every consumer
///   (strip middleware, embedded hosts) to re-derive that fact.
/// - A single trailing `/` is trimmed (`Some("/prod/")` → `Some("/prod")`),
///   because the strip layer matches at a segment boundary and a trailing
///   slash would otherwise make `"/prod/health"` fail to match its own
///   prefix.
///
/// A value without a leading `/` is returned unchanged: normalisation never
/// invents path structure the operator did not write; [`AppConfig::validate`]
/// rejects it instead, naming the field.
fn normalise_base_path(base_path: Option<String>) -> Option<String> {
    let base_path = base_path?;
    // Trim exactly one trailing slash, then fold any resulting root/empty
    // form back to unset (`"/"` → `None`, `"//"` → `None`) so the canonical
    // set is closed under the operation.
    let trimmed = match base_path.strip_suffix('/') {
        Some(head) => head.to_string(),
        None => base_path,
    };
    if trimmed.is_empty() || trimmed == "/" {
        None
    } else {
        Some(trimmed)
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

/// Default for `[server] request_timeout` when the key is absent from config: a humantime
/// duration string parsed the same way as the `[token]` TTLs (see
/// [`crate::service::parse_duration_secs`]). Named rather than a bare literal so the value
/// backing `AppConfig::validate` and the docs stay in lockstep.
/// See `06-configuration.md` → Sections → `[server]` and Defaults summary.
pub const DEFAULT_REQUEST_TIMEOUT: &str = "30s";

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub issuer: String,
    pub role: String,
    /// Humantime duration string (e.g. `"30s"`) bounding how long the server's
    /// request-timeout middleware layer lets a single request run before aborting it with a
    /// `408`. Parsed via [`crate::service::parse_duration_secs`] and validated at startup by
    /// [`AppConfig::validate`] — an unparseable value fails config load rather than silently
    /// falling back to [`DEFAULT_REQUEST_TIMEOUT`].
    pub request_timeout: String,
    /// Path prefix (e.g. `"/prod"`) stripped from incoming request paths before routing.
    /// Absent (`None`) by default; exists for deployments fronted by a mount prefix such as
    /// an API Gateway stage, where the platform includes the stage name in the request path
    /// but the app's routes are defined without it.
    ///
    /// Canonical by the time any consumer sees it: both configuration entry points run
    /// [`AppConfig::normalise`] (which folds `""`/`"/"` into `None` and trims one trailing
    /// `/`) followed by [`AppConfig::validate`] (which rejects a value with no leading `/`
    /// or a residual trailing `/`). The strip middleware therefore never has to re-derive
    /// these cases on the per-request path.
    pub base_path: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            issuer: String::new(),
            role: "all".to_string(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT.to_string(),
            base_path: None,
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
    /// Severity floor for emitting events at all: events strictly less
    /// severe than this threshold are dropped before any adapter dispatch,
    /// independently of `blocking_threshold`. Parsed with
    /// `service::parse_severity`; defaults to `"info"`.
    pub emit_threshold: String,
    pub sqs: Option<SqsAuditConfig>,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            adapter: "noop".to_string(),
            blocking_threshold: "warning".to_string(),
            emit_threshold: "info".to_string(),
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

/// Upper bound on `[user_sync.webhook].retries`. A misconfigured `retries`
/// must never turn a synchronous request (an admin call, or the JIT notify
/// on token exchange) into an hours-long hang or overflow the backoff shift;
/// [`WebhookConfig::effective_retries`] clamps to this at config load time.
/// See `06-configuration.md` → `[user_sync]`.
pub const MAX_WEBHOOK_RETRIES: u32 = 10;

/// Default `retries` when `[user_sync.webhook].retries` is unset.
const DEFAULT_WEBHOOK_RETRIES: u32 = 2;

#[derive(Clone, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    pub secret: String,
    pub timeout: Option<String>,
    pub retries: Option<u32>,
}

impl WebhookConfig {
    /// The `retries` value that reaches the webhook adapter: the configured
    /// value clamped to [`MAX_WEBHOOK_RETRIES`], or [`DEFAULT_WEBHOOK_RETRIES`]
    /// when unset. Logs a warning naming the configured and clamped values
    /// when clamping actually reduces the configured value.
    pub fn effective_retries(&self) -> u32 {
        let configured = self.retries.unwrap_or(DEFAULT_WEBHOOK_RETRIES);
        if configured > MAX_WEBHOOK_RETRIES {
            tracing::warn!(
                configured_retries = configured,
                clamped_retries = MAX_WEBHOOK_RETRIES,
                "user_sync.webhook.retries exceeds the maximum of {MAX_WEBHOOK_RETRIES}; \
                 clamping"
            );
            MAX_WEBHOOK_RETRIES
        } else {
            configured
        }
    }
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
        assert_eq!(config.server.request_timeout, DEFAULT_REQUEST_TIMEOUT);
        assert_eq!(config.server.request_timeout, "30s");
        assert!(config.server.base_path.is_none());

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
request_timeout = "45s"

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
        assert_eq!(config.server.request_timeout, "45s");
        assert_eq!(
            crate::service::parse_duration_secs(&config.server.request_timeout)
                .expect("overridden request_timeout must still parse as a humantime duration"),
            45
        );

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

    /// `[server] base_path` deserializes to `Some(prefix)` when present and to
    /// `None` when absent, so the strip layer can tell "no prefix configured"
    /// apart from "prefix is the empty string".
    #[test]
    fn server_base_path_deserializes_present_and_absent() {
        let with_base_path: AppConfig = toml::from_str(
            r#"
[server]
base_path = "/prod"
"#,
        )
        .expect("base_path = \"/prod\" must deserialize");
        assert_eq!(with_base_path.server.base_path.as_deref(), Some("/prod"));

        // Negative-space: omitting the key must still deserialize (not a
        // parse error), landing on `None` — the "no mount prefix" default.
        let absent: AppConfig = toml::from_str(
            r#"
[server]
host = "0.0.0.0"
"#,
        )
        .expect("omitting base_path must still deserialize");
        assert!(absent.server.base_path.is_none());
    }

    /// Every tolerated sloppy spelling of `[server] base_path` lands on its canonical
    /// form: unset stays unset, empty/root fold to unset, one trailing slash is trimmed,
    /// and a residual root (`"//"`) folds to unset too.
    #[test]
    fn normalise_base_path_canonicalises_unset_root_and_trailing_slash() {
        assert_eq!(normalise_base_path(None), None, "unset must stay unset");

        assert_eq!(
            normalise_base_path(Some(String::new())),
            None,
            "an empty base_path must normalise to unset"
        );
        assert_eq!(
            normalise_base_path(Some("/".to_string())),
            None,
            "a bare root base_path must normalise to unset"
        );

        assert_eq!(
            normalise_base_path(Some("/prod/".to_string())),
            Some("/prod".to_string()),
            "one trailing slash must be trimmed"
        );
        assert_eq!(
            normalise_base_path(Some("/prod".to_string())),
            Some("/prod".to_string()),
            "an already-canonical value must pass through unchanged"
        );
        assert_eq!(
            normalise_base_path(Some("//".to_string())),
            None,
            "a double slash must fold to unset, not survive as a degenerate prefix"
        );
    }

    /// Negative space: normalisation never invents path structure — a value with no
    /// leading slash survives normalisation untouched so that `validate` can reject it
    /// by name instead of silently rewriting what the operator wrote.
    #[test]
    fn normalise_base_path_leaves_missing_leading_slash_for_validate_to_reject() {
        assert_eq!(
            normalise_base_path(Some("prod".to_string())),
            Some("prod".to_string()),
            "normalise must not prepend a leading slash; validate rejects this instead"
        );
    }

    /// Postcondition of the canonicalisation: no normalised output ever carries a
    /// trailing slash or lacks a leading one — the invariant the strip middleware and
    /// later embedded hosts rely on without re-checking.
    #[test]
    fn normalised_base_path_is_always_canonical() {
        for candidate in ["/prod", "/prod/deep", "/a"] {
            let normalised = normalise_base_path(Some(candidate.to_string()))
                .unwrap_or_else(|| panic!("{candidate:?} is already canonical and must survive"));
            assert!(
                normalised.starts_with('/') && !normalised.ends_with('/'),
                "normalised {candidate:?} must be boundary-safe, got {normalised:?}"
            );
        }
    }

    /// `validate` accepts every post-normalisation shape and rejects each non-canonical
    /// one by name — including the forms `normalise` would have folded away, so a caller
    /// that skips `normalise` still fails closed rather than reaching the router.
    #[test]
    fn validate_rejects_non_canonical_base_path_shapes() {
        let mut config = AppConfig::default();
        config.server.base_path = Some("prod".to_string());
        let err = config
            .validate()
            .expect_err("base_path without a leading slash must be rejected");
        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("server.base_path"),
                    "detail must name the field: {detail}"
                );
                assert!(
                    detail.contains("prod"),
                    "detail must echo the offending value: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }

        for rejected in ["", "/", "/prod/"] {
            let mut config = AppConfig::default();
            config.server.base_path = Some(rejected.to_string());
            assert!(
                config.validate().is_err(),
                "non-canonical base_path {rejected:?} must fail validate when normalise was \
                 skipped"
            );
        }

        // The paired positive space: the canonical form validate exists to guard.
        let mut config = AppConfig::default();
        config.server.base_path = Some("/prod".to_string());
        assert!(
            config.validate().is_ok(),
            "canonical base_path must pass validate: {:?}",
            config.validate()
        );
    }

    /// Load-time contract end to end: through `AppConfig::normalise` + `validate`, the
    /// tolerated spellings resolve to their canonical values and only genuinely invalid
    /// ones abort the load.
    #[test]
    fn normalise_then_validate_yields_load_time_base_path_contract() {
        let mut unset_empty: AppConfig = toml::from_str(
            r#"
[server]
base_path = ""
"#,
        )
        .expect("empty base_path must deserialize");
        unset_empty.normalise();
        unset_empty
            .validate()
            .expect("empty base_path must normalise to unset and load cleanly");
        assert_eq!(unset_empty.server.base_path, None);

        let mut trailing: AppConfig = toml::from_str(
            r#"
[server]
base_path = "/prod/"
"#,
        )
        .expect("trailing-slash base_path must deserialize");
        trailing.normalise();
        trailing
            .validate()
            .expect("trailing slash must be trimmed at load, not rejected");
        assert_eq!(trailing.server.base_path.as_deref(), Some("/prod"));

        let mut missing_leading: AppConfig = toml::from_str(
            r#"
[server]
base_path = "prod"
"#,
        )
        .expect("missing-leading-slash base_path must deserialize");
        missing_leading.normalise();
        assert!(
            missing_leading.validate().is_err(),
            "missing leading slash must abort the load even after normalisation"
        );
    }

    #[test]
    fn effective_retries_clamps_over_max_passes_through_in_range_and_default() {
        let over_max = WebhookConfig {
            url: "https://hooks.example.com".to_string(),
            secret: "s".to_string(),
            timeout: None,
            retries: Some(20),
        };
        assert_eq!(
            over_max.effective_retries(),
            MAX_WEBHOOK_RETRIES,
            "retries = 20 must clamp down to the named maximum of {MAX_WEBHOOK_RETRIES}"
        );

        let in_range = WebhookConfig {
            url: "https://hooks.example.com".to_string(),
            secret: "s".to_string(),
            timeout: None,
            retries: Some(5),
        };
        assert_eq!(
            in_range.effective_retries(),
            5,
            "an in-range retries value must pass through unchanged"
        );

        let unset = WebhookConfig {
            url: "https://hooks.example.com".to_string(),
            secret: "s".to_string(),
            timeout: None,
            retries: None,
        };
        assert_eq!(
            unset.effective_retries(),
            DEFAULT_WEBHOOK_RETRIES,
            "an unset retries must resolve to the documented default of \
             {DEFAULT_WEBHOOK_RETRIES}"
        );
    }

    #[test]
    fn effective_retries_at_max_is_not_clamped() {
        // Negative-space boundary: exactly at the maximum must not trigger
        // the clamp path (it is not "greater than" the maximum).
        let at_max = WebhookConfig {
            url: "https://hooks.example.com".to_string(),
            secret: "s".to_string(),
            timeout: None,
            retries: Some(MAX_WEBHOOK_RETRIES),
        };
        assert_eq!(at_max.effective_retries(), MAX_WEBHOOK_RETRIES);
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

    /// Negative-space: an unparseable `server.request_timeout` must fail `validate` (and
    /// therefore `load_config`/`parse_config`, which call it before anything else is built)
    /// rather than being absorbed and silently falling back to `DEFAULT_REQUEST_TIMEOUT`.
    #[test]
    fn validate_rejects_unparseable_request_timeout() {
        let mut config = AppConfig::default();
        config.server.request_timeout = "not-a-duration".to_string();

        let err = config
            .validate()
            .expect_err("bad request_timeout must be rejected");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("server.request_timeout"),
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
