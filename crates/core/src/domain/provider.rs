use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
