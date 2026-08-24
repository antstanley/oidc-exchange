use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::domain::AuditSeverity;
use crate::error::Error;

/// Top-level serde boundary for configuration, matching the TOML structure.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawConfig {
    pub server: RawServerConfig,
    pub registration: RawRegistrationConfig,
    pub token: RawTokenConfig,
    pub grants: RawGrantsConfig,
    pub audit: RawAuditConfig,
    pub key_manager: RawKeyManagerConfig,
    pub repository: RawRepositoryConfig,
    #[serde(default)]
    pub session_repository: RawSessionRepositoryConfig,
    pub rate_limit: RawRateLimitConfig,
    pub user_sync: RawUserSyncConfig,
    pub telemetry: RawTelemetryConfig,
    pub internal_api: RawInternalApiConfig,
    #[serde(default)]
    pub providers: HashMap<String, RawProviderConfig>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub registration: RegistrationConfig,
    pub token: TokenConfig,
    pub grants: GrantsConfig,
    pub audit: AuditConfig,
    pub key_manager: KeyManagerConfig,
    pub repository: RepositoryConfig,
    pub session_repository: SessionRepositoryConfig,
    pub rate_limit: RateLimitConfig,
    pub user_sync: UserSyncConfig,
    pub telemetry: TelemetryConfig,
    pub internal_api: InternalApiConfig,
    pub providers: HashMap<String, ProviderConfig>,
}

impl Config {
    pub fn test_default() -> Self {
        Self::resolve(
            toml::from_str(include_str!("../../../config/default.toml"))
                .expect("default test config is valid"),
        )
        .expect("default test config resolves")
    }

