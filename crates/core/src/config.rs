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
    /// - `server.request_timeout`, `token.access_token_ttl`, and
    ///   `token.refresh_token_ttl` parse via
    ///   [`crate::service::parse_duration_secs`].
    /// - Every `token.custom_claims` key is a non-reserved protocol claim name
    ///   ([`crate::service::claims::RESERVED_CLAIMS`]) — a template claim keyed
    ///   by a reserved name would be silently dropped at token build, so it is
    ///   refused at startup instead.
    /// - Every `registration.domain_allowlist` entry is an exact domain or a
    ///   `*.`-prefixed wildcard.
    /// - When the internal API will be served (`server.role` is `admin` or
    ///   `all`, and `internal_api.enabled == true`), the full
    ///   `[internal_api]` contract holds: non-empty, known, duplicate-free
    ///   `auth_methods`; a shared secret of at least
    ///   [`MIN_SHARED_SECRET_BYTES`] bytes while that mechanism is enabled;
    ///   issuer/key-manager requirements for `operator_token`; parseable
    ///   throttle bounds. See [`Self::validate_internal_api`].
    /// - When `server.role` is `all` and the internal API is enabled, the
    ///   admin listener (`internal_api.host`/`port`) does not collide with the
    ///   public listener (`server.host`/`port`) — see [`listeners_collide`].
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

        if let Some(custom_claims) = &self.token.custom_claims {
            // Sorted so the reported offender is deterministic when several
            // reserved keys are configured.
            let mut keys: Vec<&String> = custom_claims.keys().collect();
            keys.sort();
            for key in keys {
                if crate::service::claims::is_reserved_claim(key) {
                    return Err(Error::ConfigError {
                        detail: format!(
                            "token.custom_claims key {key:?} is a reserved protocol claim \
                             name and cannot be used as a custom claim"
                        ),
                    });
                }
            }
        }

        if let Some(allowlist) = &self.registration.domain_allowlist {
            for entry in allowlist {
                validate_allowlist_entry(entry)?;
            }
        }

        // When the internal API will be served, the whole `[internal_api]`
        // contract applies: a non-empty mechanism list, per-mechanism
        // requirements (the shared secret's length floor; a real issuer and a
        // non-noop key manager for operator tokens), and parseable throttle
        // bounds. See `06-configuration.md` → Validation at load.
        let internal_api_served =
            matches!(self.server.role.as_str(), "admin" | "all") && self.internal_api.enabled;
        if internal_api_served {
            self.validate_internal_api()?;
        }

        if internal_api_served && self.internal_api.shared_secret_is_only_mechanism() {
            tracing::warn!(
                mechanisms = ?self.internal_api.auth_methods,
                "shared_secret is the only enabled internal-API authentication mechanism; \
                 it authenticates requests without identifying anyone - migrate to \
                 operator_token or mtls for attributed admin actions"
            );
        }

        // Only role = "all" binds both sockets, so only that role can collide.
        // Under role = "admin" the public socket is never bound (same values
        // are harmless), and under any other role the admin listener is not
        // bound at all.
        let binds_both_listeners = self.server.role == "all" && self.internal_api.enabled;
        if binds_both_listeners
            && listeners_collide(
                &self.server.host,
                self.server.port,
                &self.internal_api.host,
                self.internal_api.port,
            )
        {
            return Err(Error::ConfigError {
                detail: format!(
                    "internal_api listener {}:{} collides with the public listener {}:{}; \
                     role = \"all\" binds two distinct sockets and they must not share one",
                    self.internal_api.host,
                    self.internal_api.port,
                    self.server.host,
                    self.server.port
                ),
            });
        }

        Ok(())
    }

    /// Validate the `[internal_api]` section whenever the role binds the admin
    /// listener and `internal_api.enabled = true` — i.e. exactly when a
    /// rejected credential would otherwise be discovered at request time.
    ///
    /// Checks (each returning a `ConfigError` naming the offending field):
    /// - `auth_methods` is non-empty, holds only known mechanism names, and
    ///   has no duplicates (a duplicated mechanism would silently try twice).
    /// - when `shared_secret` is among them: present and at least
    ///   [`MIN_SHARED_SECRET_BYTES`] bytes — non-empty is not sufficient for
    ///   the string that is the plane's entire authentication under its
    ///   mechanism.
    /// - when `operator_token` is among them: a non-empty `server.issuer`
    ///   (tokens are verified against it) and a non-noop `key_manager`
    ///   adapter (verification needs real keys; under `role = "admin"` the
    ///   bootstrap otherwise builds a noop manager); a non-empty
    ///   `token_audience`; a non-empty `required_claim` *and*
    ///   `required_value` (an empty required value would accept tokens whose
    ///   claim equals the empty string — no credential gate at all); and a
    ///   `token_audience` distinct from `[token].audience`, which is the one
    ///   structural replay defense separating user access tokens from
    ///   operator credentials minted by the same key manager.
    /// - parseable `auth_failure_window` and `auth_lockout`, and a non-zero
    ///   `max_auth_failures`.
    /// - parseable `stats_cache_ttl` within
    ///   `[MIN_STATS_CACHE_TTL_SECS, MAX_STATS_CACHE_TTL_SECS]` — a
    ///   zero-length TTL would serve "cached" numbers that are never usable,
    ///   and beyond an hour the dashboard counts stop being estimates.
    fn validate_internal_api(&self) -> Result<(), Error> {
        let methods = &self.internal_api.auth_methods;
        if methods.is_empty() {
            return Err(Error::ConfigError {
                detail: "internal_api.auth_methods must be non-empty when the internal API \
                         is served"
                    .to_string(),
            });
        }
        // Sorted so the reported offender is deterministic.
        let mut sorted: Vec<&String> = methods.iter().collect();
        sorted.sort();
        for window in sorted.windows(2) {
            if window[0] == window[1] {
                return Err(Error::ConfigError {
                    detail: format!(
                        "internal_api.auth_methods lists {:?} more than once",
                        window[0]
                    ),
                });
            }
        }
        for method in methods {
            if !ALLOWED_AUTH_METHODS.contains(&method.as_str()) {
                return Err(Error::ConfigError {
                    detail: format!(
                        "internal_api.auth_methods entry {method:?} is not one of \
                         {ALLOWED_AUTH_METHODS:?}"
                    ),
                });
            }
        }

        if self.internal_api.uses_shared_secret() {
            let secret_len = self
                .internal_api
                .shared_secret
                .as_deref()
                .map(str::len)
                .unwrap_or(0);
            if secret_len < MIN_SHARED_SECRET_BYTES {
                // Only the length is reported, never the value.
                return Err(Error::ConfigError {
                    detail: format!(
                        "internal_api.shared_secret must be at least {MIN_SHARED_SECRET_BYTES} \
                         bytes while the shared_secret mechanism is enabled (got {secret_len} bytes)"
                    ),
                });
            }
        }

        if self.internal_api.uses_operator_token() {
            if self.server.issuer.trim().is_empty() {
                return Err(Error::ConfigError {
                    detail: "server.issuer must be non-empty while the operator_token mechanism \
                             is enabled: operator tokens are verified against it"
                        .to_string(),
                });
            }
            let adapter = self.key_manager.adapter.trim();
            if adapter.is_empty() || adapter == "noop" {
                return Err(Error::ConfigError {
                    detail: format!(
                        "key_manager.adapter ({adapter:?}) cannot serve the operator_token \
                         mechanism: token verification requires a real key manager"
                    ),
                });
            }
            if self.internal_api.token_audience.trim().is_empty() {
                return Err(Error::ConfigError {
                    detail: "internal_api.token_audience must be non-empty while the \
                             operator_token mechanism is enabled"
                        .to_string(),
                });
            }
            if self.internal_api.required_claim.trim().is_empty() {
                return Err(Error::ConfigError {
                    detail: "internal_api.required_claim must be non-empty while the \
                             operator_token mechanism is enabled"
                        .to_string(),
                });
            }
            if self.internal_api.required_value.trim().is_empty() {
                // An empty (or blank) required value would gate nothing: any
                // token whose claim equals the empty string — i.e. no real
                // credential property at all — would authenticate.
                return Err(Error::ConfigError {
                    detail: "internal_api.required_value must be non-empty while the \
                             operator_token mechanism is enabled"
                        .to_string(),
                });
            }
            if let Some(public_audience) = &self.token.audience {
                if public_audience == &self.internal_api.token_audience {
                    // The internal audience is the one structural replay
                    // defense between user access tokens and operator
                    // credentials, which share this service's issuer and key
                    // manager. Defaults differ, so equality can only arise
                    // through deliberate misconfiguration — refuse it at load.
                    return Err(Error::ConfigError {
                        detail: format!(
                            "internal_api.token_audience ({public_audience:?}) must differ from \
                             token.audience: a shared audience lets any user access token minted \
                             by this service's key manager be replayed as an operator \
                             credential"
                        ),
                    });
                }
            }
        }

        if self.internal_api.uses_mtls()
            && self.internal_api.mtls_subject_header().trim().is_empty()
        {
            return Err(Error::ConfigError {
                detail: "internal_api.mtls.subject_header must be non-empty while the mtls \
                         mechanism is enabled"
                    .to_string(),
            });
        }

        let failure_window_secs = prefix_config_error(
            crate::service::parse_duration_secs(&self.internal_api.auth_failure_window),
            "internal_api.auth_failure_window",
        )?;
        if failure_window_secs == 0 {
            // A syntactically valid but zero-length window would make the
            // throttle either meaningless or permanent; refuse it at load
            // instead of letting the limiter discover it at startup.
            return Err(Error::ConfigError {
                detail: "internal_api.auth_failure_window must be non-zero".to_string(),
            });
        }
        let lockout_secs = prefix_config_error(
            crate::service::parse_duration_secs(&self.internal_api.auth_lockout),
            "internal_api.auth_lockout",
        )?;
        if lockout_secs == 0 {
            return Err(Error::ConfigError {
                detail: "internal_api.auth_lockout must be non-zero".to_string(),
            });
        }
        if self.internal_api.max_auth_failures == 0 {
            return Err(Error::ConfigError {
                detail: "internal_api.max_auth_failures must be non-zero".to_string(),
            });
        }

        let stats_cache_secs = prefix_config_error(
            crate::service::parse_duration_secs(&self.internal_api.stats_cache_ttl),
            "internal_api.stats_cache_ttl",
        )?;
        if stats_cache_secs < MIN_STATS_CACHE_TTL_SECS {
            return Err(Error::ConfigError {
                detail: format!(
                    "internal_api.stats_cache_ttl must be at least \
                     {MIN_STATS_CACHE_TTL_SECS}s (got {stats_cache_secs}s)"
                ),
            });
        }
        if stats_cache_secs > MAX_STATS_CACHE_TTL_SECS {
            // The bound is stated, never any cached value.
            return Err(Error::ConfigError {
                detail: format!(
                    "internal_api.stats_cache_ttl exceeds the maximum of \
                     {MAX_STATS_CACHE_TTL_SECS}s (got {stats_cache_secs}s)"
                ),
            });
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

/// Default for `[server] request_timeout` when the key is absent from config: a humantime
/// duration string parsed the same way as the `[token]` TTLs (see
/// [`crate::service::parse_duration_secs`]). Named rather than a bare literal so the value
/// backing `AppConfig::validate` and the docs stay in lockstep.
/// See `06-configuration.md` → Sections → `[server]` and Defaults summary.
pub const DEFAULT_REQUEST_TIMEOUT: &str = "30s";

/// Default for `[server] role` when the key is absent from config: serve only
/// the public exchange plane. Named rather than a bare literal so the value
/// backing `ServerConfig::default`, `AppConfig::validate`, and the deployment
/// docs stay in lockstep — admin reachability must be a deliberate
/// configuration act, never a consequence of an omitted key.
/// See `06-configuration.md` → Sections → `[server]` and Defaults summary.
pub const DEFAULT_SERVER_ROLE: &str = "exchange";

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
    pub base_path: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            issuer: String::new(),
            role: DEFAULT_SERVER_ROLE.to_string(),
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

/// Default for `[internal_api] host` when the key is absent from config: the
/// admin listener is reachable only from an operator network, so it binds the
/// loopback interface unless an operator explicitly publishes it. Named rather
/// than a bare literal so the default, `AppConfig::validate`, and the
/// deployment docs stay in lockstep.
/// See `06-configuration.md` → Sections → `[internal_api]` and Defaults summary.
pub const DEFAULT_INTERNAL_API_HOST: &str = "127.0.0.1";

/// Default for `[internal_api] port` when the key is absent from config: one
/// above the public listener's 8080, so the two planes never collide by
/// accident. See `06-configuration.md` → Defaults summary.
pub const DEFAULT_INTERNAL_API_PORT: u16 = 8081;

/// Host values that bind every interface. When either listener binds one of
/// these, a same-port admin listener collides with the public listener on at
/// least one interface, so the pair is rejected rather than discovered as an
/// `EADDRINUSE` at bind time (or worse, silently shared).
const WILDCARD_LISTENER_HOSTS: [&str; 3] = ["0.0.0.0", "::", "[::]"];

/// Whether an admin listener at (`admin_host`, `admin_port`) can share the
/// public listener's socket at (`public_host`, `public_port`).
///
/// Two listeners collide when their host/port pairs are identical, or when
/// their ports are equal and either side binds a wildcard host — a wildcard
/// listener covers every specific interface, so `0.0.0.0:8081` and
/// `127.0.0.1:8081` cannot coexist. Different ports never collide.
pub fn listeners_collide(
    public_host: &str,
    public_port: u16,
    admin_host: &str,
    admin_port: u16,
) -> bool {
    if public_port != admin_port {
        return false;
    }
    let public_wildcard = WILDCARD_LISTENER_HOSTS.contains(&public_host);
    let admin_wildcard = WILDCARD_LISTENER_HOSTS.contains(&admin_host);
    public_wildcard || admin_wildcard || public_host == admin_host
}

/// Minimum accepted length, in bytes, of `internal_api.shared_secret` while
/// the shared-secret mechanism is enabled (source-spec decision "A minimum
/// secret length all the same"). Validation measures length because entropy
/// cannot be measured at load; 32 bytes of generated randomness puts online
/// guessing out of reach without imposing a format.
pub const MIN_SHARED_SECRET_BYTES: usize = 32;

/// Default `token_audience` an operator token must carry for the
/// `operator_token` mechanism to accept it (`06-configuration.md`,
/// `[internal_api]`).
pub const DEFAULT_TOKEN_AUDIENCE: &str = "internal";

/// Default claim name the `operator_token` mechanism requires on a verified
/// token.
pub const DEFAULT_REQUIRED_CLAIM: &str = "role";

/// Default value [`DEFAULT_REQUIRED_CLAIM`] must carry on a verified operator
/// token.
pub const DEFAULT_REQUIRED_VALUE: &str = "admin";

/// Default header a TLS-terminating proxy uses to hand over the client
/// certificate subject for the `mtls` mechanism.
pub const DEFAULT_MTLS_SUBJECT_HEADER: &str = "x-client-cert-subject";

/// Default `[internal_api] max_auth_failures`: failed authentications one peer
/// may spend per failure window before lockout. Far tighter than any public
/// budget — an operator does not retry a credential sixty times a minute.
pub const DEFAULT_MAX_AUTH_FAILURES: u64 = 5;

/// Default `[internal_api] auth_failure_window` (humantime).
pub const DEFAULT_AUTH_FAILURE_WINDOW: &str = "1m";

/// Default `[internal_api] auth_lockout` (humantime): how long a locked-out
/// peer stays denied after exhausting its failure budget.
pub const DEFAULT_AUTH_LOCKOUT: &str = "5m";

/// Default `[internal_api] stats_cache_ttl` (humantime): how long the DynamoDB
/// adapter may serve cached dashboard counts (`count_active_sessions`) before
/// re-scanning the table.
pub const DEFAULT_STATS_CACHE_TTL: &str = "60s";

/// Lower bound, in seconds, on `[internal_api] stats_cache_ttl`. The duration
/// parser resolves whole seconds, and a zero-length TTL would make the cache
/// useless while still reporting "cached" numbers, so validation refuses
/// anything below one second.
pub const MIN_STATS_CACHE_TTL_SECS: u64 = 1;

/// Upper bound, in seconds, on `[internal_api] stats_cache_ttl`: beyond one
/// hour the "active sessions" figure would be an audit-grade lie rather than a
/// cached estimate. Matches the DynamoDB adapter's own `MAX_STATS_CACHE_TTL`.
pub const MAX_STATS_CACHE_TTL_SECS: u64 = 3600;

/// The only values `internal_api.auth_methods` accepts, tried in configured
/// order. `shared_secret` is the compatibility mechanism; it identifies nobody.
const ALLOWED_AUTH_METHODS: [&str; 3] = ["operator_token", "mtls", "shared_secret"];

/// Deserialize a string-or-sequence field: accepts either a single scalar
/// string (read as a one-element list) or a sequence of strings.
///
/// This is what keeps the pre-hardening singular `auth_method` key loadable
/// alongside the list-valued `auth_methods` key (see the field's docs).
fn string_or_seq_string<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct StringOrSeqVisitor;

    impl<'de> serde::de::Visitor<'de> for StringOrSeqVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a string or a sequence of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(vec![value.to_string()])
        }

        fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
        where
            S: serde::de::SeqAccess<'de>,
        {
            let mut methods = Vec::new();
            while let Some(entry) = seq.next_element::<String>()? {
                methods.push(entry);
            }
            Ok(methods)
        }
    }

    deserializer.deserialize_any(StringOrSeqVisitor)
}

