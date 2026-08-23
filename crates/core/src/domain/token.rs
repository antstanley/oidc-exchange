use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::secret::Secret;

/// Returned to the client from POST /token
#[derive(Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    /// Present on code exchange, absent on refresh. The minted refresh token is wrapped
    /// so the value that exists only in memory and in this response cannot be formatted;
    /// serde transparency keeps the wire body a plain string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<Secret<String>>,
    /// Always "Bearer"
    pub token_type: String,
    /// Seconds until expiry
    pub expires_in: u64,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// Claims embedded in the access token JWT
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessTokenClaims {
    /// Internal user ID
    pub sub: String,
    /// This service's issuer URL
    pub iss: String,
    pub aud: String,
    pub iat: u64,
    pub exp: u64,
    /// Merged: config template claims + user.claims
    #[serde(flatten)]
    pub custom: HashMap<String, Value>,
}

/// What we get back from a provider after code exchange
#[derive(Clone, Serialize, Deserialize)]
pub struct ProviderTokens {
    pub id_token: String,
    pub refresh_token: Option<String>,
    pub access_token: Option<String>,
}

impl std::fmt::Debug for ProviderTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderTokens")
            .field("id_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Verified claims extracted from a provider's ID token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityClaims {
    /// Provider's sub / DID
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    /// Apple private-relay flag, coerced bool-or-string like `email_verified`;
    /// `None` for non-Apple providers.
    pub is_private_email: Option<bool>,
    pub raw_claims: HashMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Secret;

    fn sample_response() -> TokenResponse {
        TokenResponse {
            access_token: "access-token-value".to_string(),
            refresh_token: Some(Secret::new("minted-refresh-token".to_string())),
            token_type: "Bearer".to_string(),
            expires_in: 900,
        }
    }

    /// The wrapped refresh token serializes as the bare string under the same field name,
    /// so `/token` wire bodies are byte-identical to before the type change.
    #[test]
    fn token_response_serialization_is_string_identical() {
        const REFRESH_TOKEN: &str = "minted-refresh-token";
        let response = sample_response();

        let serialized = serde_json::to_value(&response).expect("serialize token response");
        assert_eq!(
            serialized["refresh_token"], REFRESH_TOKEN,
            "the wrapped refresh token must serialize exactly as the bare string"
        );
        assert_eq!(serialized["token_type"], "Bearer");
        assert_eq!(serialized["expires_in"], 900);
        assert_eq!(serialized["access_token"], "access-token-value");

        // A client-side deserialization of the same shape lands back in the typed fields.
        let back: TokenResponse =
            serde_json::from_value(serialized).expect("deserialize token response");
        assert_eq!(
            back.refresh_token
                .expect("refresh_token present")
                .into_inner(),
            REFRESH_TOKEN.to_string()
        );
    }

    /// A refresh-only response omits the optional field entirely, unchanged.
    #[test]
    fn absent_refresh_token_is_skipped_in_serialization() {
        let response = TokenResponse {
            access_token: "access-token-value".to_string(),
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_in: 900,
        };

        let serialized = serde_json::to_value(&response).expect("serialize token response");
        assert!(
            serialized.get("refresh_token").is_none(),
            "an absent refresh token must stay absent on the wire"
        );
    }

    /// The hand-written Debug keeps redacting both tokens now that the minted refresh
    /// token is a `Secret`.
    #[test]
    fn debug_output_redacts_tokens() {
        let rendered = format!("{:?}", sample_response());

        assert_eq!(rendered.matches("<redacted>").count(), 2);
        assert!(!rendered.contains("minted-refresh-token"));
        assert!(!rendered.contains("access-token-value"));
    }
}
