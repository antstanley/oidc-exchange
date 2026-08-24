use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Returned to the client from POST /token
#[derive(Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    /// Present on code exchange and on refresh (rotation issues a replacement
    /// on every redemption). Absent on refresh only when
    /// `token.refresh_rotation = false`, which restores reusable tokens.
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
    /// Stable session identity: the `family_id` (`fam_` + lowercase ULID) of
    /// the session this token was minted for. Rotation never moves it, so the
    /// `sid` names exactly one revocable token family for the token's whole
    /// validity however often the refresh token rotates beneath it. Revocation
    /// acts solely on this claim.
    ///
    /// A plain `String` field is required on deserialization: a payload
    /// without a `sid` fails closed rather than minting an un-revocable token.
    pub sid: String,
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
    /// The JWS algorithm the resolved JWK actually verified this ID token with
    /// (e.g. `"RS256"`, `"ES256"`), never the untrusted JWT header's value. The
    /// core's `at_hash` binding check reads it to select the matching digest
    /// (SHA-256 for `*256`, SHA-384 for `*384`, SHA-512 for `*512`) without
    /// re-deciding the algorithm itself.
    pub signing_alg: String,
    pub raw_claims: HashMap<String, Value>,
}
