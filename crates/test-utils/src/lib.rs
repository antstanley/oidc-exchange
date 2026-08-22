use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use oidc_exchange_core::config::DEFAULT_REFRESH_REUSE_RETENTION;
use oidc_exchange_core::domain::{
    is_valid_family_id, AuditEvent, IdentityClaims, NewUser, ProviderTokens, RefreshResolution,
    RetiredRefreshToken, Session, User, UserPatch, UserStatus, INITIAL_USER_VERSION,
};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::{
    AuditLog, IdentityProvider, KeyManager, SessionRepository, UserRepository, UserSync,
};

// ---------------------------------------------------------------------------
// MockRepository
// ---------------------------------------------------------------------------

struct MockRepositoryState {
    users: HashMap<String, User>,
    /// Live session generations, keyed by refresh-token hash. At most one
    /// generation of a family lives at any instant.
    sessions: HashMap<String, Session>,
    /// Retired generations retained for reuse detection, keyed by the retired
    /// hash. Written only by `rotate_refresh_token`'s atomic swap.
    retired: HashMap<String, RetiredRefreshToken>,
}

#[derive(Clone)]
pub struct MockRepository {
    state: Arc<Mutex<MockRepositoryState>>,
    /// When set, `revoke_session`, `revoke_family`, and
    /// `revoke_all_user_sessions` return `Err(StoreError)` instead of mutating
    /// state — models a session store that is unreachable, for exercising the
    /// `/revoke` 503 path.
    session_fail_mode: Arc<Mutex<bool>>,
    /// When set, `get_session_by_refresh_token` returns `Err(StoreError)`
    /// instead of a lookup result — models a session store that is
    /// unreachable during the `/revoke` presence-check read, distinct from
    /// `session_fail_mode` which only fires on the mutating revoke calls.
    session_lookup_fail_mode: Arc<Mutex<bool>>,
    /// How long a retirement record stays readable after its rotation:
    /// `retired_at + reuse_retention_secs`, capped per record at the family's
    /// absolute `expires_at`. Mirrors what each persistent adapter computes
    /// from `[token] refresh_reuse_retention`; configurable so tests can pin
    /// record expiry deterministically instead of sleeping past the default.
    reuse_retention_secs: u64,
}

