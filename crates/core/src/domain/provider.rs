use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::secret::Secret;

#[derive(Clone, Serialize, Deserialize)]
pub struct OidcProviderConfig {
    pub provider_id: String,
    /// Required -- used for discovery
    pub issuer: String,
    pub client_id: String,
    /// The provider's client secret, when it uses one. Wrapped so the configured value
    /// cannot be formatted; serde transparency keeps config deserialization and any
    /// serialization of this struct unchanged.
    pub client_secret: Option<Secret<String>>,
    /// Optional -- discovered from issuer if absent
    pub jwks_uri: Option<String>,
    /// Optional -- discovered from issuer if absent
    pub token_endpoint: Option<String>,
    /// Optional -- discovered from issuer if absent
    pub revocation_endpoint: Option<String>,
    pub scopes: Vec<String>,
    pub additional_params: HashMap<String, String>,
}

impl std::fmt::Debug for OidcProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcProviderConfig")
            .field("provider_id", &self.provider_id)
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("jwks_uri", &self.jwks_uri)
            .field("token_endpoint", &self.token_endpoint)
            .field("revocation_endpoint", &self.revocation_endpoint)
            .field("scopes", &self.scopes)
            .field("additional_params", &self.additional_params)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET_SENTINEL: &str = "provider-config-secret-sentinel";

    fn sample_config() -> OidcProviderConfig {
        OidcProviderConfig {
            provider_id: "google".to_string(),
            issuer: "https://accounts.google.com".to_string(),
            client_id: "client-id".to_string(),
            client_secret: Some(Secret::new(SECRET_SENTINEL.to_string())),
            jwks_uri: None,
            token_endpoint: None,
            revocation_endpoint: None,
            scopes: vec!["openid".to_string()],
            additional_params: HashMap::new(),
        }
    }

    /// The hand-written Debug keeps redacting the secret now that it is a `Secret`.
    #[test]
    fn debug_output_redacts_client_secret() {
        let rendered = format!("{:?}", sample_config());

        assert!(rendered.contains("<redacted>"));
        assert!(
            !rendered.contains(SECRET_SENTINEL),
            "debug output must never contain the client secret"
        );
        assert!(rendered.contains("google"));
    }

    /// serde transparency: the client secret serializes as a plain string with the same
    /// field name and shape as before, so config files need no migration.
    #[test]
    fn serde_round_trip_keeps_plain_string_shape() {
        let serialized =
            serde_json::to_string(&sample_config()).expect("serialize provider config");
        let value: serde_json::Value = serde_json::from_str(&serialized).expect("valid JSON");
        assert_eq!(
            value["client_secret"], SECRET_SENTINEL,
            "the wrapped secret must serialize exactly as the bare string"
        );

        let back: OidcProviderConfig =
            serde_json::from_str(&serialized).expect("deserialize provider config");
        assert_eq!(
            back.client_secret.unwrap().expose(),
            &SECRET_SENTINEL.to_string()
        );
    }
}