/// Per-mechanism configuration for the `mtls` authentication mechanism.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MtlsConfig {
    /// Header carrying the client-certificate subject, set by the
    /// TLS-terminating proxy in front of the admin listener. Trusted only
    /// because the listener is not publicly routable by construction (the
    /// default host is loopback); the server reads this header nowhere except
    /// the admin router's authentication layer.
    pub subject_header: String,
}

impl Default for MtlsConfig {
    fn default() -> Self {
        Self {
            subject_header: DEFAULT_MTLS_SUBJECT_HEADER.to_string(),
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct InternalApiConfig {
    pub enabled: bool,
    /// Host for the dedicated admin listener. Defaults to
    /// [`DEFAULT_INTERNAL_API_HOST`] so publishing the admin plane is an
    /// explicit configuration act.
    pub host: String,
    /// Port for the dedicated admin listener. Defaults to
    /// [`DEFAULT_INTERNAL_API_PORT`]. `AppConfig::validate` rejects a
    /// role = "all" config whose admin listener collides with the public
    /// socket.
    pub port: u16,
    /// Enabled authentication mechanisms, tried in the order given. Empty is
    /// rejected by validation whenever the internal API is served: a served
    /// admin plane with no way in would answer every request with
    /// `not_configured` forever.
    ///
    /// The singular `auth_method = "shared_secret"` key from pre-hardening
    /// deployments is still accepted and read as a one-element list (the
    /// `alias`), so an unedited config keeps loading across this change; both
    /// spellings in one file fail the load as a duplicate field rather than
    /// silently picking one.
    #[serde(
        default,
        alias = "auth_method",
        deserialize_with = "string_or_seq_string"
    )]
    pub auth_methods: Vec<String>,
    /// Compatibility shared secret for the `shared_secret` mechanism;
    /// redacted in `Debug`. Required (at [`MIN_SHARED_SECRET_BYTES`] bytes)
    /// whenever that mechanism is enabled and the internal API is served.
    pub shared_secret: Option<String>,
    /// Audience a verified operator token must carry. Only meaningful when
    /// `operator_token` is among `auth_methods`.
    pub token_audience: String,
    /// Claim name a verified operator token must carry.
    pub required_claim: String,
    /// Value [`Self::required_claim`] must carry on a verified operator token.
    pub required_value: String,
    /// Configuration for the `mtls` mechanism (the proxy header carrying the
    /// client-certificate subject).
    pub mtls: Option<MtlsConfig>,
    /// Failed-authentication budget per peer before lockout.
    pub max_auth_failures: u64,
    /// Window over which failed attempts draw down the budget (humantime).
    pub auth_failure_window: String,
    /// Lockout duration once the failure budget is exhausted (humantime).
    pub auth_lockout: String,
    /// How long the DynamoDB adapter may serve cached dashboard counts
    /// (`count_active_sessions`) before re-scanning (humantime). Validated to
    /// `[MIN_STATS_CACHE_TTL_SECS, MAX_STATS_CACHE_TTL_SECS]`; only the
    /// DynamoDB adapter consumes it.
    pub stats_cache_ttl: String,
}

impl Default for InternalApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: DEFAULT_INTERNAL_API_HOST.to_string(),
            port: DEFAULT_INTERNAL_API_PORT,
            auth_methods: vec!["shared_secret".to_string()],
            shared_secret: None,
            token_audience: DEFAULT_TOKEN_AUDIENCE.to_string(),
            required_claim: DEFAULT_REQUIRED_CLAIM.to_string(),
            required_value: DEFAULT_REQUIRED_VALUE.to_string(),
            mtls: None,
            max_auth_failures: DEFAULT_MAX_AUTH_FAILURES,
            auth_failure_window: DEFAULT_AUTH_FAILURE_WINDOW.to_string(),
            auth_lockout: DEFAULT_AUTH_LOCKOUT.to_string(),
            stats_cache_ttl: DEFAULT_STATS_CACHE_TTL.to_string(),
        }
    }
}

