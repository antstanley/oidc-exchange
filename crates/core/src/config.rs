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
    pub audit: RawAuditConfig,
    pub key_manager: RawKeyManagerConfig,
    pub repository: RawRepositoryConfig,
    #[serde(default)]
    pub session_repository: RawSessionRepositoryConfig,
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
    pub audit: AuditConfig,
    pub key_manager: KeyManagerConfig,
    pub repository: RepositoryConfig,
    pub session_repository: SessionRepositoryConfig,
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
        let audit = AuditConfig::resolve(raw.audit)?;
        let key_manager = KeyManagerConfig::resolve(raw.key_manager)?;
        let repository = RepositoryConfig::resolve(raw.repository)?;
        let session_repository = SessionRepositoryConfig::resolve(raw.session_repository)?;
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
            audit,
            key_manager,
            repository,
            session_repository,
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawServerConfig {
    pub host: String,
    pub port: u16,
    pub issuer: String,
    pub role: String,
    pub request_timeout: String,
    pub base_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub issuer: HttpsUrl,
    pub role: ServerRole,
    pub request_timeout: std::time::Duration,
    pub base_path: Option<String>,
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawTokenConfig {
    pub access_token_ttl: String,
    pub refresh_token_ttl: String,
    pub audience: String,
    pub custom_claims: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct TokenConfig {
    pub access_token_ttl: std::time::Duration,
    pub refresh_token_ttl: std::time::Duration,
    pub audience: NonEmptyString,
    pub custom_claims: Option<HashMap<String, String>>,
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
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawAuditConfig {
    pub adapter: String,
    pub blocking_threshold: String,
    pub emit_threshold: String,
    pub sqs: Option<RawSqsAuditConfig>,
}

#[derive(Debug, Clone)]
pub struct AuditConfig {
    pub adapter: AuditAdapter,
    pub blocking_threshold: AuditSeverity,
    pub emit_threshold: AuditSeverity,
    pub sqs: Option<SqsAuditConfig>,
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
            algorithm: SigningAlgorithm::parse_field("key_manager.kms.algorithm", raw.algorithm)?,
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
            algorithm: SigningAlgorithm::parse_field("key_manager.local.algorithm", raw.algorithm)?,
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawSessionRepositoryConfig {
    pub adapter: Option<String>,
    pub valkey: Option<RawValkeyConfig>,
    pub lmdb: Option<RawLmdbConfig>,
}

#[derive(Debug, Clone)]
pub struct SessionRepositoryConfig {
    pub adapter: Option<ProviderAdapter>,
    pub valkey: Option<ValkeyConfig>,
    pub lmdb: Option<LmdbConfig>,
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
}

impl ProviderConfig {
    fn resolve(provider_id: String, raw: RawProviderConfig) -> Result<Self, Error> {
        Ok(Self {
            provider_id,
            adapter: ProviderAdapter::parse_field("providers.adapter", raw.adapter)?,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigningAlgorithm {
    EdDSA,
    ES256,
    RS256,
}
impl SigningAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EdDSA => "EdDSA",
            Self::ES256 => "ES256",
            Self::RS256 => "RS256",
        }
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "EdDSA" => Ok(Self::EdDSA),
            "ES256" => Ok(Self::ES256),
            "RS256" => Ok(Self::RS256),
            _ => Err(Error::ConfigError {
                detail: format!("{field}: invalid signing algorithm {value:?}"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditAdapter {
    Noop,
    Sqs,
}
impl AuditAdapter {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::Sqs => "sqs",
        }
    }

    fn parse_field(field: &str, value: String) -> Result<Self, Error> {
        match value.as_str() {
            "noop" => Ok(Self::Noop),
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
    /// Construct a validated HTTPS URL outside config deserialization, for tests.
    pub fn parse(value: impl Into<String>) -> Result<Self, Error> {
        Self::parse_field("URL", value.into())
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
    fn resolve_default_toml() {
        let config = Config::resolve(default_raw_config()).expect("failed to resolve config");
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.issuer.as_ref(), "https://localhost:8080");
        assert_eq!(config.server.request_timeout.as_secs(), 30);
        assert_eq!(config.registration.mode, RegistrationMode::Open);
        assert!(config.registration.domain_allowlist.is_none());
        assert_eq!(config.token.access_token_ttl.as_secs(), 15 * 60);
        assert_eq!(config.token.refresh_token_ttl.as_secs(), 30 * 24 * 60 * 60);
        assert_eq!(config.token.audience.as_ref(), "oidc-exchange");
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
    fn resolve_rejects_invalid_closed_config_values_with_field_names() {
        let cases = [
            ("server.role", "unknown"),
            ("registration.mode", "existing_users"),
            ("key_manager.adapter", ""),
            ("repository.adapter", ""),
            ("audit.adapter", "stdout"),
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
}
