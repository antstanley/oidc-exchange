use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use oidc_exchange_core::config::DEFAULT_REFRESH_REUSE_RETENTION;
use oidc_exchange_core::cursor::KeysetCursor;
use oidc_exchange_core::domain::{
    is_valid_family_id, AuditEvent, IdentityClaims, NewUser, ProviderTokens,
    RateLimitDecision, RateLimitKey, RefreshResolution, RetiredRefreshToken, Session,
    SingleUseRecord, User, UserPage, UserPatch, UserStatus, INITIAL_USER_VERSION,
    MAX_ADMIN_PAGE_SIZE,
};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::secret::Secret;
use oidc_exchange_core::ports::{
    AuditLog, IdentityProvider, KeyManager, RateLimiter, SessionRepository, UserRepository,
    UserSync,
};

pub mod corpus;

pub mod telemetry;

pub mod session_contract;


// ---------------------------------------------------------------------------
// MockRepository
// ---------------------------------------------------------------------------

struct MockRepositoryState {
    users: HashMap<String, User>,
    /// Live session generations, keyed by refresh-token hash. At most one
    /// generation of a family lives at any instant.
    sessions: HashMap<String, Session>,
    /// Single-use records (nonces, assertion-replay markers) keyed by their namespaced
    /// digest. Shared by every `MockRepository` clone of one instance, so claim
    /// operations are atomic exactly as against a real store.
    single_use: HashMap<String, DateTime<Utc>>,
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
                single_use: HashMap::new(),
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
        sessions.sort_by(|a, b| a.refresh_token_hash.expose().cmp(b.refresh_token_hash.expose()));
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

    /// Test infrastructure: rewind one retirement record's `retired_at` by
    /// `seconds` so a flow test can place it deterministically outside the
    /// grace window without sleeping. Returns whether a record named by
    /// `token_hash` existed and was rewritten; callers assert on it.
    pub async fn backdate_retirement(&self, token_hash: &str, seconds: u64) -> bool {
        assert!(seconds > 0, "backdating by zero seconds is a no-op bug");
        let mut state = self.state.lock().await;
        match state.retired.get_mut(token_hash) {
            Some(record) => {
                record.retired_at -= chrono::Duration::seconds(seconds as i64);
                true
            }
            None => false,
        }
    }

    /// Toggle whether `revoke_session`, `revoke_family`, and
    /// `revoke_all_user_sessions` fail with `Error::StoreError`, simulating an
    /// unreachable session store.
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

    /// Keyset-paginated listing ordered `created_at DESC, id DESC` — the same
    /// ordering contract as the SQL adapters — resuming strictly after the
    /// decoded cursor's position and peeking one row past the limit so
    /// termination is exact (a short page is always the last page).
    async fn list_users(&self, cursor: Option<&str>, limit: u32) -> Result<UserPage> {
        assert!(
            (1..=MAX_ADMIN_PAGE_SIZE).contains(&limit),
            "mock list_users expects a pre-clamped limit within 1..={MAX_ADMIN_PAGE_SIZE}, got {limit}"
        );

        let resume_after = cursor.map(KeysetCursor::decode).transpose()?;

        let state = self.state.lock().await;
        let mut users: Vec<User> = state.users.values().cloned().collect();
        users.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));

        // Strictly-after semantics on the composite key: a row deleted between
        // pages neither duplicates nor skips its neighbours.
        let remaining: Vec<&User> = match &resume_after {
            Some(position) => users
                .iter()
                .filter(|u| {
                    (u.created_at, u.id.as_str()) < (position.created_at, position.id.as_str())
                })
                .collect(),
            None => users.iter().collect(),
        };

        let take = limit as usize;
        let has_more = remaining.len() > take;
        let page: Vec<User> = remaining.iter().take(take).map(|u| (*u).clone()).collect();

        let next_cursor = if has_more {
            let last = page.last().expect("has_more implies a non-empty page");
            assert!(
                page.len() == take,
                "a continued page must carry exactly the requested limit"
            );
            Some(KeysetCursor::new(last.created_at, last.id.clone()).encode())
        } else {
            assert!(
                remaining.len() <= take,
                "no continuation may be signalled when every remaining row fit"
            );
            None
        };

        Ok(UserPage {
            users: page,
            next_cursor,
        })
    }
}