impl MockRepository {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockRepositoryState {
                users: HashMap::new(),
                sessions: HashMap::new(),
                retired: HashMap::new(),
            })),
            session_fail_mode: Arc::new(Mutex::new(false)),
            session_lookup_fail_mode: Arc::new(Mutex::new(false)),
            reuse_retention_secs: Self::default_reuse_retention_secs(),
        }
    }

    /// Build a repository whose retirement records expire exactly
    /// `reuse_retention_secs` after their rotation (still capped by the
    /// family's absolute deadline). Must be positive: a zero-width retention
    /// window would retire and forget a generation in the same instant,
    /// silently disarming reuse detection.
    pub fn with_refresh_reuse_retention_secs(reuse_retention_secs: u64) -> Self {
        assert!(
            reuse_retention_secs > 0,
            "reuse retention must be greater than zero"
        );
        Self {
            reuse_retention_secs,
            ..Self::new()
        }
    }

    fn default_reuse_retention_secs() -> u64 {
        oidc_exchange_core::service::parse_duration_secs(DEFAULT_REFRESH_REUSE_RETENTION)
            .expect("DEFAULT_REFRESH_REUSE_RETENTION must parse as a duration")
    }

    pub async fn get_all_users(&self) -> Vec<User> {
        let state = self.state.lock().await;
        state.users.values().cloned().collect()
    }

    pub async fn get_all_sessions(&self) -> Vec<Session> {
        let state = self.state.lock().await;
        let mut sessions: Vec<Session> = state.sessions.values().cloned().collect();
        // Sort by hash so callers observe a deterministic order from the
        // hash-map-backed store (assertions on collections must not depend on
        // HashMap iteration order).
        sessions.sort_by(|a, b| a.refresh_token_hash.cmp(&b.refresh_token_hash));
        sessions
    }

    /// Every retained retirement record, sorted by retired hash for
    /// deterministic inspection of the reuse-detection state.
    pub async fn get_all_retired_tokens(&self) -> Vec<RetiredRefreshToken> {
        let state = self.state.lock().await;
        let mut retired: Vec<RetiredRefreshToken> = state.retired.values().cloned().collect();
        retired.sort_by(|a, b| a.refresh_token_hash.cmp(&b.refresh_token_hash));
        retired
    }

    /// Toggle whether `revoke_session`, `revoke_family`, and
    /// `revoke_all_user_sessions` fail with `Error::StoreError`, simulating an
    /// unreachable session store.
    pub async fn set_session_fail_mode(&self, fail: bool) {
        *self.session_fail_mode.lock().await = fail;
    }

    /// Toggle whether `get_session_by_refresh_token` fails with
    /// `Error::StoreError`, simulating an unreachable session store during
    /// the `/revoke` presence-check read.
    pub async fn set_session_lookup_fail_mode(&self, fail: bool) {
        *self.session_lookup_fail_mode.lock().await = fail;
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

    async fn get_user_by_external_id(
        &self,
        external_id: &str,
        provider: &str,
    ) -> Result<Option<User>> {
        let state = self.state.lock().await;
        Ok(state
            .users
            .values()
            .find(|u| {
                u.external_id == external_id
                    && u.provider == provider
                    && u.status != UserStatus::Deleted
            })
            .cloned())
    }

    async fn create_user(&self, new_user: &NewUser) -> Result<User> {
        let mut state = self.state.lock().await;

        // Mirror the durable backends' uniqueness constraint on the live
        // (provider, external_id) pair: a deleted user frees the slot, but a
        // concurrent winner that already created a live row must be reported
        // as a conflict rather than silently overwritten.
        let conflict = state.users.values().any(|u| {
            u.external_id == new_user.external_id
                && u.provider == new_user.provider
                && u.status != UserStatus::Deleted
        });
        if conflict {
            return Err(Error::Conflict {
                detail: format!(
                    "user already exists for provider={} external_id={}",
                    new_user.provider, new_user.external_id
                ),
            });
        }

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
            version: INITIAL_USER_VERSION,
            created_at: now,
            updated_at: now,
        };
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
        // Mirror the durable backends' version-conditional `update_user`: every successful
        // update increments `version` by exactly one, so mock-backed tests exercise the
        // same optimistic-concurrency semantics as Dynamo/Postgres/SQLite.
        user.version += 1;

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

    async fn count_by_status(&self) -> Result<HashMap<String, u64>> {
        let state = self.state.lock().await;
        let mut counts: HashMap<String, u64> = HashMap::new();
        for user in state.users.values() {
            let status_str = match user.status {
                UserStatus::Active => "active",
                UserStatus::Suspended => "suspended",
                UserStatus::Deleted => "deleted",
            };
            *counts.entry(status_str.to_string()).or_insert(0) += 1;
        }
        Ok(counts)
    }

    async fn list_users(&self, offset: u64, limit: u64) -> Result<Vec<User>> {
        let state = self.state.lock().await;
        let mut users: Vec<User> = state.users.values().cloned().collect();
        users.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        let start = offset as usize;
        let end = std::cmp::min(start + limit as usize, users.len());
        if start >= users.len() {
            return Ok(Vec::new());
        }
        Ok(users[start..end].to_vec())
    }
}

/// Compute a retirement record's expiry the way every backend must:
/// `retired_at + reuse_retention`, capped at the family's absolute deadline so
/// a record never outlives its family.
fn retirement_expires_at(
    retired_at: DateTime<Utc>,
    reuse_retention_secs: u64,
    family_expires_at: DateTime<Utc>,
) -> DateTime<Utc> {
    let retention_deadline = retired_at + chrono::Duration::seconds(reuse_retention_secs as i64);
    if retention_deadline < family_expires_at {
        retention_deadline
    } else {
        family_expires_at
    }
}

