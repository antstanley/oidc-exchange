use async_trait::async_trait;

use crate::error::Result;

#[async_trait]
pub trait KeyManager: Send + Sync {
    /// Sign a byte payload, return the signature
    async fn sign(&self, payload: &[u8]) -> Result<Vec<u8>>;

    /// Return the public key in JWK format for the JWKS endpoint
    async fn public_jwk(&self) -> Result<serde_json::Value>;

    /// Key algorithm identifier (e.g., "EdDSA", "ES256")
    fn algorithm(&self) -> &str;

    /// Key ID for the JWT kid header
    fn key_id(&self) -> &str;
}