impl InternalApiConfig {
    /// Whether the `shared_secret` compatibility mechanism is enabled.
    pub fn uses_shared_secret(&self) -> bool {
        self.auth_methods
            .iter()
            .any(|method| method == "shared_secret")
    }

    /// Whether the named-principal operator-token mechanism is enabled.
    pub fn uses_operator_token(&self) -> bool {
        self.auth_methods
            .iter()
            .any(|method| method == "operator_token")
    }

    /// Whether the proxy-asserted mTLS-subject mechanism is enabled.
    pub fn uses_mtls(&self) -> bool {
        self.auth_methods.iter().any(|method| method == "mtls")
    }

    /// Whether the shared secret is the *only* enabled mechanism — the state
    /// every deployment should migrate away from; warned about at startup.
    pub fn shared_secret_is_only_mechanism(&self) -> bool {
        self.auth_methods.len() == 1 && self.uses_shared_secret()
    }

    /// The `host:port` string the admin listener binds under the hyper
    /// runtime, asserted non-empty so a misconfigured host can never produce
    /// a degenerate bind address.
    pub fn bind_address(&self) -> String {
        assert!(
            !self.host.is_empty(),
            "internal_api.host must be non-empty before a bind address is composed"
        );
        format!("{}:{}", self.host, self.port)
    }

