use std::collections::HashMap;

use crate::config::HttpsUrl;

use crate::secret::Secret;

#[derive(Clone)]
pub struct OidcProviderConfig {
    pub provider_id: String,
    /// Required -- used for discovery
    pub issuer: HttpsUrl,
    pub client_id: String,
    /// The provider's client secret, when it uses one. Wrapped so the configured value
    /// cannot be formatted; serde transparency keeps config deserialization and any
    /// serialization of this struct unchanged.
    pub client_secret: Option<Secret<String>>,
    /// Optional -- discovered from issuer if absent
    pub jwks_uri: Option<HttpsUrl>,
    /// Optional -- discovered from issuer if absent
    pub token_endpoint: Option<HttpsUrl>,
    /// Optional -- discovered from issuer if absent
    pub revocation_endpoint: Option<HttpsUrl>,
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

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET_SENTINEL: &str = "provider-config-secret-sentinel";

    fn sample_config() -> OidcProviderConfig {
        OidcProviderConfig {
            provider_id: "google".to_string(),
            issuer: crate::config::HttpsUrl::parse("https://accounts.google.com").expect("valid https url"),
            client_id: "client-id".to_string(),
            client_secret: Some(Secret::new(SECRET_SENTINEL.to_string())),
            jwks_uri: None,
            token_endpoint: None,
            revocation_endpoint: None,
            endpoint_origins: Vec::new(),
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

}
