use serde::Deserialize;
use std::collections::HashMap;

/// Top-level application configuration, matching the TOML structure.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub registration: RegistrationConfig,
    pub token: TokenConfig,
    pub audit: AuditConfig,
    pub key_manager: KeyManagerConfig,
    pub repository: RepositoryConfig,
    pub user_sync: UserSyncConfig,
    pub telemetry: TelemetryConfig,
    pub internal_api: InternalApiConfig,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            registration: RegistrationConfig::default(),
            token: TokenConfig::default(),
            audit: AuditConfig::default(),
            key_manager: KeyManagerConfig::default(),
            repository: RepositoryConfig::default(),
            user_sync: UserSyncConfig::default(),
            telemetry: TelemetryConfig::default(),
            internal_api: InternalApiConfig::default(),
            providers: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub issuer: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            issuer: String::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct AuditConfig {
    pub adapter: String,
    pub blocking_threshold: String,
    pub cloudtrail: Option<CloudTrailConfig>,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            adapter: "noop".to_string(),
            blocking_threshold: "warning".to_string(),
            cloudtrail: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CloudTrailConfig {
    pub channel_arn: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct KeyManagerConfig {
    pub adapter: String,
    pub kms: Option<KmsConfig>,
    pub local: Option<LocalKeyConfig>,
}

impl Default for KeyManagerConfig {
    fn default() -> Self {
        Self {
            adapter: String::new(),
            kms: None,
            local: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct KmsConfig {
    pub key_id: String,
    pub algorithm: String,
    pub kid: String,
}

#[derive(Debug, Deserialize)]
pub struct LocalKeyConfig {
    pub private_key_path: String,
    pub algorithm: String,
    pub kid: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct RepositoryConfig {
    pub adapter: String,
    pub dynamodb: Option<DynamoConfig>,
}

impl Default for RepositoryConfig {
    fn default() -> Self {
        Self {
            adapter: String::new(),
            dynamodb: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DynamoConfig {
    pub table_name: String,
    pub region: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct UserSyncConfig {
    pub enabled: bool,
    pub adapter: Option<String>,
    pub webhook: Option<WebhookConfig>,
}

impl Default for UserSyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            adapter: None,
            webhook: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    pub secret: String,
    pub timeout: Option<String>,
    pub retries: Option<u32>,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct InternalApiConfig {
    pub enabled: bool,
    pub auth_method: Option<String>,
    pub shared_secret: Option<String>,
}

impl Default for InternalApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auth_method: None,
            shared_secret: None,
        }
    }
}

/// Provider configuration. The `adapter` field selects the provider type, and
/// all remaining fields are captured into `extra` via `#[serde(flatten)]` so
/// that each adapter can define its own schema.
#[derive(Debug, Deserialize)]
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
        let toml_str =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/default.toml"))
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
        assert!(config.audit.cloudtrail.is_none());

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
adapter = "cloudtrail"
blocking_threshold = "warning"

[audit.cloudtrail]
channel_arn = "arn:aws:cloudtrail:us-east-1:123456:channel/abc"

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

        let config: AppConfig = toml::from_str(toml_str).expect("failed to deserialize full config");

        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 9090);
        assert_eq!(config.server.issuer, "https://auth.example.com");

        assert_eq!(config.registration.mode, "existing_users_only");
        let allowlist = config.registration.domain_allowlist.unwrap();
        assert_eq!(allowlist.len(), 2);

        assert_eq!(config.token.audience.as_deref(), Some("https://api.example.com"));
        let claims = config.token.custom_claims.unwrap();
        assert_eq!(claims.get("org").unwrap(), "example");

        assert_eq!(config.audit.adapter, "cloudtrail");
        assert_eq!(
            config.audit.cloudtrail.unwrap().channel_arn,
            "arn:aws:cloudtrail:us-east-1:123456:channel/abc"
        );

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
        assert_eq!(config.internal_api.shared_secret.as_deref(), Some("my-secret"));

        let google = config.providers.get("google").unwrap();
        assert_eq!(google.adapter, "oidc");
        assert_eq!(
            google.extra.get("issuer").unwrap().as_str().unwrap(),
            "https://accounts.google.com"
        );
    }
}
