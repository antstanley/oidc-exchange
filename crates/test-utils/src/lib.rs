use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use oidc_exchange_core::domain::{
    AuditEvent, IdentityClaims, NewUser, ProviderTokens, Session, SingleUseRecord, User, UserPatch,
    UserStatus, INITIAL_USER_VERSION,
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
    sessions: HashMap<String, Session>,
    /// Single-use records (nonces, assertion-replay markers) keyed by their namespaced
    /// digest. Shared by every `MockRepository` clone of one instance, so claim
    /// operations are atomic exactly as against a real store.
    single_use: HashMap<String, DateTime<Utc>>,
}

#[derive(Clone)]
pub struct MockRepository {
    state: Arc<Mutex<MockRepositoryState>>,
    /// When set, `revoke_session` and `revoke_all_user_sessions` return
    /// `Err(StoreError)` instead of mutating state — models a session store
    /// that is unreachable, for exercising the `/revoke` 503 path.
    session_fail_mode: Arc<Mutex<bool>>,
    /// When set, `get_session_by_refresh_token` returns `Err(StoreError)`
    /// instead of a lookup result — models a session store that is
    /// unreachable during the `/revoke` presence-check read, distinct from
    /// `session_fail_mode` which only fires on the mutating revoke calls.
    session_lookup_fail_mode: Arc<Mutex<bool>>,
}

impl MockRepository {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockRepositoryState {
                users: HashMap::new(),
                sessions: HashMap::new(),
                single_use: HashMap::new(),
            })),
            session_fail_mode: Arc::new(Mutex::new(false)),
            session_lookup_fail_mode: Arc::new(Mutex::new(false)),
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

    /// Observe stored single-use records (test introspection): the live record at
    /// `key`, or `None` when the key is absent. Expired records may still be physically
    /// present until a cleanup — the claim operations treat them as absent.
    pub async fn get_single_use_record(&self, key: &str) -> Option<SingleUseRecord> {
        let state = self.state.lock().await;
        state.single_use.get(key).map(|expires_at| SingleUseRecord {
            key: key.to_string(),
            expires_at: *expires_at,
        })
    }

    /// Toggle whether `revoke_session` / `revoke_all_user_sessions` fail
    /// with `Error::StoreError`, simulating an unreachable session store.
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

    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()> {
        if *self.session_fail_mode.lock().await {
            return Err(Error::StoreError {
                detail: "mock session store failure".into(),
            });
        }
        let mut state = self.state.lock().await;
        state.sessions.retain(|_, s| s.user_id != user_id);
        Ok(())
    }

    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let mut state = self.state.lock().await;
        let now = Utc::now();
        let before = state.sessions.len();
        state.sessions.retain(|_, s| s.expires_at > now);
        let sessions_removed = (before - state.sessions.len()) as u64;

        // The sweep also reclaims expired single-use records (space reclamation only —
        // the claim operations already treat an expired record as absent), and the
        // returned count covers both kinds, per the port contract.
        let single_use_before = state.single_use.len();
        state.single_use.retain(|_, expires_at| *expires_at > now);
        let single_use_removed = (single_use_before - state.single_use.len()) as u64;

        debug_assert!(
            sessions_removed as usize <= before,
            "removed session count cannot exceed the pre-sweep size"
        );
        Ok(sessions_removed + single_use_removed)
    }

    async fn put_single_use(&self, key: &str, expires_at: DateTime<Utc>) -> Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        let mut state = self.state.lock().await;
        // Expired-is-absent: a record whose expiry has passed never blocks a fresh claim.
        if state
            .single_use
            .get(key)
            .is_some_and(|live_expires_at| *live_expires_at > Utc::now())
        {
            return Ok(false);
        }
        state.single_use.insert(key.to_string(), expires_at);
        Ok(true)
    }

    async fn take_single_use(&self, key: &str) -> Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        let mut state = self.state.lock().await;
        match state.single_use.remove(key) {
            Some(expires_at) if expires_at > Utc::now() => Ok(true),
            // Absent, already-burned, and expired are indistinguishable: all report false.
            _ => Ok(false),
        }
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

