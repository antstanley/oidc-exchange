use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct OidcProviderConfig {
    pub provider_id: String,
    /// Required -- used for discovery
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
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
            .field("client_secret", &self.client_secret.as_ref().map(|_| "<redacted>"))
            .field("jwks_uri", &self.jwks_uri)
            .field("token_endpoint", &self.token_endpoint)
            .field("revocation_endpoint", &self.revocation_endpoint)
            .field("scopes", &self.scopes)
            .field("additional_params", &self.additional_params)
            .finish()
    }
}
