use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::Mutex;

use oidc_exchange_core::domain::{
    AuditEvent, IdentityClaims, NewUser, ProviderTokens, Session, User, UserPatch, UserStatus,
};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::{AuditLog, IdentityProvider, KeyManager, SessionRepository, UserRepository, UserSync};

// ---------------------------------------------------------------------------
// MockRepository
// ---------------------------------------------------------------------------

struct MockRepositoryState {
    users: HashMap<String, User>,
    sessions: HashMap<String, Session>,
}

#[derive(Clone)]
pub struct MockRepository {
    state: Arc<Mutex<MockRepositoryState>>,
}

impl MockRepository {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockRepositoryState {
                users: HashMap::new(),
                sessions: HashMap::new(),
            })),
        }
    }

    pub async fn get_all_users(&self) -> Vec<User> {
        let state = self.state.lock().await;
        state.users.values().cloned().collect()
    }

    pub async fn get_all_sessions(&self) -> Vec<Session> {
        let state = self.state.lock().await;
        state.sessions.values().cloned().collect()
    }
}

impl Default for MockRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl UserRepository for MockRepository {
    async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>> {
        let state = self.state.lock().await;
        Ok(state.users.get(user_id).cloned())
    }

    async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<User>> {
        let state = self.state.lock().await;
        Ok(state
            .users
            .values()
            .find(|u| u.external_id == external_id)
            .cloned())
    }

    async fn create_user(&self, new_user: &NewUser) -> Result<User> {
        let now = Utc::now();
        let id = format!("usr_{}", ulid::Ulid::new().to_string().to_lowercase());
        let user = User {
            id: id.clone(),
            external_id: new_user.external_id.clone(),
            provider: new_user.provider.clone(),
            email: new_user.email.clone(),
            display_name: new_user.display_name.clone(),
            metadata: HashMap::new(),
            claims: HashMap::new(),
            status: UserStatus::Active,
            created_at: now,
            updated_at: now,
        };
        let mut state = self.state.lock().await;
        state.users.insert(id, user.clone());
        Ok(user)
    }

    async fn update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User> {
        let mut state = self.state.lock().await;
        let user = state
            .users
            .get_mut(user_id)
            .ok_or_else(|| Error::StoreError {
                detail: format!("user not found: {}", user_id),
            })?;

        if let Some(ref email) = patch.email {
            user.email = Some(email.clone());
        }
        if let Some(ref display_name) = patch.display_name {
            user.display_name = Some(display_name.clone());
        }
        if let Some(ref metadata) = patch.metadata {
            user.metadata = metadata.clone();
        }
        if let Some(ref claims) = patch.claims {
            user.claims = claims.clone();
        }
        if let Some(ref status) = patch.status {
            user.status = status.clone();
        }
        user.updated_at = Utc::now();

        Ok(user.clone())
    }

    async fn delete_user(&self, user_id: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        let user = state
            .users
            .get_mut(user_id)
            .ok_or_else(|| Error::StoreError {
                detail: format!("user not found: {}", user_id),
            })?;
        user.status = UserStatus::Deleted;
        user.updated_at = Utc::now();
        Ok(())
    }
}

#[async_trait]
impl SessionRepository for MockRepository {
    async fn store_refresh_token(&self, session: &Session) -> Result<()> {
        let mut state = self.state.lock().await;
        state
            .sessions
            .insert(session.refresh_token_hash.clone(), session.clone());
        Ok(())
    }

    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>> {
        let state = self.state.lock().await;
        Ok(state.sessions.get(token_hash).cloned())
    }

    async fn revoke_session(&self, token_hash: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        state.sessions.remove(token_hash);
        Ok(())
    }

    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        state.sessions.retain(|_, s| s.user_id != user_id);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MockKeyManager
// ---------------------------------------------------------------------------

pub struct MockKeyManager {
    signing_key: ed25519_dalek::SigningKey,
}

impl MockKeyManager {
    pub fn new() -> Self {
        let seed: [u8; 32] = [1u8; 32];
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
        Self { signing_key }
    }
}

impl Default for MockKeyManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl KeyManager for MockKeyManager {
    async fn sign(&self, payload: &[u8]) -> Result<Vec<u8>> {
        use ed25519_dalek::Signer;
        let signature = self.signing_key.sign(payload);
        Ok(signature.to_bytes().to_vec())
    }