/// A stand-in [`IdentityProvider`] whose responses are pinned per test.
///
/// `Clone` shares one underlying state (`Arc`), so a test can hold a handle,
/// move a clone into the service under test, and still re-pin claims or
/// exchange responses afterwards.
#[derive(Clone)]
pub struct MockIdentityProvider {
    provider_id: String,
    /// The audience the mock's claims are pinned to, reported through the port's
    /// `client_id()`; configurable so binding tests can exercise `azp` mismatches.
    client_id: String,
    exchange_response: Arc<Mutex<Option<ProviderTokens>>>,
    claims_response: Arc<Mutex<Option<IdentityClaims>>>,
}

/// Default `client_id()` the mock reports; matches the audience used across the
/// repo's provider test fixtures (`test-client-id`) unless overridden.
pub const MOCK_CLIENT_ID: &str = "test-client-id";

/// Remaining lifetime stamped into a default mock assertion's `exp` claim
/// (10 minutes): comfortably inside the default `grants.max_assertion_lifetime`
/// ceiling of 1h, so exchanges over default-config services bind cleanly.
pub const MOCK_DEFAULT_ASSERTION_TTL_SECS: u64 = 600;

impl MockIdentityProvider {
    pub fn new(provider_id: &str) -> Self {
        let default_tokens = ProviderTokens {
            id_token: "mock-id-token".to_string(),
            refresh_token: Some("mock-refresh-token".to_string()),
            access_token: Some("mock-access-token".to_string()),
        };

        Self {
            provider_id: provider_id.to_string(),
            client_id: MOCK_CLIENT_ID.to_string(),
            exchange_response: Arc::new(Mutex::new(Some(default_tokens))),
            // `None` until a test pins explicit claims; `validate_id_token`
            // then builds fresh defaults per call (unique `jti`, live `exp`)
            // instead of replaying one frozen template forever.
            claims_response: Arc::new(Mutex::new(None)),
        }
    }

    /// Default verified claims for callers that never pinned explicit ones.
    ///
    /// Built per call, not stored: each exchange gets a fresh `jti` and an
    /// `exp` relative to now, mirroring what real validators return — every
    /// code-path exchange is therefore a distinct, unspent assertion. The raw
    /// claims carry exactly what the core binding controls read (`exp`,
    /// `jti`, `sub`) and nothing more.
    pub fn default_claims() -> IdentityClaims {
        IdentityClaims {
            subject: "test-subject".to_string(),
            email: Some("test@example.com".to_string()),
            email_verified: Some(true),
            name: Some("Test User".to_string()),
            is_private_email: None,
            // A stand-in for the resolved JWK algorithm a real validator would report;
            // RS256 is the common upstream case and selects SHA-256 for at_hash tests.
            signing_alg: "RS256".to_string(),
            raw_claims: Self::default_raw_claims(),
        }
    }

    /// Raw claims backing [`Self::default_claims`]: a usable `exp`, a fresh
    /// per-call `jti`, and the subject echoed as OIDC validators would.
    fn default_raw_claims() -> std::collections::HashMap<String, serde_json::Value> {
        use serde_json::json;

        let mut raw = std::collections::HashMap::new();
        raw.insert("sub".to_string(), json!("test-subject"));
        raw.insert("jti".to_string(), json!(ulid::Ulid::new().to_string()));
        raw.insert(
            "exp".to_string(),
            json!(chrono::Utc::now().timestamp() + MOCK_DEFAULT_ASSERTION_TTL_SECS as i64),
        );
        raw
    }

    pub async fn set_claims(&self, claims: IdentityClaims) {
        *self.claims_response.lock().await = Some(claims);
    }

    pub async fn set_exchange_response(&self, tokens: ProviderTokens) {
        *self.exchange_response.lock().await = Some(tokens);
    }

    /// Override the audience reported through the port's `client_id()`. Consuming
    /// builder (not a setter): the port hands out `&str` borrowed from this field, so
    /// it must be fixed before the mock is shared with the service under test.
    pub fn with_client_id(mut self, client_id: &str) -> Self {
        assert!(!client_id.is_empty(), "mock client_id must be non-empty");
        self.client_id = client_id.to_string();
        self
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
        Ok(response
            .clone()
            .unwrap_or_else(MockIdentityProvider::default_claims))
    }