    pub fn resolve(raw: RawConfig) -> Result<Self, Error> {
        let server = ServerConfig::resolve(raw.server)?;
        let registration = RegistrationConfig::resolve(raw.registration)?;
        let token = TokenConfig::resolve(raw.token)?;
        let grants = GrantsConfig::resolve(raw.grants)?;
        let audit = AuditConfig::resolve(raw.audit)?;
        let key_manager = KeyManagerConfig::resolve(raw.key_manager)?;
        let repository = RepositoryConfig::resolve(raw.repository)?;
        let session_repository = SessionRepositoryConfig::resolve(raw.session_repository)?;
        let rate_limit = RateLimitConfig::resolve(raw.rate_limit)?;
        let user_sync = UserSyncConfig::resolve(raw.user_sync)?;
        let telemetry = TelemetryConfig::resolve(raw.telemetry)?;
        let internal_api = InternalApiConfig::resolve(raw.internal_api)?;
        let providers = raw
            .providers
            .into_iter()
            .map(|(provider_id, raw_provider)| {
                ProviderConfig::resolve(provider_id, raw_provider)
                    .map(|provider| (provider.provider_id.clone(), provider))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;

        let config = Self {
            server,
            registration,
            token,
            grants,
            audit,
            key_manager,
            repository,
            session_repository,
            rate_limit,
            user_sync,
            telemetry,
            internal_api,
            providers,
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.internal_api.enabled
            && matches!(self.server.role, ServerRole::Admin | ServerRole::All)
            && self
                .internal_api
                .shared_secret
                .as_ref()
                .map(|s| s.as_ref())
                .unwrap_or("")
                .is_empty()
        {
            return Err(Error::ConfigError { detail: "internal_api.shared_secret must be non-empty when the internal API is served (server.role is \"admin\" or \"all\" and internal_api.enabled = true)".to_string() });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RawServerConfig {
    pub host: String,
    pub port: u16,
    pub issuer: String,
    pub role: String,
    pub request_timeout: String,
    pub base_path: Option<String>,
    /// CIDR blocks of reverse proxies whose `X-Forwarded-For` may be trusted.
    pub trusted_proxies: Vec<String>,
    /// How many proxy hops to strip when resolving the client address.
    pub trusted_proxy_hops: usize,
}

impl Default for RawServerConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 0,
            issuer: String::new(),
            role: String::new(),
            request_timeout: String::new(),
            base_path: None,
            trusted_proxies: Vec::new(),
            trusted_proxy_hops: DEFAULT_TRUSTED_PROXY_HOPS,
        }
    }
}

/// Default for `server.trusted_proxy_hops`: one reverse proxy in front.
pub const DEFAULT_TRUSTED_PROXY_HOPS: usize = 1;
/// Upper bound on `server.trusted_proxy_hops`.
pub const MAX_TRUSTED_PROXY_HOPS: usize = 16;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub issuer: HttpsUrl,
    pub role: ServerRole,
    pub request_timeout: std::time::Duration,
    pub base_path: Option<String>,
    /// CIDR blocks of reverse proxies whose forwarded-address headers may be
    /// trusted; parsed at load so the middleware never re-parses per request.
    pub trusted_proxies: Vec<IpNet>,
    /// How many proxy hops to strip when resolving the client address.
    /// Narrowed at load to `1..=MAX_TRUSTED_PROXY_HOPS`.
    pub trusted_proxy_hops: usize,
}

impl ServerConfig {
    fn resolve(raw: RawServerConfig) -> Result<Self, Error> {
        Ok(Self {
            host: raw.host,
            port: raw.port,
            issuer: HttpsUrl::parse_field("server.issuer", raw.issuer)?,
            role: ServerRole::parse_field("server.role", raw.role)?,
            request_timeout: parse_duration_field("server.request_timeout", &raw.request_timeout)?,
            base_path: raw.base_path,
            trusted_proxies: raw
                .trusted_proxies
                .iter()
                .map(|cidr| {
                    cidr.parse::<IpNet>().map_err(|_| Error::ConfigError {
                        detail: format!("server.trusted_proxies entry {cidr:?} must be a CIDR"),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            trusted_proxy_hops: {
                if !(1..=MAX_TRUSTED_PROXY_HOPS).contains(&raw.trusted_proxy_hops) {
                    return Err(Error::ConfigError {
                        detail: format!(
                            "server.trusted_proxy_hops must be between 1 and {MAX_TRUSTED_PROXY_HOPS}"
                        ),
                    });
                }
                raw.trusted_proxy_hops
            },
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawRegistrationConfig {
    pub mode: String,
    pub domain_allowlist: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct RegistrationConfig {
    pub mode: RegistrationMode,
    pub domain_allowlist: Option<Vec<AsciiDomainPattern>>,
}

impl RegistrationConfig {
    fn resolve(raw: RawRegistrationConfig) -> Result<Self, Error> {
        Ok(Self {
            mode: RegistrationMode::parse_field("registration.mode", raw.mode)?,
            domain_allowlist: raw
                .domain_allowlist
                .map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| {
                            AsciiDomainPattern::parse_field("registration.domain_allowlist", entry)
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RawTokenConfig {
    pub access_token_ttl: String,
    pub refresh_token_ttl: String,
    pub audience: String,
    pub custom_claims: Option<HashMap<String, String>>,
    /// Whether each refresh-token redemption mints a replacement and retires
    /// the presented generation (the default), or restores reusable tokens for
    /// clients that cannot discard a rotated token. `false` disables both the
    /// replacement response and rotation itself; retirement records left over
    /// from a rotation-enabled period are then refused as unknown.
    pub refresh_rotation: bool,
    /// Duration string bounding how long the immediately-preceding generation
    /// stays redeemable after a rotation. Narrowed at load: strictly positive
    /// and capped at [`MAX_REFRESH_ROTATION_GRACE_SECS`].
    pub refresh_rotation_grace: String,
    /// Duration string bounding how long a retired generation is remembered so
    /// its re-presentation raises a reuse alarm; per record it is additionally
    /// capped at the family's own `expires_at`. Narrowed at load: strictly
    /// positive.
    pub refresh_reuse_retention: String,
}

impl Default for RawTokenConfig {
    fn default() -> Self {
        Self {
            access_token_ttl: String::new(),
            refresh_token_ttl: String::new(),
            audience: String::new(),
            custom_claims: None,
            refresh_rotation: true,
            refresh_rotation_grace: DEFAULT_REFRESH_ROTATION_GRACE.to_string(),
            refresh_reuse_retention: DEFAULT_REFRESH_REUSE_RETENTION.to_string(),
        }
    }
}

/// Upper bound, in seconds, on `[token] refresh_rotation_grace`. The grace
/// window is a deliberate weakening — inside it a superseded generation may
/// still rotate — so an unbounded window is indistinguishable from no
/// rotation. See `06-configuration.md` → `[token]`.
pub const MAX_REFRESH_ROTATION_GRACE_SECS: u64 = 60;

/// Default for `[token] refresh_rotation_grace`: covers a retried HTTP round
/// trip and little else. Named rather than a bare literal so the value backing
/// resolution, `RawTokenConfig::default`, and the docs stay in lockstep. See
/// `06-configuration.md` → Defaults summary.
pub const DEFAULT_REFRESH_ROTATION_GRACE: &str = "10s";

/// Default for `[token] refresh_reuse_retention`: long enough to cover an
/// attacker racing the legitimate holder for the current credential, short
/// enough that continuously-refreshing families do not accumulate thousands of
/// retirement records. See `06-configuration.md` → Defaults summary.
pub const DEFAULT_REFRESH_REUSE_RETENTION: &str = "24h";

#[derive(Debug, Clone)]
pub struct TokenConfig {
    pub access_token_ttl: std::time::Duration,
    pub refresh_token_ttl: std::time::Duration,
    pub audience: NonEmptyString,
    pub custom_claims: Option<HashMap<String, String>>,
    /// Whether refresh-token redemption rotates the presented generation.
    pub refresh_rotation: bool,
    /// How long the immediately-preceding generation stays redeemable after a
    /// rotation. Strictly positive, at most [`MAX_REFRESH_ROTATION_GRACE_SECS`].
    pub refresh_rotation_grace: std::time::Duration,
    /// How long a retired generation is remembered so its re-presentation
    /// raises a reuse alarm. Strictly positive.
    pub refresh_reuse_retention: std::time::Duration,
}

impl TokenConfig {
    /// The reuse-retention window in seconds. The session adapters compute
    /// every retirement record's deadline from this. Mirrors
    /// [`SessionRepositoryConfig::cleanup_interval_secs`].
    pub fn refresh_reuse_retention_secs(&self) -> u64 {
        self.refresh_reuse_retention.as_secs()
    }
}

impl TokenConfig {
    fn resolve(raw: RawTokenConfig) -> Result<Self, Error> {
        Ok(Self {
            access_token_ttl: parse_duration_field(
                "token.access_token_ttl",
                &raw.access_token_ttl,
            )?,
            refresh_token_ttl: parse_duration_field(
                "token.refresh_token_ttl",
                &raw.refresh_token_ttl,
            )?,
            audience: NonEmptyString::parse_field("token.audience", raw.audience)?,
            custom_claims: raw.custom_claims,
            refresh_rotation: raw.refresh_rotation,
            refresh_rotation_grace: parse_positive_duration_field_capped(
                "token.refresh_rotation_grace",
                &raw.refresh_rotation_grace,
                MAX_REFRESH_ROTATION_GRACE_SECS,
            )?,
            refresh_reuse_retention: parse_positive_duration_field(
                "token.refresh_reuse_retention",
                &raw.refresh_reuse_retention,
            )?,
        })
    }
}

/// Whether the direct ID-token grant is served when `[grants]` is absent from config.
/// The compiled default keeps the grant **off**: an operator who never asks for the
/// direct grant serves no new public surface and gains the replay protection this
/// switch gates by default. See `06-configuration.md` → Sections → `[grants]`.
pub const DEFAULT_GRANTS_ID_TOKEN: bool = false;

/// Default for `[grants] nonce_ttl` when the key is absent: how long a nonce minted for
/// the direct ID-token grant remains claimable. A humantime duration string parsed the
/// same way as the `[token]` TTLs. See `06-configuration.md` → Defaults summary.
pub const DEFAULT_NONCE_TTL: &str = "10m";

/// Default for `[grants] max_assertion_lifetime` when the key is absent: the ceiling on
/// an accepted provider ID token's remaining lifetime, so a replay marker always outlives
/// the assertion it guards. A humantime duration string.
/// See `06-configuration.md` → Defaults summary.
pub const DEFAULT_MAX_ASSERTION_LIFETIME: &str = "1h";

/// Serde mirror of `[grants]`, resolved into [`GrantsConfig`].
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RawGrantsConfig {
    pub id_token: bool,
    pub nonce_ttl: String,
    pub max_assertion_lifetime: String,
}

impl Default for RawGrantsConfig {
    fn default() -> Self {
        Self {
            id_token: DEFAULT_GRANTS_ID_TOKEN,
            nonce_ttl: DEFAULT_NONCE_TTL.to_string(),
            max_assertion_lifetime: DEFAULT_MAX_ASSERTION_LIFETIME.to_string(),
        }
    }
}

/// Which grants `/token` serves and the replay-protection parameters of the direct
/// ID-token grant. The authorization-code and refresh-token grants are always served and
/// have no switch; only the direct ID-token grant — whose credential is a transferable
/// bearer assertion — is opt-in. Both durations are narrowed at load, so an unparseable
/// value fails config resolution rather than being absorbed until first use.
/// See `06-configuration.md` → Sections → `[grants]`.
#[derive(Debug, Clone)]
pub struct GrantsConfig {
    /// Whether the direct ID-token grant is served at all. Defaults to
    /// [`DEFAULT_GRANTS_ID_TOKEN`] (`false`), keeping the grant off unless an operator
    /// explicitly enables it.
    pub id_token: bool,
    /// How long a nonce minted for the direct ID-token grant remains claimable.
    /// Defaults to [`DEFAULT_NONCE_TTL`].
    pub nonce_ttl: std::time::Duration,
    /// The ceiling on the remaining lifetime an accepted provider ID token may carry;
    /// an assertion with longer to live is refused. Defaults to
    /// [`DEFAULT_MAX_ASSERTION_LIFETIME`].
    pub max_assertion_lifetime: std::time::Duration,
}

impl GrantsConfig {
    fn resolve(raw: RawGrantsConfig) -> Result<Self, Error> {
        Ok(Self {
            id_token: raw.id_token,
            nonce_ttl: parse_duration_field("grants.nonce_ttl", &raw.nonce_ttl)?,
            max_assertion_lifetime: parse_duration_field(
                "grants.max_assertion_lifetime",
                &raw.max_assertion_lifetime,
            )?,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RawAuditConfig {
    pub adapter: String,
    pub blocking_threshold: String,
    pub emit_threshold: String,
    /// Mandatory security audit failure policy: `observe` or `enforce`.
    pub durability: String,
    pub sqs: Option<RawSqsAuditConfig>,
}

impl Default for RawAuditConfig {
    fn default() -> Self {
        Self {
            adapter: String::new(),
            blocking_threshold: String::new(),
            emit_threshold: String::new(),
            durability: "observe".to_string(),
            sqs: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditConfig {
    pub adapter: AuditAdapter,
    pub blocking_threshold: AuditSeverity,
    pub emit_threshold: AuditSeverity,
    /// Mandatory security audit failure policy: whether a failed mandatory
    /// emission is observed (logged, counted) or enforced (the request fails).
    pub durability: AuditDurability,
    pub sqs: Option<SqsAuditConfig>,
}

/// Closed domain for `audit.durability`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditDurability {
    Observe,
    Enforce,
}

impl AuditDurability {
    pub fn is_enforce(self) -> bool {
        matches!(self, Self::Enforce)
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "observe" => Ok(Self::Observe),
            "enforce" => Ok(Self::Enforce),
            other => Err(Error::ConfigError {
                detail: format!("{field} {other:?} must be \"observe\" or \"enforce\""),
            }),
        }
    }
}

impl AuditConfig {
    fn resolve(raw: RawAuditConfig) -> Result<Self, Error> {
        Ok(Self {
            adapter: AuditAdapter::parse_field("audit.adapter", raw.adapter)?,
            blocking_threshold: parse_audit_severity_field(
                "audit.blocking_threshold",
                raw.blocking_threshold,
            )?,
            emit_threshold: parse_audit_severity_field("audit.emit_threshold", raw.emit_threshold)?,
            durability: AuditDurability::parse_field("audit.durability", raw.durability)?,
            sqs: raw.sqs.map(SqsAuditConfig::resolve).transpose()?,
        })
    }
}

fn parse_audit_severity_field(field: &str, value: String) -> Result<AuditSeverity, Error> {
    crate::service::parse_severity(&value).ok_or_else(|| Error::ConfigError {
        detail: format!("{field}: invalid audit severity {value:?}"),
    })
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawSqsAuditConfig {
    pub queue_url: String,
    pub region: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SqsAuditConfig {
    pub queue_url: String,
    pub region: Option<String>,
}

impl SqsAuditConfig {
    fn resolve(raw: RawSqsAuditConfig) -> Result<Self, Error> {
        Ok(Self {
            queue_url: NonEmptyString::parse_field("audit.sqs.queue_url", raw.queue_url)?
                .into_inner(),
            region: raw.region,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RawKeyManagerConfig {
    #[serde(default = "default_unconfigured_adapter")]
    pub adapter: String,
    pub kms: Option<RawKmsConfig>,
    pub local: Option<RawLocalKeyConfig>,
}

impl Default for RawKeyManagerConfig {
    fn default() -> Self {
        Self {
            adapter: default_unconfigured_adapter(),
            kms: None,
            local: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct KeyManagerConfig {
    pub adapter: ProviderAdapter,
    pub kms: Option<KmsConfig>,
    pub local: Option<LocalKeyConfig>,
}

fn default_unconfigured_adapter() -> String {
    "oidc".to_string()
}

impl KeyManagerConfig {
    fn resolve(raw: RawKeyManagerConfig) -> Result<Self, Error> {
        Ok(Self {
            adapter: ProviderAdapter::parse_field("key_manager.adapter", raw.adapter)?,
            kms: raw.kms.map(KmsConfig::resolve).transpose()?,
            local: raw.local.map(LocalKeyConfig::resolve).transpose()?,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawKmsConfig {
    pub key_id: String,
    pub algorithm: String,
    pub kid: String,
}

#[derive(Debug, Clone)]
pub struct KmsConfig {
    pub key_id: NonEmptyString,
    pub algorithm: SigningAlgorithm,
    pub kid: NonEmptyString,
}

impl KmsConfig {
    fn resolve(raw: RawKmsConfig) -> Result<Self, Error> {
        Ok(Self {
            key_id: NonEmptyString::parse_field("key_manager.kms.key_id", raw.key_id)?,
            algorithm: SigningAlgorithm::parse_kms_field(
                "key_manager.kms.algorithm",
                raw.algorithm,
            )?,
            kid: NonEmptyString::parse_field("key_manager.kms.kid", raw.kid)?,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawLocalKeyConfig {
    pub private_key_path: String,
    pub algorithm: String,
    pub kid: String,
}

#[derive(Debug, Clone)]
pub struct LocalKeyConfig {
    pub private_key_path: NonEmptyString,
    pub algorithm: SigningAlgorithm,
    pub kid: NonEmptyString,
}

impl LocalKeyConfig {
    fn resolve(raw: RawLocalKeyConfig) -> Result<Self, Error> {
        Ok(Self {
            private_key_path: NonEmptyString::parse_field(
                "key_manager.local.private_key_path",
                raw.private_key_path,
            )?,
            algorithm: SigningAlgorithm::parse_local_field(
                "key_manager.local.algorithm",
                raw.algorithm,
            )?,
            kid: NonEmptyString::parse_field("key_manager.local.kid", raw.kid)?,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RawRepositoryConfig {
    #[serde(default = "default_unconfigured_adapter")]
    pub adapter: String,
    pub dynamodb: Option<RawDynamoConfig>,
    pub postgres: Option<RawPostgresConfig>,
    pub sqlite: Option<RawSqliteConfig>,
}

impl Default for RawRepositoryConfig {
    fn default() -> Self {
        Self {
            adapter: default_unconfigured_adapter(),
            dynamodb: None,
            postgres: None,
            sqlite: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RepositoryConfig {
    pub adapter: ProviderAdapter,
    pub dynamodb: Option<DynamoConfig>,
    pub postgres: Option<PostgresConfig>,
    pub sqlite: Option<SqliteConfig>,
}

impl RepositoryConfig {
    fn resolve(raw: RawRepositoryConfig) -> Result<Self, Error> {
        Ok(Self {
            adapter: ProviderAdapter::parse_field("repository.adapter", raw.adapter)?,
            dynamodb: raw.dynamodb.map(DynamoConfig::resolve).transpose()?,
            postgres: raw.postgres.map(PostgresConfig::resolve).transpose()?,
            sqlite: raw.sqlite.map(SqliteConfig::resolve).transpose()?,
        })
    }
}

/// Default for `[session_repository] cleanup_interval`: how often the
/// long-lived runtimes' session reaper calls `cleanup_expired_sessions` to
/// sweep expired sessions and retirement records. On the natively-expiring
/// stores (DynamoDB TTL, Valkey key expiry) the sweep is a cheap backstop. See
/// `06-configuration.md` → Defaults summary.
pub const DEFAULT_SESSION_CLEANUP_INTERVAL: &str = "1h";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RawSessionRepositoryConfig {
    pub adapter: Option<String>,
    pub valkey: Option<RawValkeyConfig>,
    pub lmdb: Option<RawLmdbConfig>,
    /// Duration string setting how often the long-lived runtimes' reaper calls
    /// `cleanup_expired_sessions` to sweep expired sessions and retirement
    /// records. Narrowed at load: strictly positive.
    pub cleanup_interval: String,
}

impl Default for RawSessionRepositoryConfig {
    fn default() -> Self {
        Self {
            adapter: None,
            valkey: None,
            lmdb: None,
            cleanup_interval: DEFAULT_SESSION_CLEANUP_INTERVAL.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionRepositoryConfig {
    pub adapter: Option<ProviderAdapter>,
    pub valkey: Option<ValkeyConfig>,
    pub lmdb: Option<LmdbConfig>,
    /// How often the long-lived runtimes' session reaper sweeps expired
    /// sessions and retirement records. Strictly positive.
    pub cleanup_interval: std::time::Duration,
}

impl SessionRepositoryConfig {
    /// The cleanup interval in seconds, for the reaper's tick arithmetic.
    pub fn cleanup_interval_secs(&self) -> u64 {
        self.cleanup_interval.as_secs()
    }
}

pub const DEFAULT_RATE_LIMIT_MAX_ENTRIES: usize = 10_000;
pub const MIN_RATE_LIMIT_MAX_ENTRIES: usize = 1;
pub const MAX_RATE_LIMIT_MAX_ENTRIES: usize = 100_000;
pub const MIN_RATE_LIMIT_WINDOW_SECS: u64 = 1;
pub const MAX_RATE_LIMIT_WINDOW_SECS: u64 = 24 * 60 * 60;
pub const MIN_RATE_LIMIT_MAX_CONCURRENT_REQUESTS: usize = 1;
pub const MAX_RATE_LIMIT_MAX_CONCURRENT_REQUESTS: usize = 4_096;
pub const MAX_RATE_LIMIT_BUDGET: u64 = 1_000_000;

/// Serde mirror of `[rate_limit]`, resolved into [`RateLimitConfig`].
/// A scope budget of zero intentionally disables that scope.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RawRateLimitConfig {
    pub enabled: bool,
    pub store: String,
    pub window: String,
    pub per_ip: u64,
    pub per_ip_failures: u64,
    pub per_subject: u64,
    pub per_provider: u64,
    pub max_concurrent_requests: usize,
    pub max_entries: usize,
}

impl Default for RawRateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            store: "in_process".to_string(),
            window: "1m".to_string(),
            per_ip: 60,
            per_ip_failures: 10,
            per_subject: 10,
            per_provider: 600,
            max_concurrent_requests: 256,
            max_entries: DEFAULT_RATE_LIMIT_MAX_ENTRIES,
        }
    }
}

/// Closed domain for `rate_limit.store`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitStore {
    InProcess,
    None,
}

impl RateLimitStore {
    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "in_process" => Ok(Self::InProcess),
            "none" => Ok(Self::None),
            other => Err(Error::ConfigError {
                detail: format!("{field} {other:?} must be \"in_process\" or \"none\""),
            }),
        }
    }
}

/// Fixed-window rate-limit settings, narrowed at load. A scope budget of zero
/// intentionally disables that scope.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub store: RateLimitStore,
    pub window: std::time::Duration,
    pub per_ip: u64,
    pub per_ip_failures: u64,
    pub per_subject: u64,
    pub per_provider: u64,
    pub max_concurrent_requests: usize,
    pub max_entries: usize,
}

impl RateLimitConfig {
    fn resolve(raw: RawRateLimitConfig) -> Result<Self, Error> {
        let store = RateLimitStore::parse_field("rate_limit.store", raw.store)?;
        if raw.enabled && store == RateLimitStore::None {
            return Err(Error::ConfigError {
                detail: "rate_limit.store must be \"in_process\" when rate_limit.enabled is true"
                    .to_string(),
            });
        }
        let window = parse_duration_field("rate_limit.window", &raw.window)?;
        if !(MIN_RATE_LIMIT_WINDOW_SECS..=MAX_RATE_LIMIT_WINDOW_SECS).contains(&window.as_secs()) {
            return Err(Error::ConfigError {
                detail: format!(
                    "rate_limit.window must be between {MIN_RATE_LIMIT_WINDOW_SECS}s and {MAX_RATE_LIMIT_WINDOW_SECS}s"
                ),
            });
        }
        if !(MIN_RATE_LIMIT_MAX_ENTRIES..=MAX_RATE_LIMIT_MAX_ENTRIES).contains(&raw.max_entries) {
            return Err(Error::ConfigError {
                detail: format!(
                    "rate_limit.max_entries must be between {MIN_RATE_LIMIT_MAX_ENTRIES} and {MAX_RATE_LIMIT_MAX_ENTRIES}"
                ),
            });
        }
        if !(MIN_RATE_LIMIT_MAX_CONCURRENT_REQUESTS..=MAX_RATE_LIMIT_MAX_CONCURRENT_REQUESTS)
            .contains(&raw.max_concurrent_requests)
        {
            return Err(Error::ConfigError {
                detail: format!(
                    "rate_limit.max_concurrent_requests must be between {MIN_RATE_LIMIT_MAX_CONCURRENT_REQUESTS} and {MAX_RATE_LIMIT_MAX_CONCURRENT_REQUESTS}"
                ),
            });
        }
        for (field, budget) in [
            ("rate_limit.per_ip", raw.per_ip),
            ("rate_limit.per_ip_failures", raw.per_ip_failures),
            ("rate_limit.per_subject", raw.per_subject),
            ("rate_limit.per_provider", raw.per_provider),
        ] {
            if budget > MAX_RATE_LIMIT_BUDGET {
                return Err(Error::ConfigError {
                    detail: format!(
                        "{field} must be between 0 (disabled) and {MAX_RATE_LIMIT_BUDGET}"
                    ),
                });
            }
        }
        Ok(Self {
            enabled: raw.enabled,
            store,
            window,
            per_ip: raw.per_ip,
            per_ip_failures: raw.per_ip_failures,
            per_subject: raw.per_subject,
            per_provider: raw.per_provider,
            max_concurrent_requests: raw.max_concurrent_requests,
            max_entries: raw.max_entries,
        })
    }
}

impl SessionRepositoryConfig {
    fn resolve(raw: RawSessionRepositoryConfig) -> Result<Self, Error> {
        Ok(Self {
            adapter: raw
                .adapter
                .map(|adapter| ProviderAdapter::parse_field("session_repository.adapter", adapter))
                .transpose()?,
            valkey: raw.valkey.map(ValkeyConfig::resolve).transpose()?,
            lmdb: raw.lmdb.map(LmdbConfig::resolve).transpose()?,
            cleanup_interval: parse_positive_duration_field(
                "session_repository.cleanup_interval",
                &raw.cleanup_interval,
            )?,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawDynamoConfig {
    pub table_name: String,
    pub region: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DynamoConfig {
    pub table_name: NonEmptyString,
    pub region: Option<String>,
}

impl DynamoConfig {
    fn resolve(raw: RawDynamoConfig) -> Result<Self, Error> {
        Ok(Self {
            table_name: NonEmptyString::parse_field(
                "repository.dynamodb.table_name",
                raw.table_name,
            )?,
            region: raw.region,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawPostgresConfig {
    pub url: String,
    pub max_connections: Option<u32>,
    pub run_migrations: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct PostgresConfig {
    pub url: NonEmptyString,
    pub max_connections: Option<u32>,
    pub run_migrations: Option<bool>,
}

impl PostgresConfig {
    fn resolve(raw: RawPostgresConfig) -> Result<Self, Error> {
        Ok(Self {
            url: NonEmptyString::parse_field("repository.postgres.url", raw.url)?,
            max_connections: raw.max_connections,
            run_migrations: raw.run_migrations,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawSqliteConfig {
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct SqliteConfig {
    pub path: NonEmptyString,
}

impl SqliteConfig {
    fn resolve(raw: RawSqliteConfig) -> Result<Self, Error> {
        Ok(Self {
            path: NonEmptyString::parse_field("repository.sqlite.path", raw.path)?,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawValkeyConfig {
    pub url: String,
    pub key_prefix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ValkeyConfig {
    pub url: NonEmptyString,
    pub key_prefix: Option<String>,
}

impl ValkeyConfig {
    fn resolve(raw: RawValkeyConfig) -> Result<Self, Error> {
        Ok(Self {
            url: NonEmptyString::parse_field("session_repository.valkey.url", raw.url)?,
            key_prefix: raw.key_prefix,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawLmdbConfig {
    pub path: String,
    pub max_size_mb: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct LmdbConfig {
    pub path: NonEmptyString,
    pub max_size_mb: Option<u64>,
}

impl LmdbConfig {
    fn resolve(raw: RawLmdbConfig) -> Result<Self, Error> {
        Ok(Self {
            path: NonEmptyString::parse_field("session_repository.lmdb.path", raw.path)?,
            max_size_mb: raw.max_size_mb,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawUserSyncConfig {
    pub enabled: bool,
    pub adapter: Option<String>,
    pub webhook: Option<RawWebhookConfig>,
}

#[derive(Debug, Clone)]
pub struct UserSyncConfig {
    pub enabled: bool,
    pub adapter: Option<ProviderAdapter>,
    pub webhook: Option<WebhookConfig>,
}

impl UserSyncConfig {
    fn resolve(raw: RawUserSyncConfig) -> Result<Self, Error> {
        Ok(Self {
            enabled: raw.enabled,
            adapter: raw
                .adapter
                .map(|adapter| ProviderAdapter::parse_field("user_sync.adapter", adapter))
                .transpose()?,
            webhook: raw.webhook.map(WebhookConfig::resolve).transpose()?,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawWebhookConfig {
    pub url: String,
    pub secret: String,
    pub timeout: Option<String>,
    pub retries: Option<u32>,
}

#[derive(Clone)]
pub struct WebhookConfig {
    pub url: HttpsUrl,
    pub secret: NonEmptyString,
    pub timeout: Option<std::time::Duration>,
    pub retries: Option<u32>,
}

impl WebhookConfig {
    fn resolve(raw: RawWebhookConfig) -> Result<Self, Error> {
        Ok(Self {
            url: HttpsUrl::parse_field("user_sync.webhook.url", raw.url)?,
            secret: NonEmptyString::parse_field("user_sync.webhook.secret", raw.secret)?,
            timeout: raw
                .timeout
                .map(|timeout| parse_duration_field("user_sync.webhook.timeout", &timeout))
                .transpose()?,
            retries: raw.retries,
        })
    }

    pub fn effective_retries(&self) -> u32 {
        let configured = self.retries.unwrap_or(DEFAULT_WEBHOOK_RETRIES);
        if configured > MAX_WEBHOOK_RETRIES {
            tracing::warn!(
                configured_retries = configured,
                clamped_retries = MAX_WEBHOOK_RETRIES,
                "user_sync.webhook.retries exceeds the maximum of {MAX_WEBHOOK_RETRIES}; clamping"
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawTelemetryConfig {
    pub enabled: bool,
    pub exporter: String,
    pub endpoint: Option<String>,
    pub service_name: Option<String>,
    pub sample_rate: Option<f64>,
    pub protocol: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub exporter: TelemetryExporter,
    pub endpoint: Option<HttpsUrl>,
    pub service_name: Option<NonEmptyString>,
    pub sample_rate: Option<f64>,
    pub protocol: Option<NonEmptyString>,
}

impl TelemetryConfig {
    fn resolve(raw: RawTelemetryConfig) -> Result<Self, Error> {
        Ok(Self {
            enabled: raw.enabled,
            exporter: TelemetryExporter::parse_field("telemetry.exporter", raw.exporter)?,
            endpoint: raw
                .endpoint
                .map(|endpoint| HttpsUrl::parse_field("telemetry.endpoint", endpoint))
                .transpose()?,
            service_name: raw
                .service_name
                .map(|service_name| {
                    NonEmptyString::parse_field("telemetry.service_name", service_name)
                })
                .transpose()?,
            sample_rate: raw.sample_rate,
            protocol: raw
                .protocol
                .map(|protocol| NonEmptyString::parse_field("telemetry.protocol", protocol))
                .transpose()?,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawInternalApiConfig {
    pub enabled: bool,
    pub auth_method: Option<String>,
    pub shared_secret: Option<String>,
}

#[derive(Clone)]
pub struct InternalApiConfig {
    pub enabled: bool,
    pub auth_method: Option<InternalAuthMethod>,
    pub shared_secret: Option<NonEmptyString>,
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

impl InternalApiConfig {
    fn resolve(raw: RawInternalApiConfig) -> Result<Self, Error> {
        Ok(Self {
            enabled: raw.enabled,
            auth_method: raw
                .auth_method
                .map(|auth_method| {
                    InternalAuthMethod::parse_field("internal_api.auth_method", auth_method)
                })
                .transpose()?,
            shared_secret: raw
                .shared_secret
                .map(|shared_secret| {
                    NonEmptyString::parse_field("internal_api.shared_secret", shared_secret)
                })
                .transpose()?,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawProviderConfig {
    pub adapter: String,
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub provider_id: String,
    pub adapter: ProviderAdapter,
    pub extra: HashMap<String, toml::Value>,
    pub issuer: Option<HttpsUrl>,
    pub jwks_uri: Option<HttpsUrl>,
    pub token_endpoint: Option<HttpsUrl>,
    pub revocation_endpoint: Option<HttpsUrl>,
}

impl ProviderConfig {
    fn resolve(provider_id: String, raw: RawProviderConfig) -> Result<Self, Error> {
        let endpoint = |name: &str| {
            raw.extra
                .get(name)
                .and_then(toml::Value::as_str)
                .map(|value| {
                    HttpsUrl::parse_field(
                        &format!("providers.{provider_id}.{name}"),
                        value.to_string(),
                    )
                })
                .transpose()
        };

        let issuer = endpoint("issuer")?;
        let jwks_uri = endpoint("jwks_uri")?;
        let token_endpoint = endpoint("token_endpoint")?;
        let revocation_endpoint = endpoint("revocation_endpoint")?;

        if matches!(
            ProviderAdapter::parse_field("providers.adapter", raw.adapter.clone())?,
            ProviderAdapter::Oidc
        ) && issuer.is_none()
        {
            return Err(Error::ConfigError {
                detail: format!("providers.{provider_id}.issuer: missing required HTTPS URL"),
            });
        }

        Ok(Self {
            provider_id,
            adapter: ProviderAdapter::parse_field("providers.adapter", raw.adapter)?,
            issuer,
            jwks_uri,
            token_endpoint,
            revocation_endpoint,
            extra: raw.extra,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerRole {
    All,
    Exchange,
    Admin,
}
impl ServerRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Exchange => "exchange",
            Self::Admin => "admin",
        }
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "all" => Ok(Self::All),
            "exchange" => Ok(Self::Exchange),
            "admin" => Ok(Self::Admin),
            _ => Err(Error::ConfigError {
                detail: format!("{field}: invalid server role {value:?}"),
            }),
        }
    }
}
impl AsRef<str> for ServerRole {
    fn as_ref(&self) -> &str {
        match self {
            Self::All => "all",
            Self::Exchange => "exchange",
            Self::Admin => "admin",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationMode {
    Open,
    ExistingUsersOnly,
}
impl RegistrationMode {
    /// Construct a validated registration mode outside config deserialization, for tests.
    pub fn parse(value: impl Into<String>) -> Result<Self, Error> {
        Self::parse_field("registration.mode", value.into())
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::ExistingUsersOnly => "existing_users_only",
        }
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "open" => Ok(Self::Open),
            "existing_users_only" => Ok(Self::ExistingUsersOnly),
            _ => Err(Error::ConfigError {
                detail: format!("{field}: invalid registration mode {value:?}"),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningAlgorithm {
    EdDSA,
    RS256,
    RS384,
    RS512,
    PS256,
    PS384,
    PS512,
    ES256,
    ES384,
    ES512,
}
impl SigningAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EdDSA => "EdDSA",
            Self::RS256 => "RS256",
            Self::RS384 => "RS384",
            Self::RS512 => "RS512",
            Self::PS256 => "PS256",
            Self::PS384 => "PS384",
            Self::PS512 => "PS512",
            Self::ES256 => "ES256",
            Self::ES384 => "ES384",
            Self::ES512 => "ES512",
        }
    }

    fn parse_local_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "EdDSA" => Ok(Self::EdDSA),
            _ => Err(Error::ConfigError {
                detail: format!("{field}: local Ed25519 keys require signing algorithm \"EdDSA\", got {value:?}"),
            }),
        }
    }

    fn parse_kms_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "RS256" => Ok(Self::RS256),
            "RS384" => Ok(Self::RS384),
            "RS512" => Ok(Self::RS512),
            "PS256" => Ok(Self::PS256),
            "PS384" => Ok(Self::PS384),
            "PS512" => Ok(Self::PS512),
            "ES256" => Ok(Self::ES256),
            "ES384" => Ok(Self::ES384),
            "ES512" => Ok(Self::ES512),
            _ => Err(Error::ConfigError {
                detail: format!(
                    "{field}: invalid KMS JWS signing algorithm {value:?}; expected RS256, RS384, RS512, PS256, PS384, PS512, ES256, ES384, or ES512"
                ),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditAdapter {
    Noop,
    Stdout,
    Stderr,
    /// `stdout` under a detected Lambda runtime, `stderr` elsewhere.
    Auto,
    Sqs,
}
impl AuditAdapter {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Auto => "auto",
            Self::Sqs => "sqs",
        }
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "noop" => Ok(Self::Noop),
            "stdout" => Ok(Self::Stdout),
            "stderr" => Ok(Self::Stderr),
            "auto" => Ok(Self::Auto),
            "sqs" => Ok(Self::Sqs),
            _ => Err(Error::ConfigError {
                detail: format!("{field}: invalid audit adapter {value:?}"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelemetryExporter {
    None,
    Otlp,
    Stdout,
    Prometheus,
}
impl TelemetryExporter {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Otlp => "otlp",
            Self::Stdout => "stdout",
            Self::Prometheus => "prometheus",
        }
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "none" => Ok(Self::None),
            "otlp" => Ok(Self::Otlp),
            "stdout" => Ok(Self::Stdout),
            "prometheus" => Ok(Self::Prometheus),
            _ => Err(Error::ConfigError {
                detail: format!("{field}: invalid telemetry exporter {value:?}"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InternalAuthMethod {
    SharedSecret,
    Oidc,
}
impl InternalAuthMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SharedSecret => "shared_secret",
            Self::Oidc => "oidc",
        }
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "shared_secret" => Ok(Self::SharedSecret),
            "oidc" => Ok(Self::Oidc),
            _ => Err(Error::ConfigError {
                detail: format!("{field}: invalid internal auth method {value:?}"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAdapter {
    Oidc,
    Local,
    Kms,
    Sqlite,
    Dynamodb,
    Postgres,
    Valkey,
    Lmdb,
    Webhook,
}
impl ProviderAdapter {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Oidc => "oidc",
            Self::Local => "local",
            Self::Kms => "kms",
            Self::Sqlite => "sqlite",
            Self::Dynamodb => "dynamodb",
            Self::Postgres => "postgres",
            Self::Valkey => "valkey",
            Self::Lmdb => "lmdb",
            Self::Webhook => "webhook",
        }
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "oidc" => Ok(Self::Oidc),
            "local" => Ok(Self::Local),
            "kms" => Ok(Self::Kms),
            "sqlite" => Ok(Self::Sqlite),
            "dynamodb" => Ok(Self::Dynamodb),
            "postgres" => Ok(Self::Postgres),
            "valkey" => Ok(Self::Valkey),
            "lmdb" => Ok(Self::Lmdb),
            "webhook" => Ok(Self::Webhook),
            _ => Err(Error::ConfigError {
                detail: format!("{field}: invalid provider adapter {value:?}"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsUrl(String);
impl HttpsUrl {
    /// Construct a validated HTTPS URL outside config deserialization.
    pub fn parse(value: impl Into<String>) -> Result<Self, Error> {
        Self::parse_field("URL", value.into())
    }

    /// Construct an HTTP URL for test fixtures only.
    ///
    /// This API is an explicit fixture seam; production configuration and endpoint parsing use
    /// [`Self::parse`] exclusively.
    #[doc(hidden)]
    pub fn parse_for_test(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.starts_with("http://") && trimmed.len() > "http://".len() {
            Ok(Self(trimmed.to_string()))
        } else {
            Self::parse(value)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        let trimmed = value.trim();
        if trimmed.starts_with("https://") && trimmed.len() > "https://".len() {
            Ok(Self(trimmed.to_string()))
        } else {
            Err(Error::ConfigError {
                detail: format!("{field}: expected non-empty HTTPS URL, got {value:?}"),
            })
        }
    }
}
impl AsRef<str> for HttpsUrl {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsciiDomainPattern(String);
impl AsciiDomainPattern {
    /// Construct a validated domain pattern outside config deserialization, for tests.
    pub fn parse(value: impl Into<String>) -> Result<Self, Error> {
        Self::parse_field("domain pattern", value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        let trimmed = value.trim();
        let candidate = trimmed.strip_prefix("*.").unwrap_or(trimmed);
        if !trimmed.is_empty()
            && trimmed.is_ascii()
            && candidate.contains('.')
            && !trimmed.contains('/')
        {
            Ok(Self(trimmed.to_string()))
        } else {
            Err(Error::ConfigError {
                detail: format!("{field}: invalid allowlist entry {value:?}"),
            })
        }
    }
}
impl AsRef<str> for AsciiDomainPattern {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmptyString(String);
impl NonEmptyString {
    /// Construct a validated non-empty value outside config deserialization, for tests.
    pub fn parse(value: impl Into<String>) -> Result<Self, Error> {
        Self::parse_field("value", value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        if value.trim().is_empty() {
            Err(Error::ConfigError {
                detail: format!("{field}: must be non-empty"),
            })
        } else {
            Ok(Self(value))
        }
    }
    fn into_inner(self) -> String {
        self.0
    }
}
impl AsRef<str> for NonEmptyString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// [`parse_duration_field`] plus a strictly-positive requirement. Zero is
/// rejected because a zero-width window or interval can never do its job (a
/// zero grace window makes every lost response a reuse alarm; a zero cleanup
/// interval would spin); negatives cannot be expressed through the unsigned
/// parser, so only zero needs an explicit check.
fn parse_positive_duration_field(
    field: &str,
    value: &str,
) -> Result<std::time::Duration, Error> {
    let duration = parse_duration_field(field, value)?;
    if duration.as_secs() == 0 {
        return Err(Error::ConfigError {
            detail: format!("{field}: {value:?} must be greater than zero"),
        });
    }
    Ok(duration)
}

/// [`parse_positive_duration_field`] plus an inclusive upper bound in seconds.
/// The grace window is a deliberate weakening of rotation (an
/// immediately-preceding generation stays redeemable), so an unbounded value
/// is indistinguishable from no rotation and is rejected at load rather than
/// trusted.
fn parse_positive_duration_field_capped(
    field: &str,
    value: &str,
    max_secs: u64,
) -> Result<std::time::Duration, Error> {
    let duration = parse_positive_duration_field(field, value)?;
    if duration.as_secs() > max_secs {
        return Err(Error::ConfigError {
            detail: format!("{field}: {value:?} exceeds the maximum of {max_secs}s"),
        });
    }
    Ok(duration)
}

fn parse_duration_field(field: &str, value: &str) -> Result<std::time::Duration, Error> {
    let secs = crate::service::parse_duration_secs(value).map_err(|err| match err {
        Error::ConfigError { detail } => Error::ConfigError {
            detail: format!("{field}: {detail}"),
        },
        other => other,
    })?;
    Ok(std::time::Duration::from_secs(secs))
}

pub const MAX_WEBHOOK_RETRIES: u32 = 10;
const DEFAULT_WEBHOOK_RETRIES: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    fn default_raw_config() -> RawConfig {
        toml::from_str(include_str!("../../../config/default.toml"))
            .expect("default config should deserialize")
    }

    fn assert_config_error(result: Result<Config, Error>, field: &str) {
        let Error::ConfigError { detail } = result.expect_err("config should be rejected") else {
            unreachable!("expected ConfigError");
        };
        assert!(
            detail.contains(field),
            "error {detail:?} should name {field:?}"
        );
    }

    #[test]
    fn https_url_parse_rejects_non_https_in_production_api() {
        let err = HttpsUrl::parse("http://provider.example/token")
            .expect_err("production URL constructor must reject HTTP");
        assert!(matches!(err, Error::ConfigError { .. }));
    }

    #[test]
    fn resolve_default_toml() {
        let config = Config::resolve(default_raw_config()).expect("failed to resolve config");
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.issuer.as_ref(), "https://auth.example.com");
        assert_eq!(config.server.request_timeout.as_secs(), 30);
        assert_eq!(config.registration.mode, RegistrationMode::Open);
        assert!(config.registration.domain_allowlist.is_none());
        assert_eq!(config.token.access_token_ttl.as_secs(), 15 * 60);
        assert_eq!(config.token.refresh_token_ttl.as_secs(), 30 * 24 * 60 * 60);
        assert_eq!(config.token.audience.as_ref(), "https://api.example.com");
        // Grants defaults: the direct ID-token grant ships disabled, with the documented
        // replay-protection durations, even though default.toml carries no `[grants]`.
        assert!(!config.grants.id_token);
        assert_eq!(config.grants.nonce_ttl.as_secs(), 10 * 60);
        assert_eq!(config.grants.max_assertion_lifetime.as_secs(), 60 * 60);
        assert!(config.providers.is_empty());
    }

    #[test]
    fn resolve_accepts_representative_closed_config_values() {
        let cases = [
            ("server.role", "admin"),
            ("registration.mode", "existing_users_only"),
            ("key_manager.adapter", "local"),
            ("repository.adapter", "sqlite"),
            ("audit.adapter", "sqs"),
            ("telemetry.exporter", "otlp"),
        ];

        for (field, value) in cases {
            let mut raw = default_raw_config();
            match field {
                "server.role" => raw.server.role = value.into(),
                "registration.mode" => raw.registration.mode = value.into(),
                "key_manager.adapter" => raw.key_manager.adapter = value.into(),
                "repository.adapter" => raw.repository.adapter = value.into(),
                "audit.adapter" => raw.audit.adapter = value.into(),
                "telemetry.exporter" => raw.telemetry.exporter = value.into(),
                _ => unreachable!("test case field is known"),
            }
            Config::resolve(raw)
                .unwrap_or_else(|err| panic!("{field}={value:?} should resolve: {err}"));
        }
    }

    #[test]
    fn resolve_rejects_non_jws_and_wrong_adapter_signing_algorithms_at_load() {
        let cases = [
            ("key_manager.kms.algorithm", "ECDSA_SHA_256"),
            ("key_manager.kms.algorithm", "ECDSA_SHA256"),
            ("key_manager.kms.algorithm", "EdDSA"),
            ("key_manager.local.algorithm", "ES256"),
            ("key_manager.local.algorithm", "RS256"),
        ];

        for (field, value) in cases {
            let mut raw = default_raw_config();
            if field == "key_manager.kms.algorithm" {
                raw.key_manager.kms = Some(RawKmsConfig {
                    key_id: "key-id".into(),
                    algorithm: value.into(),
                    kid: "kid".into(),
                });
            } else {
                raw.key_manager.local = Some(RawLocalKeyConfig {
                    private_key_path: "key.pem".into(),
                    algorithm: value.into(),
                    kid: "kid".into(),
                });
            }
            assert_config_error(Config::resolve(raw), field);
        }
    }

    #[test]
    fn resolve_accepts_all_kms_jws_algorithms() {
        for algorithm in [
            "RS256", "RS384", "RS512", "PS256", "PS384", "PS512", "ES256", "ES384", "ES512",
        ] {
            let mut raw = default_raw_config();
            raw.key_manager.kms = Some(RawKmsConfig {
                key_id: "key-id".into(),
                algorithm: algorithm.into(),
                kid: "kid".into(),
            });
            let config = Config::resolve(raw)
                .unwrap_or_else(|err| panic!("KMS algorithm {algorithm} should resolve: {err}"));
            assert_eq!(
                config
                    .key_manager
                    .kms
                    .expect("KMS config present")
                    .algorithm
                    .as_str(),
                algorithm
            );
        }
    }

    #[test]
    fn resolve_rejects_invalid_closed_config_values_with_field_names() {
        let cases = [
            ("server.role", "unknown"),
            ("registration.mode", "existing_users"),
            ("key_manager.adapter", ""),
            ("repository.adapter", ""),
            ("audit.adapter", "syslog"),
            ("telemetry.exporter", "xray"),
        ];

        for (field, value) in cases {
            let mut raw = default_raw_config();
            match field {
                "server.role" => raw.server.role = value.into(),
                "registration.mode" => raw.registration.mode = value.into(),
                "key_manager.adapter" => raw.key_manager.adapter = value.into(),
                "repository.adapter" => raw.repository.adapter = value.into(),
                "audit.adapter" => raw.audit.adapter = value.into(),
                "telemetry.exporter" => raw.telemetry.exporter = value.into(),
                _ => unreachable!("test case field is known"),
            }
            assert_config_error(Config::resolve(raw), field);
        }
    }

    /// `[grants]` deserializes to its explicit values when present; every key is
    /// optional, so an operator can override just one duration.
    #[test]
    fn grants_section_resolves_explicit_values() {
        let parsed: RawConfig = toml::from_str(
            r#"
[grants]
id_token = true
nonce_ttl = "5m"
max_assertion_lifetime = "30m"
"#,
        )
        .expect("explicit [grants] must deserialize");
        let mut raw = default_raw_config();
        raw.grants = parsed.grants;
        let explicit = Config::resolve(raw).expect("explicit [grants] must resolve");

        assert!(explicit.grants.id_token);
        assert_eq!(explicit.grants.nonce_ttl.as_secs(), 5 * 60);
        assert_eq!(explicit.grants.max_assertion_lifetime.as_secs(), 30 * 60);
    }

    /// Omitting `[grants]` entirely must land on the safe compiled defaults: direct
    /// ID-token service off, and the documented `10m` / `1h` durations.
    #[test]
    fn omitted_grants_section_uses_disabled_direct_grant_defaults() {
        let config = Config::resolve(default_raw_config()).expect("default config resolves");

        assert!(
            !config.grants.id_token,
            "the direct ID-token grant must default to disabled"
        );
        assert_eq!(config.grants.nonce_ttl.as_secs(), 10 * 60);
        assert_eq!(config.grants.max_assertion_lifetime.as_secs(), 60 * 60);

        // Negative-space on deserialization: an empty TOML document (no sections at all)
        // must still parse — serde defaults everywhere, per 06-configuration.md.
        let empty: RawConfig =
            toml::from_str("").expect("an empty config document must deserialize");
        assert!(!empty.grants.id_token);
    }

    /// An unparseable `grants.nonce_ttl` must fail resolution naming the exact field,
    /// not be absorbed until some later request reads it.
    #[test]
    fn resolve_rejects_unparseable_nonce_ttl() {
        let mut raw = default_raw_config();
        raw.grants.nonce_ttl = "not-a-duration".to_string();

        let err = Config::resolve(raw).expect_err("bad nonce_ttl must be rejected");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("grants.nonce_ttl"),
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

    /// An unparseable `grants.max_assertion_lifetime` fails resolution the same way:
    /// precise field name, echoed bad value.
    #[test]
    fn resolve_rejects_unparseable_max_assertion_lifetime() {
        let mut raw = default_raw_config();
        raw.grants.max_assertion_lifetime = "forever".to_string();

        let err = Config::resolve(raw).expect_err("bad max_assertion_lifetime must be rejected");

        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("grants.max_assertion_lifetime"),
                    "detail must name the field: {detail}"
                );
                assert!(
                    detail.contains("forever"),
                    "detail must echo the bad value: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    /// Positive-space boundary: valid custom durations (including a zero-length nonce
    /// TTL) pass resolution, and an enabled switch resolves too — only *unparseable*
    /// durations fail closed.
    #[test]
    fn resolve_accepts_valid_grant_durations_and_enabled_switch() {
        let mut enabled = default_raw_config();
        enabled.grants.id_token = true;
        enabled.grants.nonce_ttl = "90s".to_string();
        enabled.grants.max_assertion_lifetime = "2h".to_string();
        let resolved = Config::resolve(enabled)
            .expect("valid custom durations with the grant enabled must pass");
        assert!(resolved.grants.id_token);
        assert_eq!(resolved.grants.nonce_ttl.as_secs(), 90);

        // At-the-boundary value: "0s" parses fine as a duration (a zero-length claim
        // window is operationally useless but not malformed), so resolution accepts it;
        // policy enforcement belongs above the config layer.
        let mut zero_ttl = default_raw_config();
        zero_ttl.grants.nonce_ttl = "0s".to_string();
        assert!(
            Config::resolve(zero_ttl).is_ok(),
            "a parseable zero duration must not fail config load"
        );
    }
}