#[async_trait]
impl SessionRepository for MockRepository {
    async fn store_refresh_token(&self, session: &Session) -> Result<()> {
        // Precondition: callers mint well-formed family ids; a malformed one
        // here is a programmer error, not data to store silently.
        assert!(
            is_valid_family_id(&session.family_id),
            "store_refresh_token: malformed family id {:?}",
            session.family_id
        );
        assert!(
            !session.refresh_token_hash.is_empty(),
            "store_refresh_token: refresh_token_hash must not be empty"
        );

        let mut state = self.state.lock().await;
        state
            .sessions
            .insert(session.refresh_token_hash.clone(), session.clone());
        Ok(())
    }

    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>> {
        if *self.session_lookup_fail_mode.lock().await {
            return Err(Error::StoreError {
                detail: "mock session store lookup failure".into(),
            });
        }
        let state = self.state.lock().await;
        Ok(state.sessions.get(token_hash).cloned())
    }

    /// Classify against live generations first, then retained retirement
    /// records. A record whose successor is still live reports `Superseded`
    /// (the grace-eligible shape); anything else it reports `Retired`. All
    /// under one lock acquisition, so the answer can never straddle a
    /// concurrent rotation (SR1).
    async fn resolve_refresh_token(&self, token_hash: &str) -> Result<RefreshResolution> {
        let state = self.state.lock().await;

        if let Some(live) = state.sessions.get(token_hash) {
            return Ok(RefreshResolution::Live(live.clone()));
        }

        let Some(record) = state.retired.get(token_hash) else {
            return Ok(RefreshResolution::Unknown);
        };

        match state.sessions.get(&record.successor_hash) {
            Some(successor_live) => {
                // Pairing invariant of `rotate_refresh_token`: a successor
                // pointer always names a generation of the same family.
                assert_eq!(
                    successor_live.family_id, record.family_id,
                    "mock store corruption: successor {} names family {} but lives in {}",
                    record.successor_hash, record.family_id, successor_live.family_id
                );
                Ok(RefreshResolution::Superseded {
                    live: successor_live.clone(),
                    retired_at: record.retired_at,
                })
            }
            None => Ok(RefreshResolution::Retired {
                family_id: record.family_id.clone(),
                user_id: record.user_id.clone(),
                retired_at: record.retired_at,
            }),
        }
    }

    /// One mutex-guarded transition performing all three effects — delete the
    /// live row, write the retirement record, install the replacement — or
    /// nothing (SR2/SR3/SR4). A failed condition returns before any mutation,
    /// so the store stays byte-identical across a losing redemption.
    async fn rotate_refresh_token(&self, live_hash: &str, replacement: &Session) -> Result<bool> {
        assert!(
            is_valid_family_id(&replacement.family_id),
            "rotate_refresh_token: malformed family id {:?}",
            replacement.family_id
        );
        assert_ne!(
            live_hash, replacement.refresh_token_hash,
            "rotate_refresh_token: replacement must be a fresh generation"
        );

        let mut state = self.state.lock().await;

        // CAS condition: the named hash must still be a live generation.
        let Some(live) = state.sessions.get(live_hash) else {
            return Ok(false);
        };
        // Precondition: a rotation replaces a generation of the same family
        // for the same user — anything else would strand credentials in a
        // family their holder no longer controls.
        assert_eq!(
            live.family_id, replacement.family_id,
            "rotate_refresh_token: replacement family {:?} must match live family {:?}",
            replacement.family_id, live.family_id
        );
        assert_eq!(
            live.user_id, replacement.user_id,
            "rotate_refresh_token: replacement user {:?} must match live user {:?}",
            replacement.user_id, live.user_id
        );
        assert!(
            !state.sessions.contains_key(&replacement.refresh_token_hash),
            "rotate_refresh_token: replacement hash already exists as a live session"
        );
        assert!(
            !state.retired.contains_key(&replacement.refresh_token_hash),
            "rotate_refresh_token: replacement hash already exists as a retired record"
        );

        let retired_record = RetiredRefreshToken {
            refresh_token_hash: live_hash.to_string(),
            family_id: replacement.family_id.clone(),
            user_id: replacement.user_id.clone(),
            successor_hash: replacement.refresh_token_hash.clone(),
            retired_at: Utc::now(),
            expires_at: retirement_expires_at(
                Utc::now(),
                self.reuse_retention_secs,
                replacement.expires_at,
            ),
        };
        state
            .retired
            .insert(retired_record.refresh_token_hash.clone(), retired_record);
        state.sessions.remove(live_hash);
        state
            .sessions
            .insert(replacement.refresh_token_hash.clone(), replacement.clone());

        Ok(true)
    }