    async fn revoke_token(&self, _token: &str) -> Result<()> {
        Ok(())
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn client_id(&self) -> &str {
        &self.client_id
    }
}

// ---------------------------------------------------------------------------
// Single-use repository conformance suite
// ---------------------------------------------------------------------------

/// Shared conformance scenarios for the `SessionRepository` single-use pair
/// (`put_single_use` / `take_single_use`). Every store adapter calls these from its own
/// test module, so the atomic-claim contract is exercised identically across DynamoDB,
/// Postgres, SQLite, Valkey, LMDB, and `MockRepository` — a reviewer runs one suite and
/// sees exactly-one-winner semantics hold everywhere.
///
/// Scenarios are written against [`SessionRepository`] only (no adapter-specific state),
/// use collision-proof keys, and leave no live records behind except where noted.
pub mod single_use_conformance {
    use std::sync::Arc;

    use chrono::{Duration, Utc};

    use oidc_exchange_core::ports::SessionRepository;

    /// Lifetime of the "short-lived" record used by the expired-is-absent scenario.
    /// Two seconds (rather than the theoretical one-second floor) tolerates adapters
    /// that truncate TTLs to whole seconds when writing.
    pub const SHORT_TTL_SECONDS: i64 = 2;

    /// How long scenarios wait before treating a [`SHORT_TTL_SECONDS`] record as
    /// expired: half a second past its death so slow CI cannot flake.
    pub const EXPIRY_WAIT_MS: u64 = 2500;

    /// A collision-proof key for one scenario invocation, safe against shared stores
    /// and concurrent nextest processes.
    fn scenario_key(label: &str) -> String {
        format!(
            "nonce:{label}:{}",
            ulid::Ulid::new().to_string().to_lowercase()
        )
    }

    /// First `put_single_use` on a fresh key claims it; an immediate second put for the
    /// same key loses (`false`) while the original expiry stays intact.
    pub async fn first_claim_wins_duplicate_loses(repo: &dyn SessionRepository) {
        let key = scenario_key("first_dup");
        let expires_at = Utc::now() + Duration::minutes(10);

        let first = repo
            .put_single_use(&key, expires_at)
            .await
            .expect("first claim on a fresh key must succeed");
        assert!(first, "the very first claim of a key must report success");

        let duplicate = repo
            .put_single_use(&key, Utc::now() + Duration::hours(1))
            .await
            .expect("a duplicate claim must not be an error");
        assert!(
            !duplicate,
            "a second claim of a live key must lose, reporting false"
        );

        // Negative-space guard on state: the loser must not have overwritten the
        // winner's expiry (checked indirectly via take still working on the original).
        let consumed = repo
            .take_single_use(&key)
            .await
            .expect("consume after winning claim");
        assert!(consumed, "the winner's record must remain consumable");
        let again = repo.take_single_use(&key).await.expect("second consume");
        assert!(!again, "consuming twice must burn the record exactly once");
    }

    /// `take_single_use` consumes a live record exactly once: present → true,
    /// immediately-repeated take → false, never-inserted key → false.
    pub async fn consume_live_record_exactly_once(repo: &dyn SessionRepository) {
        let key = scenario_key("consume");
        let absent = scenario_key("consume_absent");

        let claimed = repo
            .put_single_use(&key, Utc::now() + Duration::minutes(5))
            .await
            .expect("claim before consume");
        assert!(claimed, "setup claim must succeed");

        let first_take = repo.take_single_use(&key).await.expect("first take");
        assert!(first_take, "taking a live record must report true");

        let second_take = repo.take_single_use(&key).await.expect("second take");
        assert!(
            !second_take,
            "an already-burned key must be indistinguishable from an absent one"
        );

        let never_existed = repo.take_single_use(&absent).await.expect("absent take");
        assert!(
            !never_existed,
            "taking a key that was never inserted must report false"
        );
    }