    async fn public_jwk(&self) -> Result<serde_json::Value> {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;

        let verifying_key = self.signing_key.verifying_key();
        let pub_bytes = verifying_key.to_bytes();
        let x = URL_SAFE_NO_PAD.encode(pub_bytes);

        Ok(serde_json::json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "alg": "EdDSA",
            "use": "sig",
            "kid": "test-key-1",
            "x": x,
        }))
    }

    fn algorithm(&self) -> &str {
        "EdDSA"
    }

    fn key_id(&self) -> &str {
        "test-key-1"
    }
}

// ---------------------------------------------------------------------------
// MockAuditLog
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct MockAuditLog {
    events: Arc<Mutex<Vec<AuditEvent>>>,
    fail_mode: Arc<Mutex<bool>>,
}

impl MockAuditLog {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            fail_mode: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn events(&self) -> Vec<AuditEvent> {
        self.events.lock().await.clone()
    }

    pub async fn set_fail_mode(&self, fail: bool) {
        *self.fail_mode.lock().await = fail;
    }
}

impl Default for MockAuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuditLog for MockAuditLog {
    async fn emit(&self, event: &AuditEvent) -> Result<()> {
        if *self.fail_mode.lock().await {
            return Err(Error::AuditError {
                detail: "mock failure".into(),
            });
        }
        self.events.lock().await.push(event.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MockUserSync
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum UserSyncCall {
    Created(User),
    Updated {
        user: User,
        changed_fields: Vec<String>,
    },
    Deleted(String),
}

#[derive(Clone)]
pub struct MockUserSync {
    calls: Arc<Mutex<Vec<UserSyncCall>>>,
}

impl MockUserSync {
    pub fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn calls(&self) -> Vec<UserSyncCall> {
        self.calls.lock().await.clone()
    }
}

impl Default for MockUserSync {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl UserSync for MockUserSync {
    async fn notify_user_created(&self, user: &User) -> Result<()> {
        self.calls
            .lock()
            .await
            .push(UserSyncCall::Created(user.clone()));
        Ok(())
    }

    async fn notify_user_updated(&self, user: &User, changed_fields: &[&str]) -> Result<()> {
        self.calls.lock().await.push(UserSyncCall::Updated {
            user: user.clone(),
            changed_fields: changed_fields.iter().map(|s| s.to_string()).collect(),
        });
        Ok(())
    }

    async fn notify_user_deleted(&self, user_id: &str) -> Result<()> {
        self.calls
            .lock()
            .await
            .push(UserSyncCall::Deleted(user_id.to_string()));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MockIdentityProvider
// ---------------------------------------------------------------------------

pub struct MockIdentityProvider {
    provider_id: String,
    exchange_response: Arc<Mutex<Option<ProviderTokens>>>,
    claims_response: Arc<Mutex<Option<IdentityClaims>>>,
}

impl MockIdentityProvider {
    pub fn new(provider_id: &str) -> Self {
        let default_tokens = ProviderTokens {
            id_token: "mock-id-token".to_string(),
            refresh_token: Some("mock-refresh-token".to_string()),
            access_token: Some("mock-access-token".to_string()),
        };

        let default_claims = IdentityClaims {
            subject: "test-subject".to_string(),
            email: Some("test@example.com".to_string()),
            email_verified: Some(true),
            name: Some("Test User".to_string()),
            raw_claims: HashMap::new(),
        };

        Self {
            provider_id: provider_id.to_string(),
            exchange_response: Arc::new(Mutex::new(Some(default_tokens))),
            claims_response: Arc::new(Mutex::new(Some(default_claims))),
        }
    }

    pub async fn set_claims(&self, claims: IdentityClaims) {
        *self.claims_response.lock().await = Some(claims);
    }

    pub async fn set_exchange_response(&self, tokens: ProviderTokens) {
        *self.exchange_response.lock().await = Some(tokens);
    }
}

#[async_trait]
impl IdentityProvider for MockIdentityProvider {
    async fn exchange_code(&self, _code: &str, _redirect_uri: &str) -> Result<ProviderTokens> {
        let response = self.exchange_response.lock().await;
        Ok(response.clone().unwrap_or(ProviderTokens {
            id_token: "mock-id-token".to_string(),
            refresh_token: None,
            access_token: None,
        }))
    }

    async fn validate_id_token(&self, _id_token: &str) -> Result<IdentityClaims> {
        let response = self.claims_response.lock().await;
        Ok(response.clone().unwrap_or(IdentityClaims {
            subject: "test-subject".to_string(),
            email: Some("test@example.com".to_string()),
            email_verified: Some(true),
            name: Some("Test User".to_string()),
            raw_claims: HashMap::new(),
        }))
    }

    async fn revoke_token(&self, _token: &str) -> Result<()> {
        Ok(())
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }
}
