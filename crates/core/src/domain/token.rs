use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Returned to the client from POST /token
#[derive(Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    /// Present on code exchange, absent on refresh
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
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
    pub raw_claims: HashMap<String, Value>,
}