    /// An expired record is absent for both operations, without any sweep having run:
    /// `take` refuses it, and a fresh `put` reclaims the key.
    pub async fn expired_record_is_absent_to_put_and_take(repo: &dyn SessionRepository) {
        let key = scenario_key("expiry");

        let claimed = repo
            .put_single_use(&key, Utc::now() + Duration::seconds(SHORT_TTL_SECONDS))
            .await
            .expect("claim with short TTL");
        assert!(claimed, "setup claim must succeed");

        tokio::time::sleep(std::time::Duration::from_millis(EXPIRY_WAIT_MS)).await;

        let taken = repo.take_single_use(&key).await.expect("take after expiry");
        assert!(
            !taken,
            "an expired record must never satisfy take_single_use"
        );

        let reclaimed = repo
            .put_single_use(&key, Utc::now() + Duration::minutes(5))
            .await
            .expect("reclaim after expiry");
        assert!(
            reclaimed,
            "an expired marker's key must be reusable without a sweep"
        );
    }

    /// N concurrent `put_single_use` calls for one key produce exactly one success.
    pub async fn concurrent_put_has_exactly_one_winner(repo: Arc<dyn SessionRepository>) {
        let key = scenario_key("race_put");
        let expires_at = Utc::now() + Duration::minutes(5);

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let repo = Arc::clone(&repo);
            let key = key.clone();
            tasks.spawn(async move { repo.put_single_use(&key, expires_at).await });
        }

        let mut wins = 0usize;
        while let Some(joined) = tasks.join_next().await {
            let result = joined.expect("race task must not panic");
            if result.expect("put_single_use must not error during the race") {
                wins += 1;
            }
        }
        assert_eq!(
            wins, 1,
            "exactly one concurrent claimant may win one live key"
        );
    }

    /// N concurrent `take_single_use` calls for one live record produce exactly one
    /// success — the whole nonce check-and-burn is one atomic operation.
    pub async fn concurrent_take_has_exactly_one_winner(repo: Arc<dyn SessionRepository>) {
        let key = scenario_key("race_take");
        let claimed = repo
            .put_single_use(&key, Utc::now() + Duration::minutes(5))
            .await
            .expect("setup claim");
        assert!(claimed, "setup claim must succeed");

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let repo = Arc::clone(&repo);
            let key = key.clone();
            tasks.spawn(async move { repo.take_single_use(&key).await });
        }

