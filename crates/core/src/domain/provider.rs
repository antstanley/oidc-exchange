use std::collections::HashMap;

use crate::config::HttpsUrl;

use crate::secret::Secret;

/// How a provider's `email_verified` fact is established from its token claims.
///
/// Defaults to [`EmailVerification::Standard`], the current behaviour, so a
/// provider that never mentions the setting is byte-identical to before the
/// enum existed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EmailVerification {
    /// Read only the standard `email_verified` claim (current behaviour).
    #[default]
    Standard,
    /// An absent `email_verified` claim counts as verified iff the token
    /// carries a non-empty `email` string claim.
    TrustEmail,
    /// Read the named claim (bool-or-string coerced) when `email_verified`
    /// is absent, e.g. Entra's `xms_edov`.
    Claim(String),
}

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
    /// How the `email_verified` fact is established for this provider's tokens.
    /// Configuration-grade (a mode name, not a credential), so it stays visible
    /// in the hand-written `Debug` output below.
    pub email_verification: EmailVerification,
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
            // Like endpoint_origins, the verification mode is a configuration
            // fact and stays visible so a misconfigured provider is diagnosable
            // from debug output; only the secret above is ever redacted.
            .field("email_verification", &self.email_verification)
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
            issuer: crate::config::HttpsUrl::parse("https://accounts.google.com")
                .expect("valid https url"),
            client_id: "client-id".to_string(),
            client_secret: Some(Secret::new(SECRET_SENTINEL.to_string())),
            jwks_uri: None,
            token_endpoint: None,
            revocation_endpoint: None,
            endpoint_origins: Vec::new(),
            email_verification: EmailVerification::default(),
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

    /// A provider that never mentions email verification must land on the
    /// standard mode, because the default is what preserves prior behaviour.
    #[test]
    fn email_verification_defaults_to_standard() {
        assert_eq!(EmailVerification::default(), EmailVerification::Standard);
        // The config built without any explicit choice carries that same
        // default, so constructors that opt out of the feature stay standard.
        assert_eq!(
            sample_config().email_verification,
            EmailVerification::Standard
        );
    }

    /// The verification mode is configuration-grade and must be visible in the
    /// Debug rendering, while the secret redaction is unaffected by its arrival.
    #[test]
    fn debug_output_names_email_verification_mode() {
        let rendered = format!("{:?}", sample_config());

        assert!(
            rendered.contains("email_verification: Standard"),
            "debug output must name the email verification mode: {rendered}"
        );
        // The negative-space guarantee survives the new field: adding a
        // configuration fact must never loosen the secret redaction.
        assert!(
            !rendered.contains(SECRET_SENTINEL),
            "debug output must never contain the client secret"
        );
    }
}
