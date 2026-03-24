use async_trait::async_trait;

use crate::domain::{IdentityClaims, ProviderTokens};
use crate::error::Result;

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    /// Exchange an authorization code for provider tokens
    async fn exchange_code(&self, code: &str, redirect_uri: &str) -> Result<ProviderTokens>;

    /// Validate an ID token and return verified claims
    async fn validate_id_token(&self, id_token: &str) -> Result<IdentityClaims>;

    /// Revoke a token at the provider (if supported)
    async fn revoke_token(&self, token: &str) -> Result<()>;

    /// Provider identifier (e.g., "google", "apple", "atproto")
    fn provider_id(&self) -> &str;
}