        let mut burns = 0usize;
        while let Some(joined) = tasks.join_next().await {
            let result = joined.expect("race task must not panic");
            if result.expect("take_single_use must not error during the race") {
                burns += 1;
            }
        }
        assert_eq!(
            burns, 1,
            "exactly one concurrent consumer may burn one live record"
        );
    }

    /// On stores without native expiry, `cleanup_expired_sessions` reclaims expired
    /// single-use records, counts sessions and records together, and leaves every live
    /// record and session untouched. Called only by suites whose store needs the sweep
    /// (Postgres, SQLite, LMDB, Mock); native-expiry stores skip it by contract.
    pub async fn cleanup_sweeps_expired_single_use_records(repo: &dyn SessionRepository) {
        use oidc_exchange_core::domain::Session;

        let now = Utc::now();
        let expired_record = scenario_key("cleanup_dead");
        let live_record = scenario_key("cleanup_live");

        // Setup claims: one record already past its expiry, one alive. Writing an
        // already-expired record is legal storage state (it simply reads as absent).
        let setup_expired = repo.put_single_use(&expired_record, now - Duration::minutes(1));
        let setup_live = repo.put_single_use(
            &live_record,
            now + Duration::seconds(SHORT_TTL_SECONDS * 60),
        );
        let _ = tokio::join!(setup_expired, setup_live);

        // One expired and one live session alongside them.
        let make_session = |hash: &str, expires_at: chrono::DateTime<Utc>| Session {
            user_id: format!("usr_su_cleanup_{}", ulid::Ulid::new()),
            refresh_token_hash: hash.to_string(),
            provider: "mock".to_string(),
            expires_at,
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: now,
        };

        repo.store_refresh_token(&make_session(
            "su_cleanup_expired",
            now - Duration::hours(1),
        ))
        .await
        .expect("store expired session");
        repo.store_refresh_token(&make_session("su_cleanup_live", now + Duration::hours(1)))
            .await
            .expect("store live session");

        let removed = repo
            .cleanup_expired_sessions()
            .await
            .expect("cleanup_expired_sessions");
        assert_eq!(
            removed, 2,
            "cleanup must count exactly the expired session plus the expired single-use record"
        );

        let burned = repo
            .take_single_use(&live_record)
            .await
            .expect("take live record after cleanup");
        assert!(burned, "cleanup must not touch a live single-use record");
        let reclaimed = repo
            .take_single_use(&expired_record)
            .await
            .expect("take swept record slot");
        assert!(
            !reclaimed,
            "the expired record must be physically gone after the sweep"
        );

        // Live-session behaviour is unchanged by the sweep.
        let active = repo.count_active_sessions().await.expect("count sessions");
        assert_eq!(active, 1, "exactly the live session survives the sweep");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::single_use_conformance as conformance;
    use super::{MockIdentityProvider, MockRepository, MOCK_CLIENT_ID};
    use oidc_exchange_core::domain::{NewUser, UserPatch, INITIAL_USER_VERSION};
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

    /// The shared single-use conformance suite, run against `MockRepository`: the mock
    /// must satisfy exactly the atomic-claim contract the real adapters do.
    #[tokio::test]
    async fn single_use_first_claim_wins_duplicate_loses() {
        conformance::first_claim_wins_duplicate_loses(&MockRepository::new()).await;
    }

    /// The mock reports its configured client identity through the port, defaulting to
    /// the shared fixture audience and honouring the builder override — binding tests
    /// rely on both behaviours matching the real providers.
    #[tokio::test]
    async fn mock_identity_provider_reports_configured_client_id() {
        use oidc_exchange_core::ports::IdentityProvider;

        let default_provider = MockIdentityProvider::new("mock");
        assert_eq!(
            IdentityProvider::client_id(&default_provider),
            MOCK_CLIENT_ID,
            "the mock's default client_id must match the documented fixture constant"
        );
        assert_eq!(default_provider.provider_id(), "mock");

        let custom_provider = MockIdentityProvider::new("mock").with_client_id("sibling-client");
        assert_eq!(
            IdentityProvider::client_id(&custom_provider),
            "sibling-client",
            "with_client_id must override the audience the port reports"
        );

        // The mock's default claims carry a signing_alg a real validator would report,
        // so core-side consumers never see an empty algorithm from the fixture.
        let claims = default_provider
            .validate_id_token("unused")
            .await
            .expect("mock validate should succeed");
        assert_eq!(claims.signing_alg, "RS256");
    }

    #[tokio::test]
    async fn single_use_consume_live_record_exactly_once() {
        conformance::consume_live_record_exactly_once(&MockRepository::new()).await;
    }

    #[tokio::test]
    async fn single_use_expired_record_is_absent_to_put_and_take() {
        conformance::expired_record_is_absent_to_put_and_take(&MockRepository::new()).await;
    }

    #[tokio::test]
    async fn single_use_concurrent_put_has_exactly_one_winner() {
        conformance::concurrent_put_has_exactly_one_winner(std::sync::Arc::new(
            MockRepository::new(),
        ))
        .await;
    }

    #[tokio::test]
    async fn single_use_concurrent_take_has_exactly_one_winner() {
        conformance::concurrent_take_has_exactly_one_winner(std::sync::Arc::new(
            MockRepository::new(),
        ))
        .await;
    }

    #[tokio::test]
    async fn single_use_cleanup_sweeps_expired_records_and_counts_both_kinds() {
        conformance::cleanup_sweeps_expired_single_use_records(&MockRepository::new()).await;
    }

    /// A losing duplicate put must not disturb the winner's expiry: after a second put
    /// loses, the record still expires at the winner's (earlier) instant, observable
    /// through `get_single_use_record`.
    #[tokio::test]
    async fn single_use_duplicate_put_preserves_original_expiry() {
        let repo = MockRepository::new();
        let key = "nonce:expiry_guard";
        let winner_expiry = chrono::Utc::now() + chrono::Duration::minutes(10);

        let first = repo
            .put_single_use(key, winner_expiry)
            .await
            .expect("first");
        assert!(first);
        let loser = repo
            .put_single_use(key, chrono::Utc::now() + chrono::Duration::hours(24))
            .await
            .expect("duplicate claim is not an error");
        assert!(!loser, "the duplicate must lose");

        let stored = repo
            .get_single_use_record(key)
            .await
            .expect("record survives a losing overwrite attempt");
        assert_eq!(
            stored.expires_at, winner_expiry,
            "a losing put must not extend or replace the winner's expiry"
        );
        assert_eq!(stored.key, key);
    }
}