#[async_trait]
impl SessionRepository for MockRepository {
    async fn store_refresh_token(&self, session: &Session) -> Result<()> {
        // Precondition: callers mint well-formed family ids; a malformed one
        // here is a programmer error, not data to store silently. The
        // empty-string sentinel is the one non-well-formed value accepted: it
        // is how every backend represents a pre-rotation (legacy) row, and the
        // persistent adapters must be able to hold one just like this store.
        assert!(
            session.family_id.is_empty() || is_valid_family_id(&session.family_id),
            "store_refresh_token: malformed family id {:?}",
            session.family_id
        );
        assert!(
            !session.refresh_token_hash.expose().is_empty(),
            "store_refresh_token: refresh_token_hash must not be empty"
        );

        let mut state = self.state.lock().await;
        state
            .sessions
            .insert(session.refresh_token_hash.expose().clone(), session.clone());
        Ok(())
    }

    async fn get_session_by_refresh_token(
        &self,
        token_hash: &Secret<String>,
    ) -> Result<Option<Session>> {
        if *self.session_lookup_fail_mode.lock().await {
            return Err(Error::StoreError {
                detail: "mock session store lookup failure".into(),
            });
        }
        let state = self.state.lock().await;
        Ok(state.sessions.get(token_hash.expose()).cloned())
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
        assert!(
            live_hash != replacement.refresh_token_hash.expose().as_str(),
            "rotate_refresh_token: replacement must be a fresh generation"
        );

        let mut state = self.state.lock().await;

        // CAS condition: the named hash must still be a live generation.
        let Some(live) = state.sessions.get(live_hash) else {
            return Ok(false);
        };
        // Precondition on well-formed rows: a rotation replaces a generation
        // of the same family for the same user — anything else would strand
        // credentials in a family their holder no longer controls. A legacy
        // row (empty-family sentinel, written before rotation shipped) is the
        // one exception: it belongs to no family, so its first redemption
        // swaps to the caller's newly-minted family without a retirement
        // record (there is no prior generation to detect reuse against) and
        // only user identity is asserted.
        let legacy_row = live.family_id.is_empty();
        if !legacy_row {
            assert_eq!(
                live.family_id, replacement.family_id,
                "rotate_refresh_token: replacement family {:?} must match live family {:?}",
                replacement.family_id, live.family_id
            );
        }
        assert_eq!(
            live.user_id, replacement.user_id,
            "rotate_refresh_token: replacement user {:?} must match live user {:?}",
            replacement.user_id, live.user_id
        );
        assert!(
            !state.sessions.contains_key(replacement.refresh_token_hash.expose()),
            "rotate_refresh_token: replacement hash already exists as a live session"
        );
        assert!(
            !state.retired.contains_key(replacement.refresh_token_hash.expose()),
            "rotate_refresh_token: replacement hash already exists as a retired record"
        );

        let now = Utc::now();
        if !legacy_row {
            // A legacy row produces no retirement record: there is no prior
            // generation to detect reuse against (the source spec's SQL
            // first-redemption rule, mirrored here so wave-C flow tests
            // behave identically on the mock and the persistent stores).
            let retired_record = RetiredRefreshToken {
                refresh_token_hash: live_hash.to_string(),
                family_id: replacement.family_id.clone(),
                user_id: replacement.user_id.clone(),
                successor_hash: replacement.refresh_token_hash.expose().clone(),
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
        }
        state.sessions.remove(live_hash);
        state
            .sessions
            .insert(replacement.refresh_token_hash.expose().clone(), replacement.clone());

        Ok(true)
    }

    async fn revoke_session(&self, token_hash: &Secret<String>) -> Result<()> {
        let token_hash = token_hash.expose().as_str();
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

        // The sweep also reclaims expired single-use records (space reclamation only —
        // the claim operations already treat an expired record as absent), and the
        // returned count covers both kinds, per the port contract.
        let single_use_before = state.single_use.len();
        state.single_use.retain(|_, expires_at| *expires_at > now);
        let single_use_removed = (single_use_before - state.single_use.len()) as u64;

        debug_assert!(
            sessions_removed as usize <= sessions_before,
            "removed session count cannot exceed the pre-sweep size"
        );
        Ok(sessions_removed + retired_removed + single_use_removed)
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

    /// Test infrastructure: sign an arbitrary JSON payload into a compact
    /// three-part JWS with the same header shape and encoding
    /// (`{"alg":"EdDSA","typ":"JWT"}`, base64url-no-pad) the service mints,
    /// so tests can present validly-signed tokens carrying claim values the
    /// service itself would never issue (e.g. hash-form `sid`s from before a
    /// rotation-capable build) without reconstructing signing keys.
    pub fn sign_payload_jws(&self, payload: &serde_json::Value) -> String {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        use ed25519_dalek::Signer;

        let header_b64 = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","typ":"JWT"}"#);
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        let signing_input = format!("{header_b64}.{payload_b64}");
        let signature = self.signing_key.sign(signing_input.as_bytes());
        format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        )
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
// MockRateLimiter
// ---------------------------------------------------------------------------

/// Fixed Retry-After (seconds) the mock reports while in deny mode.
pub const MOCK_RETRY_AFTER_SECS: u64 = 60;

/// A scriptable rate limiter for tests. By default it allows everything.
/// `set_decisions`/`keys`/`set_fail_mode` script the exchange plane's
/// `check_and_consume`; `deny_mode` plus the check/consume call counters
/// script the admin plane's consult-then-consume contract, so auth-layer
/// tests can assert exactly *when* the budget is consulted and drawn down.
#[derive(Clone)]
pub struct MockRateLimiter {
    decisions: Arc<Mutex<Vec<RateLimitDecision>>>,
    keys: Arc<Mutex<Vec<RateLimitKey>>>,
    fail_mode: Arc<Mutex<bool>>,
    deny_mode: Arc<Mutex<bool>>,
    check_calls: Arc<Mutex<u64>>,
    consume_calls: Arc<Mutex<u64>>,
}

impl MockRateLimiter {
    pub fn new() -> Self {
        Self {
            decisions: Arc::new(Mutex::new(Vec::new())),
            keys: Arc::new(Mutex::new(Vec::new())),
            fail_mode: Arc::new(Mutex::new(false)),
            deny_mode: Arc::new(Mutex::new(false)),
            check_calls: Arc::new(Mutex::new(0)),
            consume_calls: Arc::new(Mutex::new(0)),
        }
    }

    pub async fn set_decisions(&self, decisions: Vec<RateLimitDecision>) {
        *self.decisions.lock().await = decisions;
    }

    pub async fn keys(&self) -> Vec<RateLimitKey> {
        self.keys.lock().await.clone()
    }

    pub async fn set_fail_mode(&self, fail: bool) {
        *self.fail_mode.lock().await = fail;
    }

    /// Force every subsequent `check`/`consume` to return `Deny`.
    pub async fn set_deny_mode(&self, deny: bool) {
        *self.deny_mode.lock().await = deny;
    }

    /// How many times [`RateLimiter::check`] has been invoked.
    pub async fn check_calls(&self) -> u64 {
        *self.check_calls.lock().await
    }

    /// How many times [`RateLimiter::consume`] has been invoked.
    pub async fn consume_calls(&self) -> u64 {
        *self.consume_calls.lock().await
    }

    async fn deny_or_allow(&self) -> RateLimitDecision {
        if *self.deny_mode.lock().await {
            RateLimitDecision::Deny {
                retry_after_secs: MOCK_RETRY_AFTER_SECS,
            }
        } else {
            RateLimitDecision::Allow
        }
    }
}

impl Default for MockRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RateLimiter for MockRateLimiter {
    async fn check_and_consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision> {
        if *self.fail_mode.lock().await {
            return Err(Error::StoreError {
                detail: "mock rate limiter failure".into(),
            });
        }
        self.keys.lock().await.push(key.clone());
        Ok(self
            .decisions
            .lock()
            .await
            .pop()
            .unwrap_or(RateLimitDecision::Allow))
    }

    async fn check(&self, _key: &RateLimitKey) -> Result<RateLimitDecision> {
        *self.check_calls.lock().await += 1;
        Ok(self.deny_or_allow().await)
    }

    async fn consume(&self, _key: &RateLimitKey) -> Result<RateLimitDecision> {
        *self.consume_calls.lock().await += 1;
        Ok(self.deny_or_allow().await)
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
/// exchange responses afterwards — and the shared per-method call counters
/// make the clone work as an observation handle.
#[derive(Clone)]
pub struct MockIdentityProvider {
    provider_id: String,
    /// The audience the mock's claims are pinned to, reported through the port's
    /// `client_id()`; configurable so binding tests can exercise `azp` mismatches.
    client_id: String,
    exchange_response: Arc<Mutex<Option<ProviderTokens>>>,
    exchange_error: Arc<Mutex<Option<String>>>,
    exchange_timeout: Arc<Mutex<bool>>,
    claims_response: Arc<Mutex<Option<IdentityClaims>>>,
    /// Monotonic counters, one per port method, so tests can prove a request
    /// path never reached the provider (e.g. grant-confusion rejections must
    /// die at the HTTP boundary before either exchange operation runs).
    exchange_code_calls: Arc<AtomicU32>,
    validate_id_token_calls: Arc<AtomicU32>,
    revoke_token_calls: Arc<AtomicU32>,
}

/// Snapshot of [`MockIdentityProvider`] call counters at read time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MockIdentityProviderCallCounts {
    pub exchange_code: u32,
    pub validate_id_token: u32,
    pub revoke_token: u32,
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
            exchange_error: Arc::new(Mutex::new(None)),
            exchange_timeout: Arc::new(Mutex::new(false)),
            exchange_code_calls: Arc::new(AtomicU32::new(0)),
            validate_id_token_calls: Arc::new(AtomicU32::new(0)),
            revoke_token_calls: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Point-in-time copy of the per-method call counters. Counters are shared
    /// through `Arc`, so clones observe the same totals.
    pub fn call_counts(&self) -> MockIdentityProviderCallCounts {
        MockIdentityProviderCallCounts {
            exchange_code: self.exchange_code_calls.load(Ordering::SeqCst),
            validate_id_token: self.validate_id_token_calls.load(Ordering::SeqCst),
            revoke_token: self.revoke_token_calls.load(Ordering::SeqCst),
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

    pub async fn set_exchange_error(&self, detail: impl Into<String>) {
        *self.exchange_error.lock().await = Some(detail.into());
    }

    /// Make the next exchange fail as an OAuth invalid_grant credential rejection.
    pub async fn set_invalid_grant(&self) {
        *self.exchange_error.lock().await = Some("invalid_grant".into());
    }

    /// Toggle a typed upstream timeout for exchange-code test paths.
    pub async fn set_exchange_timeout(&self, timeout: bool) {
        *self.exchange_timeout.lock().await = timeout;
    }

    pub async fn exchange_code_call_count(&self) -> usize {
        self.exchange_code_calls.load(Ordering::SeqCst) as usize
    }

    pub async fn validate_id_token_call_count(&self) -> usize {
        self.validate_id_token_calls.load(Ordering::SeqCst) as usize
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
        self.exchange_code_calls.fetch_add(1, Ordering::SeqCst);
        if *self.exchange_timeout.lock().await {
            return Err(Error::ProviderTimeout {
                provider: self.provider_id.clone(),
            });
        }
        if let Some(detail) = self.exchange_error.lock().await.clone() {
            if detail == "invalid_grant" {
                return Err(Error::InvalidGrant {
                    reason: "provider rejected credentials".into(),
                });
            }
            return Err(Error::ProviderError {
                provider: self.provider_id.clone(),
                detail,
            });
        }
        let response = self.exchange_response.lock().await;
        // Port-contract postcondition on the double itself: an empty id_token
        // here would silently invalidate every downstream assertion about
        // what the flow validated.
        let tokens = response.clone().unwrap_or(ProviderTokens {
            id_token: "mock-id-token".to_string(),
            refresh_token: None,
            access_token: None,
        });
        assert!(
            !tokens.id_token.is_empty(),
            "MockIdentityProvider::exchange_code must never produce an empty id_token"
        );
        Ok(tokens)
    }

    async fn validate_id_token(&self, _id_token: &str) -> Result<IdentityClaims> {
        self.validate_id_token_calls.fetch_add(1, Ordering::SeqCst);
        let response = self.claims_response.lock().await;
        let claims = response
            .clone()
            .unwrap_or_else(MockIdentityProvider::default_claims);
        assert!(
            !claims.subject.is_empty(),
            "MockIdentityProvider::validate_id_token must never produce an empty subject"
        );
        Ok(claims)
    }

    async fn revoke_token(&self, _token: &str) -> Result<()> {
        self.revoke_token_calls.fetch_add(1, Ordering::SeqCst);
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
            family_id: oidc_exchange_core::domain::new_family_id(),
            generation: 0,
            rotated_at: None,
            refresh_token_hash: oidc_exchange_core::secret::Secret::new(hash.to_string()),
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
    use crate::session_contract;
    use oidc_exchange_core::domain::{
        NewUser, RetiredRefreshToken, UserPatch, INITIAL_USER_VERSION, MAX_ADMIN_PAGE_SIZE,
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
            refresh_token_hash: oidc_exchange_core::secret::Secret::new(hash.to_string()),
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

    // -----------------------------------------------------------------------
    // SR1–SR5 conformance — MockRepository runs the shared suite
    // -----------------------------------------------------------------------
    //
    // Each obligation's named assertion lives in
    // `session_contract` and is generic over any `SessionRepository`; the
    // tests below invoke it against `MockRepository`, one test per assertion,
    // so a regression localizes to its obligation. `conformance_full_suite`
    // additionally proves the orchestrator wires the whole set.

    /// SR1 classification surface: Live → Superseded → Retired → Unknown.
    #[tokio::test]
    async fn conformance_classification_all_four_shapes() {
        session_contract::assert_resolution_classifies_all_four_shapes(
            &MockRepository::new(),
            "mock:classify",
        )
        .await;
    }

    /// SR2 observable effects: successor installed, presented demoted.
    #[tokio::test]
    async fn conformance_rotation_installs_successor_and_demotes_presented() {
        session_contract::assert_rotation_installs_successor_and_demotes_presented(
            &MockRepository::new(),
            "mock:install",
        )
        .await;
    }

    /// SR2 negative space: a losing CAS is invisible through the port.
    #[tokio::test]
    async fn conformance_failed_cas_leaves_store_byte_identical() {
        session_contract::assert_failed_cas_leaves_store_byte_identical(
            &MockRepository::new(),
            "mock:cas-port",
        )
        .await;
    }

    /// SR3: two concurrent rotations, exactly one winner, store agrees.
    #[tokio::test]
    async fn conformance_concurrent_rotation_yields_exactly_one_winner() {
        session_contract::assert_concurrent_rotation_yields_exactly_one_winner(
            &MockRepository::new(),
            "mock:race",
        )
        .await;
    }

    /// SR4: the retirement record is readable the instant the rotation is.
    #[tokio::test]
    async fn conformance_retirement_readable_immediately_after_rotation() {
        session_contract::assert_retirement_readable_immediately_after_rotation(
            &MockRepository::new(),
            "mock:sr4",
        )
        .await;
    }

    /// SR1/SR4 retained history: an older generation resolves Retired.
    #[tokio::test]
    async fn conformance_older_generation_resolves_as_retired() {
        session_contract::assert_older_generation_resolves_as_retired(
            &MockRepository::new(),
            "mock:older",
        )
        .await;
    }

    /// SR5: family revocation removes everything and returns the count.
    #[tokio::test]
    async fn conformance_family_revocation_removes_everything_and_returns_count() {
        session_contract::assert_family_revocation_removes_everything_and_returns_count(
            &MockRepository::new(),
            "mock:revoke",
        )
        .await;
    }

    /// SR1 negative space: Unknown immediately after revoke_session.
    #[tokio::test]
    async fn conformance_resolution_unknown_immediately_after_revoke() {
        session_contract::assert_resolution_unknown_immediately_after_revoke(
            &MockRepository::new(),
            "mock:sr1-revoke",
        )
        .await;
    }

    /// Expiry inheritance: rotation never moves the absolute deadline.
    #[tokio::test]
    async fn conformance_rotation_preserves_absolute_expiry() {
        session_contract::assert_rotation_preserves_absolute_expiry(
            &MockRepository::new(),
            "mock:expiry",
        )
        .await;
    }

    /// The whole suite through one orchestrator call — exactly what a
    /// persistent adapter invokes once its implementation is complete.
    #[tokio::test]
    async fn conformance_full_suite() {
        let repo = MockRepository::new();
        session_contract::assert_full_conformance(&repo, "mock:full").await;
    }

    /// A losing compare-and-swap must be a complete no-op. The shared suite
    /// proves this through the port surface; this mock-only companion
    /// snapshots the store's *internal* maps (every live row and every
    /// retirement record, not just the hashes in play) for a literal
    /// byte-identical check.
    #[tokio::test]
    async fn failed_cas_leaves_internal_maps_byte_identical() {
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
                .any(|r| r.refresh_token_hash == *stale_replacement.refresh_token_hash.expose()),
            "the loser's replacement must never appear as a retirement record"
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
        assert!(
            records[0].successor_hash == *gen1.refresh_token_hash.expose(),
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
                .all(|s| s.refresh_token_hash.expose() != "hash_expired"),
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

    /// Retirement records carry session lookup keys: their hand-written
    /// `Debug` redacts both hashes while keeping the correlating identifiers.
    #[test]
    fn retired_record_debug_redacts_its_hashes() {
        let record = RetiredRefreshToken {
            refresh_token_hash: "hash_retired".to_string(),
            family_id: format!("fam_{FAMILY_A}"),
            user_id: "usr_x".to_string(),
            successor_hash: "hash_successor".to_string(),
            retired_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(24),
        };
        let debug = format!("{record:?}");
        assert!(!debug.contains("hash_retired"));
        assert!(!debug.contains("hash_successor"));
        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("usr_x"));
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

    // ── Bounded cursor pagination (task 08): mock keyset traversal ──────────

    /// Seed `count` users and return nothing; ids are unique per call.
    async fn seed_users(repo: &MockRepository, count: usize, tag: &str) {
        for i in 0..count {
            repo.create_user(&NewUser {
                external_id: format!("{tag}-{i}"),
                provider: "mock".to_string(),
                email: Some(format!("{tag}-{i}@example.com")),
                display_name: None,
            })
            .await
            .expect("seed create_user");
        }
    }

    /// Walk every page at `limit`, returning (all row ids in page order,
    /// per-page sizes). Fails if the traversal does not terminate.
    async fn walk(repo: &MockRepository, limit: u32) -> (Vec<String>, Vec<usize>) {
        let mut seen: Vec<String> = Vec::new();
        let mut sizes: Vec<usize> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0;
        loop {
            pages += 1;
            assert!(pages <= 1000, "traversal must terminate");
            let page = repo
                .list_users(cursor.as_deref(), limit)
                .await
                .expect("each page succeeds");
            assert!(
                page.users.len() <= limit as usize,
                "a page never exceeds its limit"
            );
            sizes.push(page.users.len());
            seen.extend(page.users.iter().map(|u| u.id.clone()));
            match page.next_cursor {
                Some(next) => {
                    assert!(!next.is_empty(), "an issued cursor is a non-empty token");
                    cursor = Some(next);
                }
                None => break,
            }
        }
        (seen, sizes)
    }

    #[tokio::test]
    async fn pagination_covers_every_user_exactly_once_across_adjacent_pages() {
        let repo = MockRepository::new();
        seed_users(&repo, 11, "trav").await;

        let (seen, sizes) = walk(&repo, 3).await;

        // Exact-count traversal: no duplicates, no skips.
        assert_eq!(seen.len(), 11, "every user is returned exactly once");
        assert_eq!(
            seen.iter().collect::<std::collections::HashSet<_>>().len(),
            11,
            "no id repeats across adjacent pages"
        );
        assert_eq!(
            sizes,
            vec![3, 3, 3, 2],
            "full pages then one short final page"
        );
    }

    #[tokio::test]
    async fn ordering_is_stable_across_independent_traversals() {
        let repo = MockRepository::new();
        seed_users(&repo, 9, "ord").await;

        let (first, _) = walk(&repo, 4).await;
        let (second, _) = walk(&repo, 4).await;

        assert_eq!(first.len(), 9);
        assert_eq!(
            first, second,
            "two independent traversals must visit rows in the same order"
        );
    }

    #[tokio::test]
    async fn short_final_page_and_exact_fit_both_terminate_with_null_cursor() {
        // Short final page: 5 users at limit 2 = 2+2+1.
        let repo = MockRepository::new();
        seed_users(&repo, 5, "short").await;
        let (_, sizes) = walk(&repo, 2).await;
        assert_eq!(sizes, vec![2, 2, 1]);

        // Exact fit: 6 users at limit 2 — the peek must not invent an
        // empty trailing page nor carry a dangling cursor.
        let repo = MockRepository::new();
        seed_users(&repo, 6, "exact").await;
        let (seen, sizes) = walk(&repo, 2).await;
        assert_eq!(sizes, vec![2, 2, 2], "no empty trailing page");
        assert_eq!(seen.len(), 6);
    }

    #[tokio::test]
    async fn single_page_when_the_limit_covers_the_whole_listing() {
        let repo = MockRepository::new();
        seed_users(&repo, 4, "one").await;

        let page = repo
            .list_users(None, MAX_ADMIN_PAGE_SIZE)
            .await
            .expect("page");
        assert_eq!(page.users.len(), 4);
        assert!(
            page.next_cursor.is_none(),
            "everything fit, so the listing is exhausted"
        );
    }

    #[tokio::test]
    async fn empty_store_returns_an_empty_page_with_null_cursor() {
        let repo = MockRepository::new();

        let page = repo.list_users(None, 10).await.expect("empty page");
        assert!(page.users.is_empty());
        assert!(page.next_cursor.is_none(), "empty listing is exhausted");
    }

    #[tokio::test]
    async fn tampered_cursor_is_invalid_request_not_a_first_page() {
        let repo = MockRepository::new();
        seed_users(&repo, 3, "tamper").await;

        for bad_cursor in ["garbage", "", "aGVsbG8="] {
            let err = repo
                .list_users(Some(bad_cursor), 10)
                .await
                .expect_err("tampered cursors are rejected");
            match err {
                Error::InvalidRequest { .. } => {}
                other => panic!("expected InvalidRequest for {bad_cursor:?}, got {other:?}"),
            }
        }

        // The negative path must not have disturbed the positive one.
        let page = repo.list_users(None, 10).await.expect("valid page");
        assert_eq!(page.users.len(), 3);
    }

    #[tokio::test]
    async fn deleting_the_cursor_row_mid_traversal_neither_duplicates_nor_skips() {
        let repo = MockRepository::new();
        seed_users(&repo, 6, "del").await;

        let page_one = repo.list_users(None, 2).await.expect("page one");
        assert_eq!(page_one.users.len(), 2);
        let cursor_row_id = page_one.users[1].id.clone();
        let cursor = page_one.next_cursor.as_ref().expect("more pages remain");

        // Delete exactly the row the cursor points at, then continue.
        repo.delete_user(&cursor_row_id)
            .await
            .expect("delete cursor row");

        let mut rest: Vec<String> = Vec::new();
        let mut cursor = Some(cursor.clone());
        while let Some(c) = cursor {
            let page = repo
                .list_users(Some(&c), 2)
                .await
                .expect("continuation succeeds");
            rest.extend(page.users.iter().map(|u| u.id.clone()));
            cursor = page.next_cursor;
        }

        // Six seeded, one deleted: five distinct ids are ever observable.
        // The four rows strictly after the deleted cursor position must all
        // come back — none skipped by the removal.
        assert_eq!(rest.len(), 4, "every row after the deleted cursor position");
        let neighbour_id = &page_one.users[0].id;
        assert!(
            !rest.contains(neighbour_id),
            "the first-page neighbour sorts before the cursor and must never recur"
        );
        let mut observed: Vec<String> = page_one.users.iter().map(|u| u.id.clone()).collect();
        observed.extend(rest);
        assert_eq!(
            observed
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            6,
            "every seed is observed at most once across the whole traversal"
        );
        // The deleted row's single appearance was page one itself, before its
        // deletion; `rest`'s exclusion of it is asserted above.
    }
}
