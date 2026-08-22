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
    /// Extra origins a discovery document is permitted to name beyond the
    /// issuer's own origin and those of explicitly configured endpoints.
    ///
    /// Each entry must be a bare `https` origin (`scheme://host[:port]`, no
    /// path, query, or fragment); entries are validated at the config boundary
    /// and re-validated defensively by the adapter. Defaults to empty, which
    /// pins the provider to its issuer's origin plus its configured overrides.
    /// The set is fixed at config load: discovery may confirm these origins but
    /// can never widen them.
    pub endpoint_origins: Vec<String>,
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
                // Redacted as always: only the presence of a secret is reported,
                // never its value. Origins and endpoints are configuration-grade
                // facts (host names, not credentials) and stay visible so a
                // mis-pinned deployment is diagnosable from debug output.
                &self.client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("jwks_uri", &self.jwks_uri)
            .field("token_endpoint", &self.token_endpoint)
            .field("revocation_endpoint", &self.revocation_endpoint)
            .field("endpoint_origins", &self.endpoint_origins)
            .field("scopes", &self.scopes)
            .field("additional_params", &self.additional_params)
            .finish()
    }
}
