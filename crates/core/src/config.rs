use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::domain::AuditSeverity;
use crate::error::Error;
use crate::secret::Secret;

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
        // Reserved protocol claim names are refused at startup: a template
        // claim keyed by a reserved name would be silently dropped at token
        // build, so the misconfiguration surfaces at load instead.
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

        // When the internal API will be served, the whole `[internal_api]`
        // contract applies: a non-empty mechanism list and per-mechanism
        // requirements (the shared secret's length floor; a real key manager
        // for operator tokens). See `06-configuration.md` → Validation at load.
        let internal_api_served = matches!(self.server.role, ServerRole::Admin | ServerRole::All)
            && self.internal_api.enabled;
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

        // The mtls mechanism trusts a header asserted by whatever terminates
        // TLS in front of the admin listener. On the default loopback bind
        // that trust is anchored by reachability; published on a routable
        // interface it becomes a silent identity-spoofing surface unless the
        // trusted proxy both sets and strips the header. Warn so the
        // deployment cannot enable the pairing unknowingly.
        if internal_api_served
            && self.internal_api.uses_mtls()
            && !admin_listener_is_loopback(&self.internal_api.host)
        {
            tracing::warn!(
                host = %self.internal_api.host,
                port = self.internal_api.port,
                subject_header = %self.internal_api.mtls_subject_header(),
                "the mtls mechanism trusts the client-certificate subject header on an \
                 admin listener bound beyond loopback; any host that can reach this \
                 listener and set the header authenticates as anyone - ensure your \
                 TLS-terminating proxy overwrites the header on every request and the \
                 listener is otherwise unreachable"
            );
        }

        // Only role = "all" binds both sockets, so only that role can collide.
        // Under role = "admin" the public socket is never bound (same values
        // are harmless), and under any other role the admin listener is not
        // bound at all.
        let binds_both_listeners =
            self.server.role == ServerRole::All && self.internal_api.enabled;
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
    fn validate_internal_api(&self) -> Result<(), Error> {
        if self.internal_api.auth_methods.is_empty() {
            return Err(Error::ConfigError {
                detail: "internal_api.auth_methods must be non-empty when the internal API \
                         is served"
                    .to_string(),
            });
        }

        if self.internal_api.uses_shared_secret() {
            let secret_len = self
                .internal_api
                .shared_secret
                .as_ref()
                .map(|secret| secret.expose().len())
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
            // The typed `server.issuer` is non-empty by construction; what can
            // still be wrong is the key manager: operator tokens are verified
            // against this service's own keys, so a keyless process cannot
            // serve the mechanism.
            if !matches!(
                self.key_manager.adapter,
                ProviderAdapter::Local | ProviderAdapter::Kms
            ) {
                return Err(Error::ConfigError {
                    detail: format!(
                        "key_manager.adapter ({:?}) cannot serve the operator_token \
                         mechanism: token verification requires a real key manager",
                        self.key_manager.adapter.as_str()
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
            if self.token.audience.as_ref() == self.internal_api.token_audience {
                // The internal audience is the one structural replay defense
                // between user access tokens and operator credentials, which
                // share this service's issuer and key manager. Defaults
                // differ, so equality can only arise through deliberate
                // misconfiguration — refuse it at load.
                return Err(Error::ConfigError {
                    detail: format!(
                        "internal_api.token_audience ({:?}) must differ from \
                         token.audience: a shared audience lets any user access token minted \
                         by this service's key manager be replayed as an operator \
                         credential",
                        self.internal_api.token_audience
                    ),
                });
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

        if self.internal_api.max_auth_failures == 0 {
            return Err(Error::ConfigError {
                detail: "internal_api.max_auth_failures must be non-zero".to_string(),
            });
        }

        Ok(())
    }
}

/// Default request-body ceiling shared by native and embedded hosts: 2 MiB.
pub const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;

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
/// invents path structure the operator did not write; resolution rejects it
/// instead, naming the field.
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RawServerConfig {
    pub host: String,
    pub port: u16,
    pub issuer: String,
    pub role: String,
    pub request_timeout: String,
    /// Maximum request body accepted by every host before buffering.
    pub max_request_body_bytes: usize,
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
            max_request_body_bytes: DEFAULT_MAX_REQUEST_BODY_BYTES,
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
    /// Maximum request body accepted by every host before buffering.
    pub max_request_body_bytes: usize,
    /// Path prefix (e.g. `"/prod"`) stripped from incoming request paths
    /// before routing. Canonical by construction: resolution folds `""`/`"/"`
    /// into `None`, trims one trailing `/`, and rejects a value with no
    /// leading `/` — the strip middleware never re-derives these cases on the
    /// per-request path.
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
            // Fail-closed default: a config that never names a role serves
            // only the exchange plane; the admin plane is an explicit act.
            role: if raw.role.is_empty() {
                ServerRole::Exchange
            } else {
                ServerRole::parse_field("server.role", raw.role)?
            },
            request_timeout: parse_duration_field("server.request_timeout", &raw.request_timeout)?,
            max_request_body_bytes: raw.max_request_body_bytes,
            base_path: {
                let base_path = normalise_base_path(raw.base_path);
                if let Some(base_path) = &base_path {
                    let is_canonical = base_path.len() > 1
                        && base_path.starts_with('/')
                        && !base_path.ends_with('/');
                    if !is_canonical {
                        return Err(Error::ConfigError {
                            detail: format!(
                                "server.base_path {base_path:?} must start with '/' and carry \
                                 at least one non-slash character (\"\" and \"/\" mean unset, a \
                                 trailing \"/\" is trimmed at load)"
                            ),
                        });
                    }
                }
                base_path
            },
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
    /// HMAC key for outbound webhook signatures: validated non-empty at load
    /// and wrapped so the configured value cannot be formatted.
    pub secret: Secret<String>,
    pub timeout: Option<std::time::Duration>,
    pub retries: Option<u32>,
}

impl WebhookConfig {
    fn resolve(raw: RawWebhookConfig) -> Result<Self, Error> {
        Ok(Self {
            url: HttpsUrl::parse_field("user_sync.webhook.url", raw.url)?,
            secret: Secret::new(
                NonEmptyString::parse_field("user_sync.webhook.secret", raw.secret)?
                    .as_str()
                    .to_string(),
            ),
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

/// Default for `[internal_api] host` when the key is absent from config: the
/// admin listener is reachable only from an operator network, so it binds the
/// loopback interface unless an operator explicitly publishes it.
pub const DEFAULT_INTERNAL_API_HOST: &str = "127.0.0.1";

/// Default for `[internal_api] port` when the key is absent from config: one
/// above the public listener's 8080, so the two planes never collide by
/// accident.
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
    if public_host == admin_host {
        return true;
    }
    WILDCARD_LISTENER_HOSTS.contains(&public_host) || WILDCARD_LISTENER_HOSTS.contains(&admin_host)
}

/// Whether the admin listener host is a loopback bind — the deployment shape
/// under which trusting a proxy-asserted mTLS-subject header is anchored by
/// reachability rather than faith.
fn admin_listener_is_loopback(host: &str) -> bool {
    match host.trim_start_matches('[').trim_end_matches(']').parse::<std::net::IpAddr>() {
        Ok(addr) => addr.is_loopback(),
        Err(_) => host == "localhost",
    }
}

/// Minimum byte length for `internal_api.shared_secret` whenever the
/// shared-secret mechanism serves the internal API: 32 bytes (256 bits of
/// operator-chosen material), the floor below which offline guessing of the
/// constant-time comparison becomes a realistic project.
pub const MIN_SHARED_SECRET_BYTES: usize = 32;

/// Default audience a verified operator token must carry.
pub const DEFAULT_TOKEN_AUDIENCE: &str = "internal";

/// Default claim name a verified operator token must carry.
pub const DEFAULT_REQUIRED_CLAIM: &str = "role";

/// Default value [`DEFAULT_REQUIRED_CLAIM`] must carry on a verified operator token.
pub const DEFAULT_REQUIRED_VALUE: &str = "admin";

/// Default header the `mtls` mechanism reads the client-certificate subject from.
pub const DEFAULT_MTLS_SUBJECT_HEADER: &str = "x-client-cert-subject";

/// Default failed-authentication budget per peer before lockout.
pub const DEFAULT_MAX_AUTH_FAILURES: u64 = 5;

/// Default window over which failed operator authentications draw down the budget.
pub const DEFAULT_AUTH_FAILURE_WINDOW: &str = "1m";

/// Default lockout duration once the operator-auth failure budget is exhausted.
pub const DEFAULT_AUTH_LOCKOUT: &str = "5m";

/// Default TTL for the admin stats cache.
pub const DEFAULT_STATS_CACHE_TTL: &str = "60s";

/// Bounds for `internal_api.stats_cache_ttl`: at least 1s (a zero TTL turns
/// the cache into a per-request stampede) and at most one hour (stale stats
/// beyond that mislead more than they serve).
pub const MIN_STATS_CACHE_TTL_SECS: u64 = 1;
pub const MAX_STATS_CACHE_TTL_SECS: u64 = 3600;

/// Accept `auth_methods = ["a", "b"]` and the pre-hardening singular
/// `auth_method = "a"` (via the serde alias) as the same field: a bare string
/// is read as a one-element list.
fn string_or_seq_string<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct StringOrSeqVisitor;

    impl<'de> serde::de::Visitor<'de> for StringOrSeqVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a string or a list of strings")
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
            let mut values = Vec::new();
            while let Some(value) = seq.next_element::<String>()? {
                values.push(value);
            }
            Ok(values)
        }
    }

    deserializer.deserialize_any(StringOrSeqVisitor)
}

/// Configuration for the `mtls` mechanism: the proxy header carrying the
/// client-certificate subject.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct MtlsConfig {
    pub subject_header: String,
}

impl Default for MtlsConfig {
    fn default() -> Self {
        Self {
            subject_header: DEFAULT_MTLS_SUBJECT_HEADER.to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RawInternalApiConfig {
    pub enabled: bool,
    /// Host for the dedicated admin listener.
    pub host: String,
    /// Port for the dedicated admin listener.
    pub port: u16,
    /// Enabled authentication mechanisms, tried in the order given. The
    /// singular `auth_method = "..."` key from pre-hardening deployments is
    /// still accepted and read as a one-element list (the `alias`); both
    /// spellings in one file fail the load as a duplicate field rather than
    /// silently picking one.
    #[serde(
        default,
        alias = "auth_method",
        deserialize_with = "string_or_seq_string"
    )]
    pub auth_methods: Vec<String>,
    pub shared_secret: Option<String>,
    /// Audience a verified operator token must carry.
    pub token_audience: String,
    /// Claim name a verified operator token must carry.
    pub required_claim: String,
    /// Value `required_claim` must carry on a verified operator token.
    pub required_value: String,
    pub mtls: Option<MtlsConfig>,
    /// Failed-authentication budget per peer before lockout.
    pub max_auth_failures: u64,
    /// Window over which failed attempts draw down the budget (humantime).
    pub auth_failure_window: String,
    /// Lockout duration once the failure budget is exhausted (humantime).
    pub auth_lockout: String,
    /// TTL for the admin stats cache (humantime).
    pub stats_cache_ttl: String,
}

impl Default for RawInternalApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: DEFAULT_INTERNAL_API_HOST.to_string(),
            port: DEFAULT_INTERNAL_API_PORT,
            auth_methods: Vec::new(),
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

#[derive(Clone)]
pub struct InternalApiConfig {
    pub enabled: bool,
    /// Host for the dedicated admin listener. Defaults to
    /// [`DEFAULT_INTERNAL_API_HOST`] so publishing the admin plane is an
    /// explicit configuration act.
    pub host: String,
    /// Port for the dedicated admin listener. Defaults to
    /// [`DEFAULT_INTERNAL_API_PORT`]. `Config::resolve` rejects a
    /// role = "all" config whose admin listener collides with the public
    /// socket.
    pub port: u16,
    /// Enabled authentication mechanisms, tried in the order given. Empty is
    /// rejected at load whenever the internal API is served: a served admin
    /// plane with no way in would answer every request with `not_configured`
    /// forever.
    pub auth_methods: Vec<InternalAuthMethod>,
    /// Compatibility shared secret for the `shared_secret` mechanism;
    /// redacted in `Debug` and wrapped so the configured value cannot be
    /// formatted. Required (at [`MIN_SHARED_SECRET_BYTES`] bytes) whenever
    /// that mechanism is enabled and the internal API is served.
    pub shared_secret: Option<Secret<String>>,
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
    /// Window over which failed attempts draw down the budget.
    pub auth_failure_window: std::time::Duration,
    /// Lockout duration once the failure budget is exhausted.
    pub auth_lockout: std::time::Duration,
    /// TTL for the admin stats cache. Bounded to
    /// `MIN_STATS_CACHE_TTL_SECS..=MAX_STATS_CACHE_TTL_SECS` at load.
    pub stats_cache_ttl: std::time::Duration,
}

impl std::fmt::Debug for InternalApiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
            .field("required_value", &self.required_value)
            .field("mtls", &self.mtls)
            .field("max_auth_failures", &self.max_auth_failures)
            .field("auth_failure_window", &self.auth_failure_window)
            .field("auth_lockout", &self.auth_lockout)
            .field("stats_cache_ttl", &self.stats_cache_ttl)
            .finish()
    }
}

impl InternalApiConfig {
    fn resolve(raw: RawInternalApiConfig) -> Result<Self, Error> {
        let mut auth_methods = Vec::with_capacity(raw.auth_methods.len());
        for method in raw.auth_methods {
            let parsed = InternalAuthMethod::parse_field("internal_api.auth_methods", method)?;
            if auth_methods.contains(&parsed) {
                return Err(Error::ConfigError {
                    detail: format!(
                        "internal_api.auth_methods lists {:?} more than once",
                        parsed.as_str()
                    ),
                });
            }
            auth_methods.push(parsed);
        }

        let stats_cache_ttl =
            parse_positive_duration_field("internal_api.stats_cache_ttl", &raw.stats_cache_ttl)?;
        let stats_cache_secs = stats_cache_ttl.as_secs();
        if stats_cache_secs < MIN_STATS_CACHE_TTL_SECS {
            return Err(Error::ConfigError {
                detail: format!(
                    "internal_api.stats_cache_ttl must be at least                      {MIN_STATS_CACHE_TTL_SECS}s (got {stats_cache_secs}s)"
                ),
            });
        }
        if stats_cache_secs > MAX_STATS_CACHE_TTL_SECS {
            // The bound is stated, never any cached value.
            return Err(Error::ConfigError {
                detail: format!(
                    "internal_api.stats_cache_ttl exceeds the maximum of                      {MAX_STATS_CACHE_TTL_SECS}s (got {stats_cache_secs}s)"
                ),
            });
        }

        Ok(Self {
            enabled: raw.enabled,
            host: raw.host,
            port: raw.port,
            auth_methods,
            shared_secret: raw
                .shared_secret
                .map(|shared_secret| {
                    NonEmptyString::parse_field("internal_api.shared_secret", shared_secret)
                        .map(|secret| Secret::new(secret.as_str().to_string()))
                })
                .transpose()?,
            token_audience: raw.token_audience,
            required_claim: raw.required_claim,
            required_value: raw.required_value,
            mtls: raw.mtls,
            max_auth_failures: raw.max_auth_failures,
            auth_failure_window: parse_positive_duration_field(
                "internal_api.auth_failure_window",
                &raw.auth_failure_window,
            )?,
            auth_lockout: parse_positive_duration_field(
                "internal_api.auth_lockout",
                &raw.auth_lockout,
            )?,
            stats_cache_ttl,
        })
    }

    /// Whether the unattributed shared-secret compatibility mechanism is enabled.
    pub fn uses_shared_secret(&self) -> bool {
        self.auth_methods
            .contains(&InternalAuthMethod::SharedSecret)
    }

    /// Whether the named-principal operator-token mechanism is enabled.
    pub fn uses_operator_token(&self) -> bool {
        self.auth_methods
            .contains(&InternalAuthMethod::OperatorToken)
    }

    /// Whether the proxy-asserted mTLS-subject mechanism is enabled.
    pub fn uses_mtls(&self) -> bool {
        self.auth_methods.contains(&InternalAuthMethod::Mtls)
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
    /// against the default.
    pub fn mtls_subject_header(&self) -> &str {
        match &self.mtls {
            Some(cfg) => cfg.subject_header.as_str(),
            None => DEFAULT_MTLS_SUBJECT_HEADER,
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Xray,
    Prometheus,
}
impl TelemetryExporter {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Otlp => "otlp",
            Self::Stdout => "stdout",
            Self::Xray => "xray",
            Self::Prometheus => "prometheus",
        }
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "none" => Ok(Self::None),
            "otlp" => Ok(Self::Otlp),
            "stdout" => Ok(Self::Stdout),
            "xray" => Ok(Self::Xray),
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
    OperatorToken,
    Mtls,
}
impl InternalAuthMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SharedSecret => "shared_secret",
            Self::Oidc => "oidc",
            Self::OperatorToken => "operator_token",
            Self::Mtls => "mtls",
        }
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "shared_secret" => Ok(Self::SharedSecret),
            "oidc" => Ok(Self::Oidc),
            "operator_token" => Ok(Self::OperatorToken),
            "mtls" => Ok(Self::Mtls),
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
    fn provider_endpoint_origins_survive_resolve_as_a_string_array_in_extra() {
        // The declared endpoint origins survive resolution as a string array in
        // `extra` (the typed lift and strict per-entry validation happen in the
        // server's `provider_config_to_oidc`).
        let mut raw = default_raw_config();
        let google: RawProviderConfig = toml::from_str(
            r#"
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "google-client-id"
endpoint_origins = ["https://oauth2.googleapis.com", "https://www.googleapis.com"]
"#,
        )
        .expect("provider fixture deserializes");
        raw.providers.insert("google".to_string(), google);

        let config = Config::resolve(raw).expect("config with endpoint_origins resolves");
        let origins = config.providers["google"]
            .extra
            .get("endpoint_origins")
            .and_then(|v| v.as_array())
            .expect("endpoint_origins must parse as an array");
        assert_eq!(origins.len(), 2);
        assert_eq!(origins[0].as_str(), Some("https://oauth2.googleapis.com"));
        assert_eq!(origins[1].as_str(), Some("https://www.googleapis.com"));
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
            ("telemetry.exporter", "zipkin"),
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

#[cfg(test)]
mod admin_plane_config_tests {
    use super::*;

    /// 32 bytes exactly: the documented shared-secret floor.
    const TEST_SHARED_SECRET: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn test_shared_secret_constant_meets_the_documented_floor() {
        assert_eq!(TEST_SHARED_SECRET.len(), MIN_SHARED_SECRET_BYTES);
    }

    fn default_raw_config() -> RawConfig {
        toml::from_str(include_str!("../../../config/default.toml"))
            .expect("default config should deserialize")
    }

    /// A raw config that serves the internal API under the given role and
    /// mechanisms, with the given shared secret (when provided).
    fn served_raw(role: &str, auth_methods: &[&str], shared_secret: Option<&str>) -> RawConfig {
        let mut raw = default_raw_config();
        raw.server.role = role.to_string();
        raw.internal_api.enabled = true;
        raw.internal_api.auth_methods = auth_methods.iter().map(|s| s.to_string()).collect();
        raw.internal_api.shared_secret = shared_secret.map(str::to_string);
        raw
    }

    fn assert_rejected(raw: RawConfig, fragment: &str) {
        let Error::ConfigError { detail } =
            Config::resolve(raw).expect_err("config should be rejected")
        else {
            unreachable!("expected ConfigError");
        };
        assert!(
            detail.contains(fragment),
            "error {detail:?} should mention {fragment:?}"
        );
    }

    // ── exchange-only default (task 02) ─────────────────────────────────

    #[test]
    fn server_role_absent_resolves_to_exchange_default() {
        let mut raw = default_raw_config();
        raw.server.role = String::new();
        let config = Config::resolve(raw).expect("absent role resolves");
        assert_eq!(config.server.role, ServerRole::Exchange);
    }

    #[test]
    fn explicit_all_and_admin_roles_are_preserved() {
        for (value, expected) in [("all", ServerRole::All), ("admin", ServerRole::Admin)] {
            let mut raw = default_raw_config();
            raw.server.role = value.to_string();
            let config = Config::resolve(raw).expect("explicit role resolves");
            assert_eq!(config.server.role, expected);
        }
    }

    #[test]
    fn default_role_with_enabled_internal_api_is_not_served() {
        // The exchange-only default never binds the admin plane, so enabling
        // the flag alone must not trigger the served-plane validation (no
        // mechanisms and no secret are configured here).
        let mut raw = default_raw_config();
        raw.server.role = String::new();
        raw.internal_api.enabled = true;
        let config = Config::resolve(raw).expect("unserved internal API resolves");
        assert_eq!(config.server.role, ServerRole::Exchange);
        assert!(config.internal_api.enabled);
    }

    // ── mechanism list (task 03) ────────────────────────────────────────

    #[test]
    fn served_internal_api_with_no_mechanisms_is_rejected() {
        assert_rejected(
            served_raw("admin", &[], Some(TEST_SHARED_SECRET)),
            "internal_api.auth_methods must be non-empty",
        );
    }

    #[test]
    fn unknown_auth_mechanism_names_are_rejected() {
        assert_rejected(
            served_raw("admin", &["bogus"], Some(TEST_SHARED_SECRET)),
            "internal_api.auth_methods",
        );
    }

    #[test]
    fn duplicate_auth_mechanisms_are_rejected() {
        assert_rejected(
            served_raw(
                "admin",
                &["shared_secret", "shared_secret"],
                Some(TEST_SHARED_SECRET),
            ),
            "more than once",
        );
    }

    #[test]
    fn singular_auth_method_alias_reads_as_one_element_list() {
        let raw: RawInternalApiConfig = toml::from_str(
            r#"
enabled = true
auth_method = "shared_secret"
"#,
        )
        .expect("singular alias deserializes");
        assert_eq!(raw.auth_methods, vec!["shared_secret".to_string()]);
    }

    // ── shared-secret floor (task 03) ───────────────────────────────────

    #[test]
    fn shared_secret_length_floor_boundary_is_enforced() {
        // 31 bytes: one below the floor — rejected, naming only the length.
        let below = "a".repeat(MIN_SHARED_SECRET_BYTES - 1);
        let Error::ConfigError { detail } =
            Config::resolve(served_raw("admin", &["shared_secret"], Some(&below)))
                .expect_err("a 31-byte secret must be rejected")
        else {
            unreachable!("expected ConfigError");
        };
        assert!(detail.contains("at least 32"), "must name the floor: {detail}");
        assert!(
            !detail.contains(&below),
            "the secret value must never appear in the error"
        );

        // 32 bytes: exactly the floor — accepted.
        let config = Config::resolve(served_raw(
            "admin",
            &["shared_secret"],
            Some(TEST_SHARED_SECRET),
        ))
        .expect("a 32-byte secret must be accepted");
        assert!(config.internal_api.uses_shared_secret());
    }

    #[test]
    fn served_internal_api_with_missing_secret_is_rejected() {
        assert_rejected(
            served_raw("admin", &["shared_secret"], None),
            "internal_api.shared_secret",
        );
    }

    // ── listener collision (task 04) ────────────────────────────────────

    #[test]
    fn internal_api_listener_defaults_to_loopback_adjacent_port() {
        let config = Config::resolve(served_raw(
            "admin",
            &["shared_secret"],
            Some(TEST_SHARED_SECRET),
        ))
        .expect("served admin config resolves");
        assert_eq!(config.internal_api.host, DEFAULT_INTERNAL_API_HOST);
        assert_eq!(config.internal_api.port, DEFAULT_INTERNAL_API_PORT);
        assert_eq!(
            config.internal_api.bind_address(),
            format!("{DEFAULT_INTERNAL_API_HOST}:{DEFAULT_INTERNAL_API_PORT}")
        );
    }

    #[test]
    fn listeners_collide_matches_exact_and_wildcard_pairs_only() {
        assert!(listeners_collide("0.0.0.0", 8080, "0.0.0.0", 8080));
        assert!(listeners_collide("127.0.0.1", 8080, "127.0.0.1", 8080));
        assert!(
            listeners_collide("0.0.0.0", 8080, "127.0.0.1", 8080),
            "a wildcard covers every specific interface on the same port"
        );
        assert!(listeners_collide("::", 9000, "127.0.0.1", 9000));
        assert!(!listeners_collide("0.0.0.0", 8080, "0.0.0.0", 8081));
        assert!(!listeners_collide("127.0.0.1", 8080, "127.0.0.2", 8080));
    }

    #[test]
    fn all_role_with_colliding_admin_listener_is_rejected() {
        let mut raw = served_raw("all", &["shared_secret"], Some(TEST_SHARED_SECRET));
        raw.internal_api.host = raw.server.host.clone();
        raw.internal_api.port = raw.server.port;
        assert_rejected(raw, "collides with the public listener");
    }

    #[test]
    fn admin_role_on_the_public_port_is_accepted() {
        // role = "admin" never binds the public socket, so equal values are
        // harmless.
        let mut raw = served_raw("admin", &["shared_secret"], Some(TEST_SHARED_SECRET));
        raw.internal_api.host = raw.server.host.clone();
        raw.internal_api.port = raw.server.port;
        Config::resolve(raw).expect("admin role on the public port resolves");
    }

    #[test]
    fn all_role_with_distinct_listeners_is_accepted() {
        let raw = served_raw("all", &["shared_secret"], Some(TEST_SHARED_SECRET));
        Config::resolve(raw).expect("distinct listeners resolve");
    }

    #[test]
    fn admin_listener_collision_is_ignored_when_internal_api_disabled() {
        let mut raw = default_raw_config();
        raw.server.role = "all".to_string();
        raw.internal_api.enabled = false;
        raw.internal_api.host = raw.server.host.clone();
        raw.internal_api.port = raw.server.port;
        Config::resolve(raw).expect("a disabled admin listener cannot collide");
    }

    // ── operator-token mechanism requirements ───────────────────────────

    fn operator_token_raw() -> RawConfig {
        let mut raw = served_raw("admin", &["operator_token"], None);
        raw.key_manager.adapter = "local".to_string();
        raw.key_manager.local = Some(RawLocalKeyConfig {
            private_key_path: "/tmp/test-key.pem".to_string(),
            algorithm: "EdDSA".to_string(),
            kid: "test-kid".to_string(),
        });
        raw
    }

    #[test]
    fn operator_token_on_a_keyless_manager_is_rejected() {
        let mut raw = operator_token_raw();
        raw.key_manager.adapter = "oidc".to_string();
        raw.key_manager.local = None;
        assert_rejected(raw, "key_manager.adapter");
    }

    #[test]
    fn operator_token_with_a_real_key_manager_is_accepted() {
        let config = Config::resolve(operator_token_raw())
            .expect("operator_token over a local key manager resolves");
        assert!(config.internal_api.uses_operator_token());
        assert_eq!(config.internal_api.token_audience, DEFAULT_TOKEN_AUDIENCE);
        assert_eq!(config.internal_api.required_claim, DEFAULT_REQUIRED_CLAIM);
        assert_eq!(config.internal_api.required_value, DEFAULT_REQUIRED_VALUE);
    }

    #[test]
    fn blank_required_value_is_rejected_while_operator_token_enabled() {
        let mut raw = operator_token_raw();
        raw.internal_api.required_value = "   ".to_string();
        assert_rejected(raw, "internal_api.required_value");
    }

    #[test]
    fn operator_token_audience_shared_with_user_tokens_is_rejected() {
        let mut raw = operator_token_raw();
        raw.internal_api.token_audience = raw.token.audience.clone();
        assert_rejected(raw, "must differ from");
    }

    #[test]
    fn audience_equality_is_only_refused_while_operator_token_is_enabled() {
        let mut raw = served_raw("admin", &["shared_secret"], Some(TEST_SHARED_SECRET));
        raw.internal_api.token_audience = raw.token.audience.clone();
        Config::resolve(raw)
            .expect("a shared audience is harmless while operator_token is disabled");
    }

    // ── mtls mechanism requirements ─────────────────────────────────────

    #[test]
    fn empty_mtls_subject_header_is_rejected_while_mtls_enabled() {
        let mut raw = served_raw("admin", &["mtls"], None);
        raw.internal_api.mtls = Some(MtlsConfig {
            subject_header: "  ".to_string(),
        });
        assert_rejected(raw, "internal_api.mtls.subject_header");
    }

    // ── reserved custom claims ──────────────────────────────────────────

    #[test]
    fn reserved_name_in_token_custom_claims_is_rejected() {
        let mut raw = default_raw_config();
        raw.token.custom_claims = Some(HashMap::from([(
            "sid".to_string(),
            "forged".to_string(),
        )]));
        assert_rejected(raw, "reserved protocol claim");
    }

    #[test]
    fn non_reserved_token_custom_claim_keys_are_accepted() {
        let mut raw = default_raw_config();
        raw.token.custom_claims = Some(HashMap::from([(
            "org".to_string(),
            "example".to_string(),
        )]));
        Config::resolve(raw).expect("non-reserved custom claims resolve");
    }
}

#[cfg(test)]
mod base_path_normal_form_tests {
    use super::*;

    fn raw_with_base_path(base_path: Option<&str>) -> RawConfig {
        let mut raw: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
            .expect("default config should deserialize");
        raw.server.base_path = base_path.map(str::to_string);
        raw
    }

    /// Every tolerated sloppy spelling of `[server] base_path` lands on its
    /// canonical form: unset stays unset, empty/root fold to unset, one
    /// trailing slash is trimmed, and a residual root (`"//"`) folds to unset.
    #[test]
    fn normalise_base_path_canonicalises_unset_root_and_trailing_slash() {
        assert_eq!(normalise_base_path(None), None, "unset must stay unset");
        assert_eq!(normalise_base_path(Some(String::new())), None);
        assert_eq!(normalise_base_path(Some("/".to_string())), None);
        assert_eq!(normalise_base_path(Some("//".to_string())), None);
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
    }

    /// Normalisation never invents path structure: a missing leading slash is
    /// left for resolution to reject by name.
    #[test]
    fn normalise_base_path_leaves_missing_leading_slash_for_resolve_to_reject() {
        assert_eq!(
            normalise_base_path(Some("prod".to_string())),
            Some("prod".to_string())
        );
    }

    /// The load-time contract: sloppy-but-tolerated spellings resolve to their
    /// canonical forms, and a value with no leading slash fails resolution
    /// naming the field.
    #[test]
    fn resolve_yields_load_time_base_path_contract() {
        for (input, expected) in [
            (None, None),
            (Some(""), None),
            (Some("/"), None),
            (Some("/prod/"), Some("/prod")),
            (Some("/prod"), Some("/prod")),
        ] {
            let config = Config::resolve(raw_with_base_path(input))
                .expect("tolerated base_path spellings must resolve");
            assert_eq!(config.server.base_path.as_deref(), expected);
        }

        let Error::ConfigError { detail } = Config::resolve(raw_with_base_path(Some("prod")))
            .expect_err("base_path without a leading slash must be rejected")
        else {
            unreachable!("expected ConfigError");
        };
        assert!(
            detail.contains("server.base_path"),
            "detail must name the field: {detail}"
        );
        assert!(
            detail.contains("prod"),
            "detail must echo the offending value: {detail}"
        );
    }

    /// The shared body ceiling default survives resolution.
    #[test]
    fn max_request_body_bytes_defaults_to_two_mebibytes() {
        let config =
            Config::resolve(raw_with_base_path(None)).expect("default config resolves");
        assert_eq!(
            config.server.max_request_body_bytes,
            DEFAULT_MAX_REQUEST_BODY_BYTES
        );
    }
}