    /// The header name the `mtls` mechanism reads its subject from, resolved
    /// from config or the documented default.
    pub fn mtls_subject_header(&self) -> &str {
        match &self.mtls {
            Some(cfg) => cfg.subject_header.as_str(),
            None => DEFAULT_MTLS_SUBJECT_HEADER,
        }
    }
}

impl std::fmt::Debug for InternalApiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Credential-bearing fields are redacted: the shared secret is the
        // plane's entire authentication under its mechanism, and
        // `required_value` gates it under the token mechanism — neither value
        // nor length may leak through Debug output into logs or spans.
        f.debug_struct("InternalApiConfig")
            .field("enabled", &self.enabled)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("auth_methods", &self.auth_methods)
            .field(
                "shared_secret",
                &self.shared_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("token_audience", &self.token_audience)
            .field("required_claim", &self.required_claim)
            .field("required_value", &"<redacted>")
            .field("mtls", &self.mtls)
            .field("max_auth_failures", &self.max_auth_failures)
            .field("auth_failure_window", &self.auth_failure_window)
            .field("auth_lockout", &self.auth_lockout)
            .field("stats_cache_ttl", &self.stats_cache_ttl)
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

    /// A shared secret exactly at the validation floor: long enough to satisfy
    /// `MIN_SHARED_SECRET_BYTES`, so tests that do not target the floor rule
    /// never trip over it accidentally.
    const TEST_SHARED_SECRET: &str = "0123456789abcdef0123456789abcdef"; // 32 bytes

    #[test]
    fn test_shared_secret_constant_meets_the_documented_floor() {
        assert_eq!(TEST_SHARED_SECRET.len(), MIN_SHARED_SECRET_BYTES);
        assert_eq!(TEST_SHARED_SECRET, "0123456789abcdef0123456789abcdef");
    }

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
        assert_eq!(config.server.role, DEFAULT_SERVER_ROLE);
    }

    /// An omitted `[server].role` must land on the exchange-only default, so a
    /// stock process never serves the internal admin API by accident.
    #[test]
    fn server_role_absent_deserializes_to_exchange_default() {
        let config: AppConfig = toml::from_str(
            r#"
[server]
host = "0.0.0.0"
"#,
        )
        .expect("config without server.role must deserialize");

        assert_eq!(
            config.server.role, DEFAULT_SERVER_ROLE,
            "an omitted server.role must default to {DEFAULT_SERVER_ROLE}, never to a \
             role that serves the admin plane"
        );
    }

    /// Explicit roles are compatibility surfaces: an installation that sets
    /// `all` or `admin` deliberately must keep exactly what it configured.
    #[test]
    fn explicit_all_and_admin_roles_are_preserved() {
        let all: AppConfig = toml::from_str(
            r#"
[server]
role = "all"
"#,
        )
        .expect("explicit role = \"all\" must deserialize");
        let admin: AppConfig = toml::from_str(
            r#"
[server]
role = "admin"
"#,
        )
        .expect("explicit role = \"admin\" must deserialize");

        assert_eq!(
            all.server.role, "all",
            "an explicit \"all\" must be preserved verbatim"
        );
        assert_eq!(
            admin.server.role, "admin",
            "an explicit \"admin\" must be preserved verbatim"
        );
    }

    /// Even with `internal_api.enabled = true`, an omitted role must fail no
    /// validation and yet never count as "serving" the internal API: enabling
    /// the flag alone cannot turn the default process into an admin plane
    /// (task 04's listener split builds on this same gate).
    #[test]
    fn default_role_with_enabled_internal_api_is_not_served() {
        let mut config = AppConfig::default();
        config.internal_api.enabled = true;
        // No shared secret on purpose: under the default role the internal API
        // is not served, so the served-secret requirement must not fire.
        config.internal_api.shared_secret = None;

        let result = config.validate();

        assert!(
            result.is_ok(),
            "the exchange-only default must not require internal-API credentials: {result:?}"
        );
        assert_eq!(
            config.server.role, DEFAULT_SERVER_ROLE,
            "the test is only meaningful for the absent-role default"
        );
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

    /// A `token.custom_claims` entry keyed by a reserved protocol name must
    /// fail startup: token build would silently drop it, so the operator would
    /// ship a claim they configured but never see.
    #[test]
    fn validate_rejects_reserved_name_in_token_custom_claims() {
        for reserved in ["sub", "sid", "roles"] {
            let mut config = AppConfig::default();
            let mut custom = HashMap::new();
            custom.insert(reserved.to_string(), "override".to_string());
            config.token.custom_claims = Some(custom);

            let err = config
                .validate()
                .expect_err("a reserved key in token.custom_claims must be rejected at load");

            match err {
                Error::ConfigError { detail } => {
                    assert!(
                        detail.contains("token.custom_claims"),
                        "detail must name the field: {detail}"
                    );
                    assert!(
                        detail.contains(&format!("\"{reserved}\"")),
                        "detail must name the offending key: {detail}"
                    );
                }
                other => panic!("expected ConfigError, got {other:?}"),
            }
        }
    }

    /// Paired positive: non-reserved keys — including near-misses like
    /// `role` and case variants like `Sub` — validate, keep their template
    /// values, and survive in any order relative to reserved names.
    #[test]
    fn validate_accepts_non_reserved_token_custom_claim_keys() {
        let mut config = AppConfig::default();
        let mut custom = HashMap::new();
        custom.insert("role".to_string(), "{{ user.metadata.role }}".to_string());
        custom.insert("Sub".to_string(), "not-reserved".to_string());
        custom.insert("tenant".to_string(), "acme".to_string());
        config.token.custom_claims = Some(custom);

        let result = config.validate();

        assert!(
            result.is_ok(),
            "non-reserved custom-claim keys must validate: {result:?}"
        );
        assert_eq!(config.token.custom_claims.as_ref().unwrap().len(), 3);
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
        config.internal_api.shared_secret = Some(TEST_SHARED_SECRET.to_string());

        let result = config.validate();

        assert!(result.is_ok(), "non-empty secret must pass: {result:?}");
        assert!(config.internal_api.enabled);
    }

    // -------------------------------------------------------------------------
    // Admin listener separation (task 04): host/port defaults and collision rule
    // -------------------------------------------------------------------------

    /// The admin listener defaults to loopback on the port above the public
    /// listener: publishing it must be an explicit configuration act, and the
    /// default must never collide with `server.host`/`server.port`.
    #[test]
    fn internal_api_listener_defaults_to_loopback_adjacent_port() {
        let config = InternalApiConfig::default();

        assert_eq!(config.host, DEFAULT_INTERNAL_API_HOST);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, DEFAULT_INTERNAL_API_PORT);
        assert_eq!(config.port, 8081);
        assert_eq!(
            config.bind_address(),
            format!("{DEFAULT_INTERNAL_API_HOST}:{DEFAULT_INTERNAL_API_PORT}")
        );
    }

    /// Table over the collision predicate: identical pairs collide; equal
    /// ports with any wildcard side collide; different ports never do.
    #[test]
    fn listeners_collide_matches_exact_and_wildcard_pairs_only() {
        let cases = [
            ("0.0.0.0", 8081_u16, "0.0.0.0", 8081_u16, true),
            ("127.0.0.1", 8081, "127.0.0.1", 8081, true),
            ("0.0.0.0", 8080, "127.0.0.1", 8081, false),
            ("127.0.0.1", 8080, "127.0.0.1", 8081, false),
            // Equal port, either side wildcard: the wildcard listener covers
            // every interface including the specific one.
            ("0.0.0.0", 8081, "127.0.0.1", 8081, true),
            ("127.0.0.1", 8081, "::", 8081, true),
            ("[::]", 8081, "[::]", 8081, true),
            // Equal port, both specific and distinct hosts: separate sockets.
            ("127.0.0.1", 8081, "10.0.0.5", 8081, false),
        ];
        for (public_host, public_port, admin_host, admin_port, expected) in cases {
            assert_eq!(
                listeners_collide(public_host, public_port, admin_host, admin_port),
                expected,
                "public {public_host}:{public_port} vs admin {admin_host}:{admin_port}"
            );
        }
    }

    /// Negative space: role = "all" with an enabled internal API whose
    /// listener collides with the public socket must fail startup — two
    /// listeners are the whole point of the split, and a shared socket would
    /// silently re-merge the planes.
    #[test]
    fn validate_rejects_all_role_with_colliding_admin_listener() {
        let mut config = AppConfig::default();
        config.server.role = "all".to_string();
        config.server.port = 8080;
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(TEST_SHARED_SECRET.to_string());
        config.internal_api.host = "0.0.0.0".to_string();
        config.internal_api.port = 8080;

        let err = config
            .validate()
            .expect_err("a colliding admin listener must be rejected");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("internal_api"),
                    "detail must name the colliding section: {detail}"
                );
                assert!(
                    detail.contains("collides"),
                    "detail must describe the collision: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    /// Paired positive: the same colliding values under role = "admin" are
    /// fine, because that role never binds the public socket at all.
    #[test]
    fn validate_accepts_admin_role_on_the_public_port() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.server.port = 8080;
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(TEST_SHARED_SECRET.to_string());
        config.internal_api.host = "0.0.0.0".to_string();
        config.internal_api.port = 8080;

        let result = config.validate();

        assert!(
            result.is_ok(),
            "role=admin binds only the admin socket, so no collision exists: {result:?}"
        );
    }

    /// And role = "all" with distinct ports (the documented default shape)
    /// validates: the collision rule fires only on genuine overlap.
    #[test]
    fn validate_accepts_all_role_with_distinct_listeners() {
        let mut config = AppConfig::default();
        config.server.role = "all".to_string();
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(TEST_SHARED_SECRET.to_string());

        let result = config.validate();

        assert!(
            result.is_ok(),
            "the default loopback:8081 admin listener must not collide: {result:?}"
        );
        assert_ne!(config.internal_api.port, config.server.port);
    }

    /// A disabled internal API removes the admin listener entirely, so its
    /// host/port can duplicate anything without failing validation.
    #[test]
    fn validate_ignores_admin_listener_collision_when_disabled() {
        let mut config = AppConfig::default();
        config.server.role = "all".to_string();
        config.internal_api.enabled = false;
        config.internal_api.host = "0.0.0.0".to_string();
        config.internal_api.port = config.server.port;

        let result = config.validate();

        assert!(
            result.is_ok(),
            "a disabled admin listener cannot collide: {result:?}"
        );
        assert!(!config.internal_api.enabled);
    }

    // -----------------------------------------------------------------------
    // Operator-auth mechanism validation (task 05)
    // -----------------------------------------------------------------------

    /// The source spec's exact floor boundary: a 31-byte shared secret fails
    /// startup while a 32-byte one boots. Length is the only measurable
    /// proxy for strength at load time, so the boundary is load-bearing.
    #[test]
    fn shared_secret_length_floor_boundary_is_enforced() {
        let below = "x".repeat(MIN_SHARED_SECRET_BYTES - 1);
        assert_eq!(below.len(), MIN_SHARED_SECRET_BYTES - 1);

        let mut short = AppConfig::default();
        short.server.role = "admin".to_string();
        short.internal_api.enabled = true;
        short.internal_api.shared_secret = Some(below);
        let err = short
            .validate()
            .expect_err("a secret one byte under the floor must fail startup");
        match &err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("at least"),
                    "detail must state the floor, not the value: {detail}"
                );
                // Only the length is reported — never any part of the value.
                assert!(
                    !detail.contains("xxx"),
                    "the error must not echo the secret value: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }

        let mut exact = AppConfig::default();
        exact.server.role = "admin".to_string();
        exact.internal_api.enabled = true;
        exact.internal_api.shared_secret = Some("y".repeat(MIN_SHARED_SECRET_BYTES));
        assert!(
            exact.validate().is_ok(),
            "a secret exactly at the floor must boot"
        );
    }

    /// The operator-token mechanism requires a real key manager: on the
    /// admin role (which otherwise builds the noop manager) a noop adapter
    /// with `operator_token` enabled must fail startup rather than serve a
    /// mechanism that can never verify a signature.
    #[test]
    fn validate_rejects_operator_token_on_the_noop_key_manager() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.server.issuer = "https://auth.example.com".to_string();
        config.internal_api.enabled = true;
        config.key_manager.adapter = "noop".to_string();
        config.internal_api.auth_methods = vec!["operator_token".to_string()];

        let err = config
            .validate()
            .expect_err("operator_token cannot run on the noop key manager");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("key_manager.adapter") && detail.contains("operator_token"),
                    "detail must name both the adapter and the mechanism: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    /// Paired positive: the same mechanism with a real adapter and a non-empty
    /// issuer validates.
    #[test]
    fn validate_accepts_operator_token_with_a_real_key_manager_and_issuer() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.server.issuer = "https://auth.example.com".to_string();
        config.internal_api.enabled = true;
        config.key_manager.adapter = "local".to_string();
        config.key_manager.local = Some(crate::config::LocalKeyConfig {
            private_key_path: "/tmp/operator-signing-key.pem".to_string(),
            algorithm: "EdDSA".to_string(),
            kid: "operator-test-key".to_string(),
        });
        config.internal_api.auth_methods = vec!["operator_token".to_string()];
        config.internal_api.shared_secret = None;

        let result = config.validate();

        assert!(
            result.is_ok(),
            "a real key manager with an issuer satisfies operator_token: {result:?}"
        );
    }

    /// An empty `required_claim` was already refused; its value needs the same
    /// gate. A blank `required_value` would accept any token whose claim
    /// equals the empty string — a credential check that checks nothing.
    #[test]
    fn validate_rejects_blank_required_value_when_operator_token_enabled() {
        let base = || {
            let mut config = AppConfig::default();
            config.server.role = "admin".to_string();
            config.server.issuer = "https://auth.example.com".to_string();
            config.internal_api.enabled = true;
            config.key_manager.adapter = "local".to_string();
            config.key_manager.local = Some(crate::config::LocalKeyConfig {
                private_key_path: "/tmp/operator-signing-key.pem".to_string(),
                algorithm: "EdDSA".to_string(),
                kid: "operator-test-key".to_string(),
            });
            config.internal_api.auth_methods = vec!["operator_token".to_string()];
            config.internal_api.shared_secret = None;
            config
        };

        for blank in ["", "   "] {
            let mut config = base();
            config.internal_api.required_value = blank.to_string();
            let err = config
                .validate()
                .expect_err("a blank required_value must be rejected");
            match err {
                Error::ConfigError { detail } => {
                    assert!(
                        detail.contains("required_value"),
                        "detail must name the offending field: {detail}"
                    );
                }
                other => panic!("expected ConfigError, got {other:?}"),
            }
        }
    }

    /// The internal audience is the one replay defense between user access
    /// tokens and operator credentials (same issuer, same key manager), so an
    /// `[internal_api].token_audience` equal to `[token].audience` fails
    /// startup; distinct values — including the default pair — boot.
    #[test]
    fn validate_rejects_an_operator_token_audience_shared_with_user_tokens() {
        let mut shared = AppConfig::default();
        shared.server.role = "admin".to_string();
        shared.server.issuer = "https://auth.example.com".to_string();
        shared.internal_api.enabled = true;
        shared.key_manager.adapter = "local".to_string();
        shared.key_manager.local = Some(crate::config::LocalKeyConfig {
            private_key_path: "/tmp/operator-signing-key.pem".to_string(),
            algorithm: "EdDSA".to_string(),
            kid: "operator-test-key".to_string(),
        });
        shared.internal_api.auth_methods = vec!["operator_token".to_string()];
        shared.internal_api.shared_secret = None;
        shared.token.audience = Some(shared.internal_api.token_audience.clone());

        let err = shared
            .validate()
            .expect_err("a shared audience defeats the replay defense");
        match &err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("token_audience") && detail.contains("differ"),
                    "the rejection must name both audiences' rule: {detail}"
                );
                // The message explains why, but never echoes credential material.
                assert!(
                    detail.contains("replayed"),
                    "the rejection must state the replay risk: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }

        // Distinct audiences — here the default internal audience against an
        // explicit public one — validate.
        let mut distinct = shared;
        distinct.token.audience = Some("https://api.example.com".to_string());
        assert!(
            distinct.validate().is_ok(),
            "distinct audiences keep the replay defense intact"
        );
    }

    /// The shared-audience rule scopes to deployments that verify operator
    /// tokens at all: without `operator_token` among the mechanisms no
    /// operator credential exists to replay, so the equality is merely odd,
    /// not dangerous.
    #[test]
    fn audience_equality_is_only_refused_while_operator_token_is_enabled() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(TEST_SHARED_SECRET.to_string());
        config.token.audience = Some(config.internal_api.token_audience.clone());

        assert!(
            config.validate().is_ok(),
            "without operator_token there is no operator credential to replay"
        );
    }

    /// An empty `auth_methods` list on a served plane is rejected: a served
    /// admin API with no way in would answer every request `not_configured`
    /// forever.
    #[test]
    fn validate_rejects_served_internal_api_with_no_mechanisms() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.internal_api.enabled = true;
        config.internal_api.auth_methods = Vec::new();

        let err = config
            .validate()
            .expect_err("an empty mechanism list must be rejected when served");
        match err {
            Error::ConfigError { detail } => {
                assert!(detail.contains("non-empty"), "got: {detail}");
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    /// Unknown mechanism names are rejected loudly rather than silently
    /// ignored (a typo like `operater_token` would otherwise disable every
    /// other configured path to the plane).
    #[test]
    fn validate_rejects_unknown_auth_mechanism_names() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.internal_api.enabled = true;
        config.internal_api.auth_methods = vec![
            "shared_secret".to_string(),
            "operater_token".to_string(), // deliberate typo
        ];
        config.internal_api.shared_secret = Some(TEST_SHARED_SECRET.to_string());

        let err = config
            .validate()
            .expect_err("an unknown mechanism name must be rejected");
        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("operater_token"),
                    "detail must name the unknown entry: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    /// Duplicate mechanisms would double-evaluate credentials; the list is
    /// closed and duplicate-free.
    #[test]
    fn validate_rejects_duplicate_auth_mechanisms() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.internal_api.enabled = true;
        config.internal_api.auth_methods =
            vec!["shared_secret".to_string(), "shared_secret".to_string()];
        config.internal_api.shared_secret = Some(TEST_SHARED_SECRET.to_string());

        let err = config
            .validate()
            .expect_err("duplicate mechanisms must be rejected");
        match err {
            Error::ConfigError { detail } => {
                assert!(detail.contains("more than once"), "got: {detail}");
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    /// An empty mtls subject header cannot assert identities; validation
    /// refuses the combination up front.
    #[test]
    fn validate_rejects_empty_mtls_subject_header_when_mtls_enabled() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.internal_api.enabled = true;
        config.internal_api.auth_methods = vec!["mtls".to_string()];
        config.internal_api.mtls = Some(MtlsConfig {
            subject_header: "   ".to_string(),
        });

        let err = config
            .validate()
            .expect_err("a blank mtls subject header must be rejected");
        match err {
            Error::ConfigError { detail } => {
                assert!(detail.contains("subject_header"), "got: {detail}");
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    /// Throttle bounds are validated at load: a zero budget or zero window
    /// would produce either an always-open or an always-closed throttle.
    #[test]
    fn validate_rejects_degenerate_throttle_bounds() {
        let base = || {
            let mut config = AppConfig::default();
            config.server.role = "admin".to_string();
            config.internal_api.enabled = true;
            config.internal_api.shared_secret = Some(TEST_SHARED_SECRET.to_string());
            config
        };

        let mut zero_budget = base();
        zero_budget.internal_api.max_auth_failures = 0;
        let err = zero_budget
            .validate()
            .expect_err("a zero failure budget must be rejected");
        assert!(
            matches!(err, Error::ConfigError { ref detail } if detail.contains("max_auth_failures")),
            "got: {err:?}"
        );

        let mut zero_window = base();
        zero_window.internal_api.auth_failure_window = "0s".to_string();
        let err = zero_window
            .validate()
            .expect_err("a zero failure window must be rejected");
        assert!(
            matches!(err, Error::ConfigError { ref detail } if detail.contains("auth_failure_window")),
            "got: {err:?}"
        );

        let mut zero_lockout = base();
        zero_lockout.internal_api.auth_lockout = "0m".to_string();
        let err = zero_lockout
            .validate()
            .expect_err("a zero lockout must be rejected");
        assert!(
            matches!(err, Error::ConfigError { ref detail } if detail.contains("auth_lockout")),
            "got: {err:?}"
        );
    }

    /// `stats_cache_ttl` is validated at load like every other duration: the
    /// documented default boots, the exact bounds boot, and a zero-length,
    /// sub-minimum, over-maximum, or unparseable value fails startup naming
    /// the field.
    #[test]
    fn validate_bounds_stats_cache_ttl() {
        let base = || {
            let mut config = AppConfig::default();
            config.server.role = "admin".to_string();
            config.internal_api.enabled = true;
            config.internal_api.shared_secret = Some(TEST_SHARED_SECRET.to_string());
            config
        };

        // The documented default parses and passes as-is.
        let config = base();
        assert_eq!(config.internal_api.stats_cache_ttl, "60s");
        assert!(config.validate().is_ok());

        // Exact bounds are accepted.
        let mut low = base();
        low.internal_api.stats_cache_ttl = "1s".to_string();
        assert!(low.validate().is_ok(), "the minimum bound boots");
        let mut high = base();
        high.internal_api.stats_cache_ttl = "3600s".to_string();
        assert!(high.validate().is_ok(), "the maximum bound boots");

        // Zero is refused: a cache that never serves is worse than no cache,
        // because the number still presents itself as cached.
        let mut zero = base();
        zero.internal_api.stats_cache_ttl = "0s".to_string();
        let err = zero
            .validate()
            .expect_err("a zero stats-cache TTL must be rejected");
        assert!(
            matches!(err, Error::ConfigError { ref detail } if detail.contains("stats_cache_ttl")),
            "got: {err:?}"
        );

        // Above the one-hour maximum the dashboard counts stop being
        // estimates; refuse loudly rather than serving an audit-grade lie.
        let mut over = base();
        over.internal_api.stats_cache_ttl = "3601s".to_string();
        let err = over
            .validate()
            .expect_err("a stats-cache TTL beyond the maximum must be rejected");
        match &err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("stats_cache_ttl") && detail.contains("maximum"),
                    "the rejection must name the field and the bound: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }

        // Unparseable durations fail with the field named by the shared
        // duration-error prefixing.
        let mut garbage = base();
        garbage.internal_api.stats_cache_ttl = "soon".to_string();
        let err = garbage
            .validate()
            .expect_err("an unparseable stats-cache TTL must be rejected");
        assert!(
            matches!(err, Error::ConfigError { ref detail } if detail.contains("stats_cache_ttl")),
            "got: {err:?}"
        );
    }
}
