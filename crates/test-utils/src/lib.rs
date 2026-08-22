use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
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

        let now = Utc::now();
        let retired_record = RetiredRefreshToken {
            refresh_token_hash: live_hash.to_string(),
            family_id: replacement.family_id.clone(),
            user_id: replacement.user_id.clone(),
            successor_hash: replacement.refresh_token_hash.clone(),
            retired_at: now,
            expires_at: RetiredRefreshToken::retention_deadline(
                now,
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
    use oidc_exchange_core::domain::{
        NewUser, RefreshResolution, RetiredRefreshToken, UserPatch, INITIAL_USER_VERSION,
    };
    use oidc_exchange_core::error::Error;
    use oidc_exchange_core::ports::{SessionRepository, UserRepository};

    fn make_new_user(external_id: &str, provider: &str) -> NewUser {
        NewUser {
            external_id: external_id.to_string(),
            provider: provider.to_string(),
            email: Some("user@example.com".to_string()),
            display_name: Some("Test User".to_string()),
        }
    }

    /// Deterministic fixture builder: well-formed `fam_` ids from the
    /// Crockford alphabet so `MockRepository`'s family-id assertions accept
    /// them, with no clock or randomness in the identifiers themselves.
    fn session_fixture(
        user_id: &str,
        hash: &str,
        family_suffix: &str,
        generation: u32,
    ) -> oidc_exchange_core::domain::Session {
        assert_eq!(
            family_suffix.len(),
            26,
            "fixture family suffix must be a full ULID-length string"
        );
        let now = chrono::Utc::now();
        oidc_exchange_core::domain::Session {
            user_id: user_id.to_string(),
            refresh_token_hash: hash.to_string(),
            family_id: format!("fam_{family_suffix}"),
            generation,
            provider: "mock".to_string(),
            expires_at: now + chrono::Duration::hours(24),
            rotated_at: (generation > 0).then_some(now),
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: now,
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

    const FAMILY_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaa";
    const FAMILY_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// The four classification shapes over one family's life: live generation,
    /// superseded-but-in-grace, retired-after-successor-fell, and unknown.
    #[tokio::test]
    async fn resolve_reports_live_superseded_retired_and_unknown() {
        let repo = MockRepository::new();
        let gen0 = session_fixture("usr_1", "hash_gen0", FAMILY_A, 0);
        repo.store_refresh_token(&gen0).await.expect("store gen 0");

        // Live: the hash is the family's current generation.
        assert_eq!(
            repo.resolve_refresh_token("hash_gen0")
                .await
                .expect("resolve gen 0"),
            RefreshResolution::Live(gen0.clone())
        );

        let mut gen1 = session_fixture("usr_1", "hash_gen1", FAMILY_A, 1);
        gen1.expires_at = gen0.expires_at;
        gen1.created_at = gen0.created_at;
        assert!(
            repo.rotate_refresh_token("hash_gen0", &gen1)
                .await
                .expect("rotate to gen 1"),
            "first rotation must win its CAS"
        );

        // Superseded: gen 0 is retired and its named successor is still live.
        match repo
            .resolve_refresh_token("hash_gen0")
            .await
            .expect("resolve retired gen 0")
        {
            RefreshResolution::Superseded { live, .. } => {
                assert_eq!(live.refresh_token_hash, "hash_gen1");
            }
            other => panic!("gen 0 must classify as Superseded once retired, got {other:?}"),
        }

        let mut gen2 = session_fixture("usr_1", "hash_gen2", FAMILY_A, 2);
        gen2.expires_at = gen0.expires_at;
        gen2.created_at = gen0.created_at;
        assert!(
            repo.rotate_refresh_token("hash_gen1", &gen2)
                .await
                .expect("rotate to gen 2"),
            "second rotation must win its CAS"
        );

        // Retired: gen 0's successor (gen 1) is no longer live — reuse, not grace.
        match repo
            .resolve_refresh_token("hash_gen0")
            .await
            .expect("resolve fallen gen 0")
        {
            RefreshResolution::Retired {
                family_id, user_id, ..
            } => {
                assert_eq!(family_id, gen0.family_id);
                assert_eq!(user_id, "usr_1");
            }
            other => panic!("fallen gen 0 must classify as Retired, got {other:?}"),
        }

        // Unknown: nothing live and nothing retained matches.
        assert_eq!(
            repo.resolve_refresh_token("hash_never_seen")
                .await
                .expect("resolve unknown"),
            RefreshResolution::Unknown
        );
    }

    /// A losing compare-and-swap must be a complete no-op: every observable
    /// piece of state — live generations, retirement records, active count —
    /// is byte-identical before and after.
    #[tokio::test]
    async fn failed_cas_makes_no_state_mutation() {
        let repo = MockRepository::new();
        let gen0 = session_fixture("usr_1", "hash_gen0", FAMILY_A, 0);
        repo.store_refresh_token(&gen0).await.expect("store gen 0");

        let mut gen1 = session_fixture("usr_1", "hash_gen1", FAMILY_A, 1);
        gen1.expires_at = gen0.expires_at;
        gen1.created_at = gen0.created_at;
        assert!(
            repo.rotate_refresh_token("hash_gen0", &gen1)
                .await
                .expect("rotate"),
            "the first rotation must win"
        );
        // Also retire a second family so the snapshot covers mixed state.
        let other_family = session_fixture("usr_1", "hash_other", FAMILY_B, 0);
        repo.store_refresh_token(&other_family)
            .await
            .expect("store other family");

        let snapshot = || async {
            (
                repo.get_all_sessions().await,
                repo.get_all_retired_tokens().await,
                repo.count_active_sessions().await.expect("count"),
            )
        };
        let (sessions_before, retired_before, count_before) = snapshot().await;

        // Lose the race: rotate against gen 0's hash after gen 1 already
        // replaced it. Also lose against a hash that never existed.
        let stale_replacement = session_fixture("usr_1", "hash_stale", FAMILY_A, 2);
        assert!(
            !repo
                .rotate_refresh_token("hash_gen0", &stale_replacement)
                .await
                .expect("stale CAS must report false, not error"),
            "a CAS against a moved live generation must return false"
        );
        assert!(
            !repo
                .rotate_refresh_token("hash_never_seen", &stale_replacement)
                .await
                .expect("unknown-hash CAS must report false, not error"),
            "a CAS against an unknown hash must return false"
        );

        let (sessions_after, retired_after, count_after) = snapshot().await;
        assert_eq!(
            sessions_before, sessions_after,
            "live sessions must be untouched"
        );
        assert_eq!(
            retired_before, retired_after,
            "retirement records must be untouched by a losing CAS"
        );
        assert_eq!(count_before, count_after, "active count must be untouched");
        assert!(
            !repo
                .get_all_retired_tokens()
                .await
                .iter()
                .any(|r| r.refresh_token_hash == stale_replacement.refresh_token_hash),
            "the loser's replacement must never appear as a retirement record"
        );
    }

    /// Family revocation removes exactly that family's live generation and
    /// retained records, returns their combined count, and leaves sibling
    /// families untouched.
    #[tokio::test]
    async fn revoke_family_returns_count_and_scopes_to_one_family() {
        let repo = MockRepository::new();
        let gen0_a = session_fixture("usr_shared", "hash_a0", FAMILY_A, 0);
        repo.store_refresh_token(&gen0_a)
            .await
            .expect("store family A");
        let gen0_b = session_fixture("usr_shared", "hash_b0", FAMILY_B, 0);
        repo.store_refresh_token(&gen0_b)
            .await
            .expect("store family B");

        let mut gen1_a = session_fixture("usr_shared", "hash_a1", FAMILY_A, 1);
        gen1_a.expires_at = gen0_a.expires_at;
        gen1_a.created_at = gen0_a.created_at;
        assert!(
            repo.rotate_refresh_token("hash_a0", &gen1_a)
                .await
                .expect("rotate A"),
            "family A rotation must win"
        );

        // Family A now holds one live generation (a1) and one record (a0):
        // revoke_family must remove both and report 2.
        let removed = repo
            .revoke_family(&format!("fam_{FAMILY_A}"))
            .await
            .expect("revoke family A");
        assert_eq!(removed, 2, "count must cover the live row plus the record");
        assert_eq!(
            repo.resolve_refresh_token("hash_a1")
                .await
                .expect("resolve a1"),
            RefreshResolution::Unknown,
            "revoked family's live generation must read Unknown immediately"
        );
        assert_eq!(
            repo.resolve_refresh_token("hash_a0")
                .await
                .expect("resolve a0"),
            RefreshResolution::Unknown,
            "revoked family's retirement record must be gone"
        );
        assert!(
            repo.get_all_retired_tokens()
                .await
                .iter()
                .all(|r| r.family_id != format!("fam_{FAMILY_A}")),
            "no retirement record of family A may survive revocation"
        );

        // Sibling family B is untouched by family A's revocation.
        assert!(
            matches!(
                repo.resolve_refresh_token("hash_b0")
                    .await
                    .expect("resolve b0"),
                RefreshResolution::Live(_)
            ),
            "sibling family must stay live through another family's revocation"
        );
        assert_eq!(
            repo.count_active_sessions().await.expect("count"),
            1,
            "only family B's session may remain active"
        );

        // Revoking an unknown but well-formed family id succeeds with zero.
        assert_eq!(
            repo.revoke_family("fam_cccccccccccccccccccccccccc")
                .await
                .expect("revoke of unknown family"),
            0
        );
    }

    /// Retirement records expire at min(retired_at + retention, family
    /// expiry): the mock computes the same deadline the persistent adapters
    /// must, so a record can never outlive its family.
    #[tokio::test]
    async fn retirement_records_are_capped_at_the_family_deadline() {
        // Retention far beyond the family's remaining life: capped at expiry.
        let long_retention = MockRepository::with_refresh_reuse_retention_secs(86_400 * 30);
        let gen0 = session_fixture("usr_cap", "hash_cap0", FAMILY_A, 0);
        long_retention
            .store_refresh_token(&gen0)
            .await
            .expect("store gen 0");
        let mut gen1 = session_fixture("usr_cap", "hash_cap1", FAMILY_A, 1);
        gen1.expires_at = gen0.expires_at;
        gen1.created_at = gen0.created_at;
        assert!(
            long_retention
                .rotate_refresh_token("hash_cap0", &gen1)
                .await
                .expect("rotate"),
            "rotation must win"
        );
        let records = long_retention.get_all_retired_tokens().await;
        assert_eq!(records.len(), 1, "exactly one record after one rotation");
        assert_eq!(
            records[0].expires_at, gen0.expires_at,
            "record expiry must be capped at the family deadline"
        );
        assert_eq!(
            records[0].successor_hash, gen1.refresh_token_hash,
            "record must name its successor"
        );

        // The constructor rejects a zero-width retention window outright:
        // it would silently disarm reuse detection.
        let result = std::panic::catch_unwind(|| {
            MockRepository::with_refresh_reuse_retention_secs(0);
        });
        assert!(result.is_err(), "zero retention must be rejected");
    }

    /// Cleanup sweeps expired sessions and expired retirement records alike,
    /// counting both — the shape every persistent adapter's sweep must match.
    #[tokio::test]
    async fn cleanup_sweeps_expired_sessions_and_counts_them() {
        let repo = MockRepository::new();
        let live = session_fixture("usr_sweep", "hash_live", FAMILY_A, 0);
        repo.store_refresh_token(&live).await.expect("store live");

        // An already-expired session (written directly; store_refresh_token on
        // the mock does not police expiry) must be swept and counted.
        let mut expired = session_fixture("usr_sweep", "hash_expired", FAMILY_A, 0);
        expired.expires_at = chrono::Utc::now() - chrono::Duration::hours(1);
        repo.store_refresh_token(&expired)
            .await
            .expect("store expired");

        let removed = repo.cleanup_expired_sessions().await.expect("cleanup");
        assert_eq!(removed, 1, "cleanup must remove exactly the expired row");
        assert!(
            repo.get_all_sessions()
                .await
                .iter()
                .all(|s| s.refresh_token_hash != "hash_expired"),
            "expired session must be gone after cleanup"
        );
        assert_eq!(
            repo.count_active_sessions().await.expect("count"),
            1,
            "the live session must survive cleanup"
        );

        // A second sweep has nothing left to do: the count is honest about it.
        assert_eq!(
            repo.cleanup_expired_sessions()
                .await
                .expect("second cleanup"),
            0,
            "an idempotent second sweep must report zero removals"
        );
    }

    /// The mock keeps hashes-only data in its retirement records: nothing in
    /// the retained type carries a raw refresh token (audit/telemetry safety).
    #[test]
    fn retired_record_debug_leaks_nothing_but_hashes() {
        let record = RetiredRefreshToken {
            refresh_token_hash: "hash_retired".to_string(),
            family_id: format!("fam_{FAMILY_A}"),
            user_id: "usr_x".to_string(),
            successor_hash: "hash_successor".to_string(),
            retired_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(24),
        };
        let debug = format!("{record:?}");
        assert!(debug.contains("hash_retired"));
        assert!(!debug.to_lowercase().contains("token: \""));
    }
}