    async fn revoke_session(&self, token_hash: &str) -> Result<()> {
        if *self.session_fail_mode.lock().await {
            return Err(Error::StoreError {
                detail: "mock session store failure".into(),
            });
        }
        let mut state = self.state.lock().await;
        state.sessions.remove(token_hash);
        Ok(())
    }

    /// Remove the family's live generation and every retained retirement
    /// record, returning the combined count (SR5), under one lock acquisition.
    async fn revoke_family(&self, family_id: &str) -> Result<u64> {
        assert!(
            is_valid_family_id(family_id),
            "revoke_family: malformed family id {family_id:?}"
        );
        if *self.session_fail_mode.lock().await {
            return Err(Error::StoreError {
                detail: "mock session store failure".into(),
            });
        }

        let mut state = self.state.lock().await;

        let live_before = state.sessions.len();
        state
            .sessions
            .retain(|_, session| session.family_id != family_id);
        let sessions_removed = (live_before - state.sessions.len()) as u64;

        let retired_before = state.retired.len();
        state
            .retired
            .retain(|_, record| record.family_id != family_id);
        let retired_removed = (retired_before - state.retired.len()) as u64;

        Ok(sessions_removed + retired_removed)
    }

    async fn count_active_sessions(&self) -> Result<u64> {
        let state = self.state.lock().await;
        let now = Utc::now();
        let count = state
            .sessions
            .values()
            .filter(|s| s.expires_at > now)
            .count();
        Ok(count as u64)
    }

    /// Remove every live generation and retained retirement record belonging
    /// to `user_id` — the SR5 removal guarantee applied across the user's
    /// whole family set.
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()> {
        if *self.session_fail_mode.lock().await {
            return Err(Error::StoreError {
                detail: "mock session store failure".into(),
            });
        }
        let mut state = self.state.lock().await;
        state.sessions.retain(|_, s| s.user_id != user_id);
        state.retired.retain(|_, r| r.user_id != user_id);
        Ok(())
    }

    /// Sweep both expired sessions and expired retirement records; the count
    /// covers both, mirroring the SQL adapters' two-table sweep.
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let mut state = self.state.lock().await;
        let now = Utc::now();

        let sessions_before = state.sessions.len();
        state.sessions.retain(|_, s| s.expires_at > now);
        let sessions_removed = (sessions_before - state.sessions.len()) as u64;

        let retired_before = state.retired.len();
        state.retired.retain(|_, r| r.expires_at > now);
        let retired_removed = (retired_before - state.retired.len()) as u64;

        Ok(sessions_removed + retired_removed)
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

    async fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<bool> {
        use ed25519_dalek::{Signature, Verifier};
        let sig_bytes: [u8; 64] = signature.try_into().map_err(|_| Error::KeyError {
            detail: format!(
                "invalid Ed25519 signature length: expected 64, got {}",
                signature.len()
            ),
        })?;
        let sig = Signature::from_bytes(&sig_bytes);
        Ok(self
            .signing_key
            .verifying_key()
            .verify(payload, &sig)
            .is_ok())
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
    fail_mode: Arc<Mutex<bool>>,
}

impl MockUserSync {
    pub fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            fail_mode: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn calls(&self) -> Vec<UserSyncCall> {
        self.calls.lock().await.clone()
    }

    /// When enabled, every `UserSync` method returns `Err` instead of
    /// recording the call — models a sync backend (e.g. a webhook target)
    /// that fails every delivery attempt, for exercising best-effort
    /// log-and-continue behaviour.
    pub async fn set_fail_mode(&self, fail: bool) {
        *self.fail_mode.lock().await = fail;
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
        if *self.fail_mode.lock().await {
            return Err(Error::SyncError {
                detail: "mock user sync failure".into(),
            });
        }
        self.calls
            .lock()
            .await
            .push(UserSyncCall::Created(user.clone()));
        Ok(())
    }

