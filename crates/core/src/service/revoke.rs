use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::service::AppService;

pub struct RevokeRequest {
    pub token: String,
    pub token_type_hint: Option<String>, // "refresh_token" or "access_token"
}

impl AppService {
    pub async fn revoke(&self, request: RevokeRequest) -> Result<()> {
        match request.token_type_hint.as_deref() {
            Some("access_token") => {
                // Decode the JWT payload (second segment) without verification
                if let Some(user_id) = extract_sub_from_jwt(&request.token) {
                    let _ = self.session_repo.revoke_all_user_sessions(&user_id).await;
                }
                // Per RFC 7009: always succeed
                Ok(())
            }
            Some("refresh_token") | None => {
                // Hash the token (SHA-256, hex) — same as exchange/refresh
                let token_hash = hex::encode(Sha256::digest(request.token.as_bytes()));
                // Revoke the session; if not found, that's OK per RFC 7009
                let _ = self.session_repo.revoke_session(&token_hash).await;
                Ok(())
            }
            Some(_) => {
                // Unknown hint — treat as refresh_token per spec
                let token_hash = hex::encode(Sha256::digest(request.token.as_bytes()));
                let _ = self.session_repo.revoke_session(&token_hash).await;
                Ok(())
            }
        }
    }
}

/// Extract the `sub` claim from a JWT without verifying the signature.
///
/// Decodes the base64url-encoded payload (second segment split by `.`),
/// parses it as JSON, and returns the `sub` field value.
fn extract_sub_from_jwt(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    payload.get("sub")?.as_str().map(|s| s.to_string())
}