    async fn notify_user_updated(&self, user: &User, changed_fields: &[&str]) -> Result<()> {
        if *self.fail_mode.lock().await {
            return Err(Error::SyncError {
                detail: "mock user sync failure".into(),
            });
        }
        self.calls.lock().await.push(UserSyncCall::Updated {
            user: user.clone(),
            changed_fields: changed_fields.iter().map(|s| s.to_string()).collect(),
        });
        Ok(())
    }

    async fn notify_user_deleted(&self, user_id: &str) -> Result<()> {
        if *self.fail_mode.lock().await {
            return Err(Error::SyncError {
                detail: "mock user sync failure".into(),
            });
        }
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
            is_private_email: None,
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
            is_private_email: None,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::MockRepository;
    use oidc_exchange_core::domain::{NewUser, UserPatch, INITIAL_USER_VERSION};
    use oidc_exchange_core::error::Error;
    use oidc_exchange_core::ports::UserRepository;

    fn make_new_user(external_id: &str, provider: &str) -> NewUser {
        NewUser {
            external_id: external_id.to_string(),
            provider: provider.to_string(),
            email: Some("user@example.com".to_string()),
            display_name: Some("Test User".to_string()),
        }
    }

    #[tokio::test]
    async fn create_user_rejects_duplicate_live_external_id() {
        let repo = MockRepository::new();
        let new_user = make_new_user("sub-1", "mock");

        let first = repo
            .create_user(&new_user)
            .await
            .expect("first create should succeed");

        let err = repo
            .create_user(&new_user)
            .await
            .expect_err("duplicate live (provider, external_id) must conflict");

        match err {
            Error::Conflict { .. } => {}
            other => panic!("expected Error::Conflict, got: {:?}", other),
        }

        // The rejected create must not have mutated state: exactly one user
        // exists, and it is the winner from the first call.
        let users = repo.get_all_users().await;
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].id, first.id);
    }

    #[tokio::test]
    async fn deleted_user_frees_external_id_for_lookup_and_recreation() {
        let repo = MockRepository::new();
        let new_user = make_new_user("sub-2", "mock");

        let original = repo
            .create_user(&new_user)
            .await
            .expect("create should succeed");
        repo.delete_user(&original.id)
            .await
            .expect("delete should succeed");

        // A deleted user must not satisfy the external-id lookup.
        let lookup = repo
            .get_user_by_external_id("sub-2", "mock")
            .await
            .expect("lookup should not error");
        assert!(lookup.is_none());

        // With the deleted row excluded from uniqueness, the identity can be
        // re-registered rather than conflicting.
        let recreated = repo
            .create_user(&new_user)
            .await
            .expect("recreate after delete should succeed, not conflict");
        assert_ne!(recreated.id, original.id);

        // Both the (soft-)deleted row and the fresh row remain in storage.
        assert_eq!(repo.get_all_users().await.len(), 2);
    }

    #[tokio::test]
    async fn update_user_increments_version_each_call() {
        let repo = MockRepository::new();
        let new_user = make_new_user("sub-3", "mock");

        let created = repo
            .create_user(&new_user)
            .await
            .expect("create should succeed");
        assert_eq!(created.version, INITIAL_USER_VERSION);

        let first_patch = UserPatch {
            email: Some("updated@example.com".to_string()),
            display_name: None,
            metadata: None,
            claims: None,
            status: None,
        };
        let after_first = repo
            .update_user(&created.id, &first_patch)
            .await
            .expect("first update should succeed");
        assert_eq!(after_first.version, INITIAL_USER_VERSION + 1);

        let second_patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
            status: Some(oidc_exchange_core::domain::UserStatus::Suspended),
        };
        let after_second = repo
            .update_user(&created.id, &second_patch)
            .await
            .expect("second update should succeed");

        // Matches the durable backends: every successful `update_user` call increments
        // `version` by exactly one, not by an arbitrary amount and not left unchanged.
        assert_eq!(after_second.version, INITIAL_USER_VERSION + 2);
        assert_eq!(
            after_second.status,
            oidc_exchange_core::domain::UserStatus::Suspended
        );
    }
}
