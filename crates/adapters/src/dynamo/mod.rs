pub mod schema;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as TokioMutex;

use async_trait::async_trait;
use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
use aws_sdk_dynamodb::types::{
    AttributeValue, Delete, DeleteRequest, Put, ReturnConsumedCapacity, TransactWriteItem, Update,
    WriteRequest,
};
use chrono::{DateTime, Utc};
use oidc_exchange_core::domain::{
    is_valid_family_id, NewUser, RefreshResolution, RetiredRefreshToken, Session, User, UserPage,
    UserPatch, UserStatus, INITIAL_USER_VERSION, MAX_ADMIN_PAGE_SIZE,
};
use tracing::instrument;

use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::{SessionRepository, UserRepository};
use oidc_exchange_core::secret::Secret;

use schema::{
    guard_pk, guard_to_item, item_single_use_expiry, item_to_retired, item_to_session,
    item_to_user, retired_to_item, session_to_item, single_use_pk, single_use_to_item,
    user_to_item, FamilyRoster, UserRoster, GUARD_SK, RETIRED_SK,
};

/// DynamoDB cancellation-reason code reported for a failed `attribute_not_exists(pk)`
/// condition inside a `TransactWriteItems` call — the signal that a `create_user` lost a
/// uniqueness race, mapped to `Error::Conflict` rather than `Error::StoreError`.
const CONDITIONAL_CHECK_FAILED_CODE: &str = "ConditionalCheckFailed";

/// Partition key of the transactionally-maintained user-status counter item
/// (`count_by_status` reads this item instead of scanning the table).
pub const STATS_COUNTER_PK: &str = "STATS#USERS";

/// Sort key of the user-status counter item.
pub const STATS_COUNTER_SK: &str = "COUNTS";

/// Stats-cache TTL used when a deployment has not configured
/// `internal_api.stats_cache_ttl` — the value the configuration spec documents
/// as the default.
pub const DEFAULT_STATS_CACHE_TTL: Duration = Duration::from_secs(60);

/// Upper bound on the stats-cache TTL: beyond one hour the "active sessions"
/// figure on the dashboard would be an audit-grade lie rather than a cached
/// estimate. `AppConfig::validate` refuses larger values.
pub const MAX_STATS_CACHE_TTL: Duration = Duration::from_secs(3600);

// ---------------------------------------------------------------------------
// Scan-cursor codec
// ---------------------------------------------------------------------------

/// Opaque wire form of the list-users cursor: the base64url-encoded JSON of
/// the scan's `LastEvaluatedKey` (`pk`/`sk` string attributes). Adapter-defined
/// and only decodable by this adapter — a SQL keyset cursor handed to the
/// DynamoDB adapter fails here with `InvalidRequest`, never silently starts
/// over from the first page.
struct ScanCursor {
    pk: String,
    sk: String,
}

impl ScanCursor {
    fn from_key_map(key: &HashMap<String, AttributeValue>) -> Self {
        let get_s = |name: &str| -> String {
            let value = key
                .get(name)
                .and_then(|v| v.as_s().ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    panic!("a LastEvaluatedKey must carry a string attribute named {name}")
                });
            assert!(
                !value.is_empty(),
                "a LastEvaluatedKey must carry a non-empty {name}"
            );
            value
        };
        Self {
            pk: get_s("pk"),
            sk: get_s("sk"),
        }
    }

    fn to_key_map(&self) -> HashMap<String, AttributeValue> {
        assert!(!self.pk.is_empty(), "decoded cursor carries an empty pk");
        assert!(!self.sk.is_empty(), "decoded cursor carries an empty sk");
        HashMap::from([
            ("pk".to_string(), AttributeValue::S(self.pk.clone())),
            ("sk".to_string(), AttributeValue::S(self.sk.clone())),
        ])
    }

    fn encode(&self) -> String {
        let json = serde_json::json!({ "pk": self.pk, "sk": self.sk }).to_string();
        use base64::engine::general_purpose::URL_SAFE_NO_PAD as ENCODING;
        use base64::Engine as _;
        let encoded = ENCODING.encode(json.as_bytes());
        // Round-trip assertion: what we hand out must decode to the same key.
        let decoded = Self::decode(&encoded).expect("encode output must decode cleanly");
        assert_eq!(decoded.pk, self.pk, "cursor round-trip must preserve pk");
        assert_eq!(decoded.sk, self.sk, "cursor round-trip must preserve sk");
        encoded
    }

    /// Parse a caller-supplied cursor; any structural failure is
    /// `Error::InvalidRequest` (a tampered or foreign cursor is a caller
    /// fault, never a store error).
    fn decode(raw: &str) -> Result<Self> {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD as ENCODING;
        use base64::Engine as _;

        if raw.is_empty() {
            return Err(Error::InvalidRequest {
                reason: "cursor must be a non-empty opaque token".to_string(),
            });
        }
        let json_bytes = ENCODING
            .decode(raw.as_bytes())
            .map_err(|_| Error::InvalidRequest {
                reason: "cursor is not a valid opaque token".to_string(),
            })?;
        let parsed: serde_json::Value =
            serde_json::from_slice(&json_bytes).map_err(|_| Error::InvalidRequest {
                reason: "cursor is not a valid opaque token".to_string(),
            })?;
        let pk = parsed
            .get("pk")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or(Error::InvalidRequest {
                reason: "cursor is not a valid opaque token".to_string(),
            })?;
        let sk = parsed
            .get("sk")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or(Error::InvalidRequest {
                reason: "cursor is not a valid opaque token".to_string(),
            })?;
        Ok(Self {
            pk: pk.to_string(),
            sk: sk.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// STATS#USERS/COUNTS counter maintenance
// ---------------------------------------------------------------------------

/// Builds the transactional `Update` that moves this table's user-status
/// counter item by one row: decrementing the status the row is leaving (when
/// `from` is `Some`) and incrementing the one it is entering, in a single
/// atomic item update.
///
/// `ADD` treats missing attributes — and a wholly missing counter item — as
/// zero and creates what it touches, so the very first user write on a table
/// bootstraps the counter without a separate migration step, and a racing
/// writer that creates the item concurrently still composes correctly. The
/// update rides inside the *same* `TransactWriteItems` call as the profile
/// and guard writes, so the counters can never record a state no committed
/// write produced (08-persistence.md): a cancelled transaction rolls its
/// counter adjustment back together with everything else.
///
/// The status names go through expression-attribute-name placeholders because
/// several appear in DynamoDB's reserved-word list.
fn counter_adjustment(
    table_name: &str,
    from: Option<&UserStatus>,
    to: &UserStatus,
) -> Result<TransactWriteItem> {
    fn status_attr(status: &UserStatus) -> &'static str {
        match status {
            UserStatus::Active => "active",
            UserStatus::Suspended => "suspended",
            UserStatus::Deleted => "deleted",
        }
    }

    let to_name = status_attr(to);
    let mut update = Update::builder()
        .table_name(table_name)
        .key("pk", AttributeValue::S(STATS_COUNTER_PK.to_string()))
        .key("sk", AttributeValue::S(STATS_COUNTER_SK.to_string()));

    match from {
        Some(from_status) => {
            let from_name = status_attr(from_status);
            assert_ne!(
                from_name, to_name,
                "a counter adjustment moves between two different statuses, got {from_name} -> {to_name}"
            );
            update = update
                .update_expression("ADD #from_status :minus_one, #to_status :plus_one")
                .expression_attribute_names("#from_status", from_name)
                .expression_attribute_names("#to_status", to_name)
                .expression_attribute_values(":minus_one", AttributeValue::N("-1".to_string()))
                .expression_attribute_values(":plus_one", AttributeValue::N("1".to_string()));
        }
        None => {
            update = update
                .update_expression("ADD #to_status :plus_one")
                .expression_attribute_names("#to_status", to_name)
                .expression_attribute_values(":plus_one", AttributeValue::N("1".to_string()));
        }
    }

    let built = update.build().map_err(|e| Error::StoreError {
        detail: e.to_string(),
    })?;
    Ok(TransactWriteItem::builder().update(built).build())
}

/// Maximum number of read-modify-write attempts `update_user` makes against its
/// version-conditional `PutItem` (`version = :read_version OR attribute_not_exists(version)`)
/// before giving up: the initial attempt plus retries triggered by a
/// `ConditionalCheckFailedException` (a concurrent writer already advanced the item's
/// `version`). Bounds retries so an item whose `version` keeps changing under relentless
/// concurrent writes cannot loop unbounded — it errors instead of looping forever or
/// silently overwriting the other writer's change.
const UPDATE_MAX_ATTEMPTS: u32 = 5;

/// Drives `update_user`'s version-conditional read-modify-write. Calls `attempt` (1-indexed)
/// up to `UPDATE_MAX_ATTEMPTS` times; `attempt` performs one full read-patch-`PutItem` cycle
/// and returns `Ok(Some(user))` on a successful, version-conditioned write, or `Ok(None)`
/// when the write lost to a `ConditionalCheckFailedException` because a concurrent writer
/// already advanced the item's `version` (retry against the fresh value). Returns
/// `Error::Conflict` — not an unbounded loop — once the budget is exhausted without a
/// successful write.
async fn retry_on_version_conflict<F, Fut>(user_id: &str, mut attempt: F) -> Result<User>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<Option<User>>>,
{
    for attempt_number in 1..=UPDATE_MAX_ATTEMPTS {
        if let Some(user) = attempt(attempt_number).await? {
            return Ok(user);
        }
    }

    Err(Error::Conflict {
        detail: format!(
            "update_user for {user_id} exhausted the retry budget \
             ({UPDATE_MAX_ATTEMPTS} attempts) racing concurrent version conflicts"
        ),
    })
}

/// Maximum number of `BatchWriteItem` submission attempts (the initial send plus retries)
/// allowed while draining a batch's `unprocessed_items` before the retry budget is exhausted
/// and the call errors instead of silently leaving items undeleted.
const BATCH_WRITE_MAX_ATTEMPTS: u32 = 8;

/// Maximum number of items in one `BatchWriteItem` request — the service's
/// hard limit. Every delete fan-out chunks its key list by this constant.
const BATCH_WRITE_MAX_ITEMS: usize = 25;

/// Maximum number of read-delete-confirm attempts a roster-driven revocation
/// (`revoke_family`) makes when its confirming re-read shows the roster
/// changed mid-flight (a concurrent transactional mutation landed). Bounded
/// so a relentlessly contested family errors instead of looping forever.
const REVOCATION_MAX_ATTEMPTS: u32 = 5;

/// Base delay, in milliseconds, for the capped exponential backoff between `BatchWriteItem`
/// retry attempts: attempt `n` (n >= 2) sleeps `BATCH_WRITE_BACKOFF_BASE_MS * 2^(n-2)` ms
/// before re-submitting whatever DynamoDB reported as `unprocessed_items`.
const BATCH_WRITE_BACKOFF_BASE_MS: u64 = 50;

/// Submit `requests` via `submit`, re-submitting only the items it reports back as still
/// unprocessed, with capped exponential backoff, until the batch drains (an empty vec comes
/// back) or `BATCH_WRITE_MAX_ATTEMPTS` is exhausted. Returns the number of items deleted (equal
/// to `requests.len()` on success — every submitted item was eventually processed) or
/// `Error::StoreError` if the retry budget is exhausted with items still unprocessed.
async fn drain_unprocessed<S, Fut>(requests: Vec<WriteRequest>, mut submit: S) -> Result<u64>
where
    S: FnMut(Vec<WriteRequest>) -> Fut,
    Fut: Future<Output = Result<Vec<WriteRequest>>>,
{
    if requests.is_empty() {
        return Ok(0);
    }

    let total = requests.len() as u64;
    let mut pending = requests;

    for attempt in 1..=BATCH_WRITE_MAX_ATTEMPTS {
        if attempt > 1 {
            let backoff_ms = BATCH_WRITE_BACKOFF_BASE_MS * (1u64 << (attempt - 2));
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
        }

        pending = submit(pending).await?;

        if pending.is_empty() {
            return Ok(total);
        }
    }

    Err(Error::StoreError {
        detail: format!(
            "BatchWriteItem retry budget ({BATCH_WRITE_MAX_ATTEMPTS} attempts) exhausted with \
             {} item(s) still unprocessed",
            pending.len()
        ),
    })
}

/// Whether two roster member lists name the same hash set, order-insensitively
/// (string sets in DynamoDB are unordered, so list order carries no meaning).
fn same_members(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().all(|hash| b.contains(hash))
}

/// Build the retirement record a winning rotation writes for `live`. The
/// successor inherits the family identity; the deadline is
/// `min(now + reuse_retention, family expires_at)` so no record outlives its
/// family. Legacy rows must never reach this helper — there is no prior
/// generation for them to be detected against.
fn retirement_record(
    live_hash: &str,
    live: &Session,
    replacement: &Session,
    reuse_retention_secs: u64,
    now: DateTime<Utc>,
) -> RetiredRefreshToken {
    assert!(
        live.refresh_token_hash.expose() == live_hash,
        "retirement record must name the presented hash"
    );
    assert!(
        is_valid_family_id(&live.family_id),
        "a legacy row must not produce a retirement record: {:?}",
        live.family_id
    );
    RetiredRefreshToken {
        refresh_token_hash: live_hash.to_string(),
        family_id: live.family_id.clone(),
        user_id: live.user_id.clone(),
        successor_hash: replacement.refresh_token_hash.expose().clone(),
        retired_at: now,
        expires_at: RetiredRefreshToken::retention_deadline(
            now,
            reuse_retention_secs,
            replacement.expires_at,
        ),
    }
}

pub struct DynamoRepository {
    client: aws_sdk_dynamodb::Client,
    table_name: String,
    /// How long [`DynamoRepository::count_active_sessions`] may serve a cached
    /// walk before re-scanning. Configured per deployment
    /// (`internal_api.stats_cache_ttl`); bounded below by
    /// [`MIN_STATS_CACHE_TTL`] so the cache can never be configured into a
    /// permanently stale answer.
    stats_cache_ttl: Duration,
    /// Cached `(fetched_at, active-session count)` pair behind
    /// `count_active_sessions`; `None` until the first walk.
    session_count_cache: Arc<TokioMutex<Option<(Instant, u64)>>>,
    /// How long a retirement record stays readable after its rotation:
    /// `retired_at + reuse_retention_secs`, capped per record at the family's
    /// absolute `expires_at` by [`RetiredRefreshToken::retention_deadline`].
    /// Resolved from `[token] refresh_reuse_retention` at bootstrap; injected
    /// here because the store, not the caller, stamps every record's deadline.
    reuse_retention_secs: u64,
}

/// Lower bound on the usable stats-cache TTL. A zero (or sub-millisecond) TTL
/// would make the cache useless while still reporting "cached" numbers;
/// validation refuses such a configuration rather than letting an operator
/// believe they asked for fresh counts.
pub const MIN_STATS_CACHE_TTL: Duration = Duration::from_millis(1);

impl DynamoRepository {
    pub fn new(
        client: aws_sdk_dynamodb::Client,
        table_name: String,
        reuse_retention_secs: u64,
    ) -> Self {
        assert!(
            reuse_retention_secs > 0,
            "reuse retention must be greater than zero"
        );
        Self {
            client,
            table_name,
            reuse_retention_secs,
            stats_cache_ttl: DEFAULT_STATS_CACHE_TTL,
            session_count_cache: Arc::new(TokioMutex::new(None)),
        }
    }

    /// Override the stats-cache TTL (from `internal_api.stats_cache_ttl`).
    ///
    /// The value is asserted to be within the usable range so a misconfigured
    /// TTL fails loudly at wiring time instead of silently serving stale or
    /// never-cached counts.
    pub fn with_stats_cache_ttl(mut self, ttl: Duration) -> Self {
        assert!(
            ttl >= MIN_STATS_CACHE_TTL,
            "stats_cache_ttl of {ttl:?} is below the usable minimum of {MIN_STATS_CACHE_TTL:?}"
        );
        assert!(
            ttl <= MAX_STATS_CACHE_TTL,
            "stats_cache_ttl of {ttl:?} exceeds the maximum of {MAX_STATS_CACHE_TTL:?}"
        );
        self.stats_cache_ttl = ttl;
        self
    }

    /// Map any AWS SDK error onto [`Error::StoreError`], extracting the
    /// service's own message where one exists — a bare `SdkError` Display is
    /// just "service error", which tells an operator nothing about which
    /// request failed or why.
    fn store_err(e: impl std::fmt::Debug + std::fmt::Display) -> Error {
        let detail = format!("{e:#?}");
        Error::StoreError { detail }
    }

    fn session_pk(token_hash: &str) -> AttributeValue {
        AttributeValue::S(format!("SESSION#{token_hash}"))
    }

    fn retired_pk(token_hash: &str) -> AttributeValue {
        AttributeValue::S(format!("RETIRED#{token_hash}"))
    }

    fn user_sk() -> AttributeValue {
        AttributeValue::S("USER".to_string())
    }

    fn session_sk() -> AttributeValue {
        AttributeValue::S("SESSION".to_string())
    }

    /// Strongly consistent fetch of one live-session item. Both answers this
    /// read feeds are security decisions — `/revoke`'s liveness check and
    /// rotation's identity capture — and an eventually consistent read would
    /// let a revoked token mint an access token for the width of the
    /// replication window (`g3-dynamo-session-read-eventual-consistency`).
    async fn get_session_item(&self, token_hash: &str) -> Result<Option<Session>> {
        let result = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("pk", Self::session_pk(token_hash))
            .key("sk", Self::session_sk())
            .consistent_read(true)
            .send()
            .await
            .map_err(Self::store_err)?;

        match result.item {
            Some(item) => Ok(Some(item_to_session(&item)?)),
            None => Ok(None),
        }
    }

    /// Strongly consistent fetch of one retirement record. An eventually
    /// consistent answer would report reuse as an unknown token — refused,
    /// but with no alarm raised (SR1's retired half).
    async fn get_retired_record(&self, token_hash: &str) -> Result<Option<RetiredRefreshToken>> {
        let result = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("pk", Self::retired_pk(token_hash))
            .key("sk", AttributeValue::S(RETIRED_SK.to_string()))
            .consistent_read(true)
            .send()
            .await
            .map_err(Self::store_err)?;

        match result.item {
            Some(item) => Ok(Some(item_to_retired(&item)?)),
            None => Ok(None),
        }
    }

    /// Strongly consistent read of the user item's authoritative roster. This
    /// is what revocation enumerates instead of GSI1
    /// (`g3-dynamo-revoke-all-gsi-incompleteness`): an index can omit a
    /// session written moments earlier and strand a live credential with
    /// nothing left to find it; the roster cannot disagree with the items
    /// because every session write maintains it transactionally.
    async fn get_user_roster(&self, user_id: &str) -> Result<UserRoster> {
        let result = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(format!("USER#{user_id}")))
            .key("sk", Self::user_sk())
            .consistent_read(true)
            .send()
            .await
            .map_err(Self::store_err)?;

        match result.item {
            Some(item) => UserRoster::from_item(&item),
            None => Ok(UserRoster::default()),
        }
    }

    /// Build the roster-maintenance `Update` statement targeting
    /// `USER#<user_id> / USER`. Every session-writing transaction routes its
    /// roster arm through here so the item key, the mutual-consistency
    /// condition, and the expression wiring cannot drift between call sites.
    fn roster_update(
        &self,
        user_id: &str,
        condition: &str,
        expression: &str,
        names: &[(&str, &str)],
        values: HashMap<String, AttributeValue>,
    ) -> Result<TransactWriteItem> {
        let mut update = Update::builder()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(format!("USER#{user_id}")))
            .key("sk", Self::user_sk())
            .update_expression(expression)
            .condition_expression(condition);
        for (placeholder, attribute) in names {
            update = update.expression_attribute_names(*placeholder, *attribute);
        }
        let update = update
            .set_expression_attribute_values(Some(values))
            .build()
            .map_err(Self::store_err)?;

        Ok(TransactWriteItem::builder().update(update).build())
    }

    /// Whether a cancelled `TransactWriteItems` call failed specifically on
    /// statement `statement_index`'s condition — and only such a failure is a
    /// compare-and-swap loss. A cancellation whose reasons are absent or name
    /// any other statement (a colliding replacement, a vanished user item) is
    /// a caller bug or corruption, not a lost race.
    fn lost_race_reason(
        err: &aws_sdk_dynamodb::error::SdkError<TransactWriteItemsError>,
        statement_index: usize,
    ) -> bool {
        match err.as_service_error() {
            Some(TransactWriteItemsError::TransactionCanceledException(tce)) => tce
                .cancellation_reasons()
                .get(statement_index)
                .is_some_and(|reason| reason.code() == Some(CONDITIONAL_CHECK_FAILED_CODE)),
            _ => false,
        }
    }

    /// One-off migration step: scans every `PROFILE` item in the table and writes the
    /// uniqueness-guard item (`EXT#<provider>#<external_id>` / `UNIQUE`) for any user that
    /// does not already have one, so the `(provider, external_id)` invariant that
    /// transactional `create_user` enforces going forward also holds for rows written
    /// before the guard existed.
    ///
    /// **Ordering constraint:** this must run to completion — with no user left
    /// unguarded — before a deployment starts using [`DynamoRepository::get_user_by_external_id`],
    /// which resolves purely through the guard item (GSI1 no longer carries a User entry); a
    /// guard-less pre-existing user would otherwise become invisible to that lookup.
    ///
    /// Idempotent and safe to re-run after a partial failure: each guard write is
    /// conditioned on `attribute_not_exists(pk)`, so a user that already has a guard
    /// (backfilled by an earlier run, or created after the guard existed) is left
    /// untouched and not counted. Returns the number of guard items actually written.
    pub async fn backfill_uniqueness_guards(&self) -> Result<u64> {
        let mut written: u64 = 0;
        let mut exclusive_start_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let mut scan = self
                .client
                .scan()
                .table_name(&self.table_name)
                .filter_expression("sk = :sk")
                .expression_attribute_values(":sk", AttributeValue::S("PROFILE".to_string()));

            if let Some(ref start_key) = exclusive_start_key {
                scan = scan.set_exclusive_start_key(Some(start_key.clone()));
            }

            let result = scan.send().await.map_err(Self::store_err)?;
            let items = result.items.unwrap_or_default();

            for item in &items {
                let user = item_to_user(item)?;
                let guard_item = guard_to_item(&user.provider, &user.external_id, &user.id);

                let outcome = self
                    .client
                    .put_item()
                    .table_name(&self.table_name)
                    .set_item(Some(guard_item))
                    .condition_expression("attribute_not_exists(pk)")
                    .send()
                    .await;

                match outcome {
                    Ok(_) => written += 1,
                    Err(err) => {
                        let already_guarded = matches!(
                            err.as_service_error(),
                            Some(
                                aws_sdk_dynamodb::operation::put_item::PutItemError::ConditionalCheckFailedException(_)
                            )
                        );
                        if !already_guarded {
                            return Err(Self::store_err(err));
                        }
                    }
                }
            }

            match result.last_evaluated_key {
                Some(key) => exclusive_start_key = Some(key),
                None => break,
            }
        }

        Ok(written)
    }

    /// One-off migration step: creates the authoritative session-roster item
    /// (`pk = USER#<id>`, `sk = USER`, empty `families`) for every user whose
    /// profile predates rosters, so the nested roster writes in
    /// `store_refresh_token` and the rotation path are valid for them. New
    /// users get the item from `create_user`.
    ///
    /// Idempotent and safe to re-run after a partial failure: each write is
    /// conditioned on `attribute_not_exists(pk)`, so a user that already has a
    /// roster item (backfilled by an earlier run, or created after rosters
    /// existed) is left untouched and not counted. Returns the number of
    /// roster items actually written.
    pub async fn backfill_session_rosters(&self) -> Result<u64> {
        let mut written: u64 = 0;
        let mut exclusive_start_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let mut scan = self
                .client
                .scan()
                .table_name(&self.table_name)
                .filter_expression("sk = :profile")
                .expression_attribute_values(":profile", AttributeValue::S("PROFILE".to_string()));

            if let Some(ref start_key) = exclusive_start_key {
                scan = scan.set_exclusive_start_key(Some(start_key.clone()));
            }

            let result = scan.send().await.map_err(Self::store_err)?;

            for item in result.items.unwrap_or_default() {
                let Some(pk) = item.get("pk") else {
                    return Err(Error::StoreError {
                        detail: "profile item is missing its pk while backfilling rosters"
                            .to_string(),
                    });
                };
                assert!(
                    pk.as_s().is_ok_and(|value| value.starts_with("USER#")),
                    "backfill_session_rosters: profile scan returned a non-user pk"
                );

                let outcome = self
                    .client
                    .put_item()
                    .table_name(&self.table_name)
                    .set_item(Some(HashMap::from([
                        ("pk".to_string(), pk.clone()),
                        ("sk".to_string(), DynamoRepository::user_sk()),
                        ("families".to_string(), AttributeValue::M(HashMap::new())),
                    ])))
                    .condition_expression("attribute_not_exists(pk)")
                    .send()
                    .await;

                match outcome {
                    Ok(_) => written += 1,
                    Err(err) => {
                        // A conditional-check failure here means a concurrent
                        // writer (another backfill run, or create_user)
                        // already created this roster item — success from
                        // this step's point of view.
                        let already_exists = matches!(
                            err.as_service_error(),
                            Some(
                                aws_sdk_dynamodb::operation::put_item::PutItemError::ConditionalCheckFailedException(_)
                            )
                        );
                        if !already_exists {
                            return Err(Self::store_err(err));
                        }
                    }
                }
            }

            match result.last_evaluated_key {
                Some(key) => exclusive_start_key = Some(key),
                None => break,
            }
        }

        Ok(written)
    }

    /// Submit up to 25 `WriteRequest`s via `BatchWriteItem` on this repository's table,
    /// draining any `unprocessed_items` DynamoDB reports back (see [`drain_unprocessed`]).
    /// Returns the number of items actually deleted.
    async fn batch_write_with_retry(&self, requests: Vec<WriteRequest>) -> Result<u64> {
        let table_name = self.table_name.clone();
        drain_unprocessed(requests, |batch| {
            let table_name = table_name.clone();
            async move {
                let result = self
                    .client
                    .batch_write_item()
                    .request_items(&table_name, batch)
                    .send()
                    .await
                    .map_err(Self::store_err)?;

                Ok(result
                    .unprocessed_items
                    .and_then(|mut m| m.remove(&table_name))
                    .unwrap_or_default())
            }
        })
        .await
    }

    /// Discover the candidate owner ids of `family_id` by scanning the *base
    /// table* under strong consistency for any session or retirement item
    /// carrying the family. This is deliberately not a GSI1 query: the query
    /// would need the owner id it is trying to discover, and an index read
    /// could not be made consistent even if it could. A base-table
    /// `Scan`/`Filter` pair answers from the same committed state the
    /// strongly consistent `GetItem`s read, so a family written moments ago
    /// is discoverable now — the GSI-staleness window that strands
    /// credentials cannot hide an owner from this path.
    async fn user_ids_for_family(&self, family_id: &str) -> Result<Vec<String>> {
        assert!(
            !family_id.is_empty(),
            "user_ids_for_family: family_id must not be empty"
        );

        let mut user_ids: Vec<String> = Vec::new();
        let mut exclusive_start_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let mut scan = self
                .client
                .scan()
                .table_name(&self.table_name)
                .consistent_read(true)
                .filter_expression("family_id = :fid")
                .expression_attribute_values(":fid", AttributeValue::S(family_id.to_string()))
                .projection_expression("user_id");

            if let Some(ref start_key) = exclusive_start_key {
                scan = scan.set_exclusive_start_key(Some(start_key.clone()));
            }

            let result = scan.send().await.map_err(Self::store_err)?;

            for item in result.items.unwrap_or_default() {
                let user_id = item
                    .get("user_id")
                    .and_then(|v| v.as_s().ok())
                    .ok_or_else(|| Error::StoreError {
                        detail: format!(
                            "item carrying family {family_id} is missing its user_id attribute"
                        ),
                    })?
                    .clone();
                assert!(
                    !user_id.is_empty(),
                    "user_ids_for_family: discovered an empty owner id for family {family_id}"
                );
                if !user_ids.contains(&user_id) {
                    user_ids.push(user_id);
                }
            }

            match result.last_evaluated_key {
                Some(key) => exclusive_start_key = Some(key),
                None => break,
            }
        }

        Ok(user_ids)
    }

    /// One user's share of [`SessionRepository::revoke_family`]: the
    /// converging read-delete-confirm-apply protocol described there, plus
    /// the exact-removal count for the entries this user's roster named.
    async fn revoke_family_for_user(&self, family_id: &str, user_id: &str) -> Result<u64> {
        // Entries whose deletion has already been counted across attempts, so
        // a retry that re-names them cannot double-report the removal.
        let mut counted: std::collections::HashSet<String> = std::collections::HashSet::new();

        for _attempt in 1..=REVOCATION_MAX_ATTEMPTS {
            let roster = self.get_user_roster(user_id).await?;
            let Some(family) = roster.families.get(family_id) else {
                // Nothing left under this family for this user: earlier
                // attempts (or another writer) already cleared it.
                return Ok(counted.len() as u64);
            };
            assert!(
                family.live.is_empty() || roster.sessions.contains(&family.live),
                "roster corruption: family {family_id}'s live pointer must name a live session"
            );

            // The deletion set is exactly what the roster names: the live
            // generation's session item, and a retirement record for every
            // remembered member that is not the live one. Each entry gets a
            // stable name ("SESSION#<hash>" / "RETIRED#<hash>") used both for
            // the delete request and the de-duplicated count.
            let mut entries: Vec<(String, AttributeValue, AttributeValue)> = Vec::new();
            if !family.live.is_empty() {
                entries.push((
                    format!("SESSION#{}", family.live),
                    Self::session_pk(&family.live),
                    Self::session_sk(),
                ));
            }
            for hash in &family.members {
                if *hash != family.live {
                    entries.push((
                        format!("RETIRED#{hash}"),
                        Self::retired_pk(hash),
                        AttributeValue::S(RETIRED_SK.to_string()),
                    ));
                }
            }
            for (name, _, _) in &entries {
                counted.insert(name.clone());
            }

            for chunk in entries.chunks(BATCH_WRITE_MAX_ITEMS) {
                let requests = chunk
                    .iter()
                    .map(|(_, pk, sk)| {
                        WriteRequest::builder()
                            .delete_request(
                                DeleteRequest::builder()
                                    .key("pk", pk.clone())
                                    .key("sk", sk.clone())
                                    .build()
                                    .expect("valid delete request"),
                            )
                            .build()
                    })
                    .collect();
                self.batch_write_with_retry(requests).await?;
            }

            // Confirm the roster still names exactly what was deleted before
            // clearing the family's entry. A mismatch means a transactional
            // mutation committed between the read and the deletes; retrying
            // absorbs its effects into the next attempt.
            let fresh = self.get_user_roster(user_id).await?;
            let unchanged = match fresh.families.get(family_id) {
                Some(f) => f.live == family.live && same_members(&f.members, &family.members),
                None => false,
            };
            if !unchanged {
                continue;
            }

            // Remove the family's roster entry and its live hash from the
            // session set — only the confirmed-stale entries, never anything
            // a concurrent writer may have added.
            self.client
                .update_item()
                .table_name(&self.table_name)
                .key("pk", AttributeValue::S(format!("USER#{user_id}")))
                .key("sk", Self::user_sk())
                .update_expression("DELETE #s :live REMOVE families.#f")
                .expression_attribute_names("#s", "sessions")
                .expression_attribute_names("#f", family_id)
                .expression_attribute_values(":live", AttributeValue::Ss(vec![family.live.clone()]))
                .condition_expression("attribute_exists(pk)")
                .send()
                .await
                .map_err(Self::store_err)?;

            return Ok(counted.len() as u64);
        }

        Err(Error::StoreError {
            detail: format!(
                "revoke_family for {family_id} exhausted its retry budget \
                 ({REVOCATION_MAX_ATTEMPTS} attempts) racing concurrent session mutations"
            ),
        })
    }
}

#[async_trait]
impl UserRepository for DynamoRepository {
    #[instrument(skip(self), fields(user_id))]
    async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>> {
        let result = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(format!("USER#{user_id}")))
            .key("sk", AttributeValue::S("PROFILE".to_string()))
            .consistent_read(true)
            .send()
            .await
            .map_err(Self::store_err)?;

        match result.item {
            Some(item) => Ok(Some(item_to_user(&item)?)),
            None => Ok(None),
        }
    }

    /// Resolves a user by `(provider, external_id)` in two strongly-consistent `GetItem`s
    /// through the uniqueness-guard item: first `EXT#<provider>#<external_id>` / `UNIQUE` to
    /// learn the owning `user_id`, then `USER#<user_id>` / `PROFILE` for the profile. GSI1 no
    /// longer carries a User entry — it serves only session lookups.
    ///
    /// **Precondition:** every existing user must have a guard item before this ships, i.e.
    /// [`DynamoRepository::backfill_uniqueness_guards`] must have run to completion; a
    /// guard-less pre-existing user would otherwise be invisible to this lookup even though
    /// its profile item still exists.
    #[instrument(skip(self), fields(external_id, provider))]
    async fn get_user_by_external_id(
        &self,
        external_id: &str,
        provider: &str,
    ) -> Result<Option<User>> {
        let guard = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(guard_pk(provider, external_id)))
            .key("sk", AttributeValue::S(GUARD_SK.to_string()))
            .consistent_read(true)
            .send()
            .await
            .map_err(Self::store_err)?;

        let Some(guard_item) = guard.item else {
            return Ok(None);
        };

        let user_id = guard_item
            .get("user_id")
            .and_then(|v| v.as_s().ok())
            .ok_or_else(|| Error::StoreError {
                detail: format!(
                    "guard item EXT#{provider}#{external_id}/UNIQUE is missing its user_id \
                     attribute"
                ),
            })?;

        let profile = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(format!("USER#{user_id}")))
            .key("sk", AttributeValue::S("PROFILE".to_string()))
            .consistent_read(true)
            .send()
            .await
            .map_err(Self::store_err)?;

        match profile.item {
            Some(item) => Ok(Some(item_to_user(&item)?)),
            None => Ok(None),
        }
    }

    #[instrument(skip(self, user), fields(external_id = %user.external_id, provider = %user.provider))]
    async fn create_user(&self, user: &NewUser) -> Result<User> {
        let now = Utc::now();
        let id = format!("usr_{}", ulid::Ulid::new().to_string().to_lowercase());

        let full_user = User {
            id,
            external_id: user.external_id.clone(),
            provider: user.provider.clone(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            metadata: HashMap::new(),
            claims: HashMap::new(),
            status: UserStatus::Active,
            version: INITIAL_USER_VERSION,
            created_at: now,
            updated_at: now,
        };

        let user_item = user_to_item(&full_user);
        let guard_item = guard_to_item(&full_user.provider, &full_user.external_id, &full_user.id);
        // The authoritative session roster lives on a dedicated item
        // (`pk = USER#<id>`, `sk = USER`), distinct from the `PROFILE` item.
        // Creating it here — in the same transaction as the profile and the
        // uniqueness guard, with an empty `families` map — is what lets every
        // later roster write succeed: `store_refresh_token` files a family
        // entry at the nested path `families.#fid`, and DynamoDB refuses a
        // nested SET whose root attribute is absent. Users predating rosters
        // are covered by `backfill_session_rosters`.
        let roster_seed = Put::builder()
            .table_name(&self.table_name)
            .set_item(Some(HashMap::from([
                (
                    "pk".to_string(),
                    AttributeValue::S(format!("USER#{}", full_user.id)),
                ),
                ("sk".to_string(), Self::user_sk()),
                ("families".to_string(), AttributeValue::M(HashMap::new())),
            ])))
            .condition_expression("attribute_not_exists(pk)")
            .build()
            .map_err(Self::store_err)?;

        let user_put = Put::builder()
            .table_name(&self.table_name)
            .set_item(Some(user_item))
            .condition_expression("attribute_not_exists(pk)")
            .build()
            .map_err(Self::store_err)?;

        let guard_put = Put::builder()
            .table_name(&self.table_name)
            .set_item(Some(guard_item))
            .condition_expression("attribute_not_exists(pk)")
            .build()
            .map_err(Self::store_err)?;

        // Every user create also moves the STATS#USERS/COUNTS counter: the
        // adjustment is transactional with the profile+guard writes, so a
        // lost uniqueness race cancels the counter increment along with the
        // rest of the aborted transaction.
        let counter_adjust = counter_adjustment(&self.table_name, None, &UserStatus::Active)?;

        self.client
            .transact_write_items()
            .transact_items(TransactWriteItem::builder().put(user_put).build())
            .transact_items(TransactWriteItem::builder().put(guard_put).build())
            .transact_items(TransactWriteItem::builder().put(roster_seed).build())
            .transact_items(counter_adjust)
            .send()
            .await
            .map_err(|err| {
                let is_uniqueness_conflict = match err.as_service_error() {
                    Some(TransactWriteItemsError::TransactionCanceledException(tce)) => tce
                        .cancellation_reasons()
                        .iter()
                        .any(|reason| reason.code() == Some(CONDITIONAL_CHECK_FAILED_CODE)),
                    _ => false,
                };

                if is_uniqueness_conflict {
                    Error::Conflict {
                        detail: format!(
                            "user already exists for external_id={} provider={}",
                            user.external_id, user.provider
                        ),
                    }
                } else {
                    Self::store_err(err)
                }
            })?;

        Ok(full_user)
    }

    #[instrument(skip(self, patch), fields(user_id))]
    async fn update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User> {
        // Get-modify-put pattern, made concurrency-safe with a version-conditional write:
        // each attempt re-reads the current item, applies the patch on top of it, and puts
        // the result back conditioned on `version` still matching what was just read. A
        // concurrent writer that already advanced `version` cancels the condition instead
        // of letting this write silently clobber the other writer's change; the loop
        // re-reads and retries against the new version, up to `UPDATE_MAX_ATTEMPTS`.
        retry_on_version_conflict(user_id, |attempt_number| async move {
            let mut user = self
                .get_user_by_id(user_id)
                .await?
                .ok_or_else(|| Error::StoreError {
                    detail: format!("user not found: {user_id}"),
                })?;
            let read_version = user.version;
            // Captured from the fresh read, *before* the patch is applied, so a repeated
            // delete of an already-`Deleted` user (e.g. a retried DELETE request) is
            // recognized as a no-op transition rather than re-triggering the guard delete
            // below — see `is_delete_transition`.
            let was_deleted = user.status == UserStatus::Deleted;
            // The pre-patch status, for the counter adjustment below.
            let status_before = user.status.clone();

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
            user.version = read_version + 1;

            // The counter delta this patch implies: only an actual status
            // *change* moves a row between buckets — a same-status patch (or
            // a pure email/claims patch) adjusts nothing. Computed from the
            // fresh read each attempt, so a retry after a version conflict
            // recomputes the delta from the row it actually won.
            let status_transition = match &patch.status {
                Some(target) if *target != status_before => {
                    Some((status_before.clone(), target.clone()))
                }
                _ => None,
            };

            let item = user_to_item(&user);
            // A transition to `Deleted` frees `(provider, external_id)` for
            // re-registration (see 01-domain-model.md §Lifecycles): the guard item
            // that enforces uniqueness must be removed in the same atomic write as
            // the status change, so a reader can never observe a `Deleted` user
            // whose guard is still standing (or vice versa). Every other patch keeps
            // the plain, cheaper version-conditional `PutItem`.
            //
            // Gated on `!was_deleted` (the status read *before* this patch was applied):
            // without that guard, a repeated delete of an already-`Deleted` user (a
            // retried DELETE request, or a PATCH status=deleted arriving twice) would
            // recompute `is_delete_transition` from `patch.status` alone and re-run the
            // guard delete unconditionally, keyed only by `(provider, external_id)`. If
            // the identity had since been re-registered (delete -> recreate), that second
            // delete would remove the *new* user's guard out from under it, freeing the
            // identity for a third registration while the second user is still `Active` —
            // two live users sharing one `(provider, external_id)`, violating the
            // uniqueness invariant this guard exists to enforce.
            let is_delete_transition =
                matches!(patch.status, Some(UserStatus::Deleted)) && !was_deleted;
            // A status change that keeps the guard in place still moves the row
            // between counter buckets, so its version-conditional write is
            // promoted from a plain `PutItem` into a one-item transaction that
            // carries the counter adjustment atomically (08-persistence.md).
            let needs_transaction = is_delete_transition || status_transition.is_some();

            if needs_transaction {
                let user_put = Put::builder()
                    .table_name(&self.table_name)
                    .set_item(Some(item))
                    .condition_expression(
                        "version = :read_version OR attribute_not_exists(version)",
                    )
                    .expression_attribute_values(
                        ":read_version",
                        AttributeValue::N(read_version.to_string()),
                    )
                    .build()
                    .map_err(Self::store_err)?;

                let mut transaction = self
                    .client
                    .transact_write_items()
                    .transact_items(TransactWriteItem::builder().put(user_put).build());

                if let Some((from, to)) = &status_transition {
                    transaction = transaction.transact_items(counter_adjustment(
                        &self.table_name,
                        Some(from),
                        to,
                    )?);
                }

                if is_delete_transition {
                    // Defense in depth alongside the `!was_deleted` gate above: even if this
                    // delete somehow still fired for a stale/retried request, condition it on
                    // the guard item's `user_id` still being *this* user's id, so it can never
                    // remove a guard that has since come to belong to a re-registered user for
                    // the same `(provider, external_id)`.
                    let guard_delete = Delete::builder()
                        .table_name(&self.table_name)
                        .key(
                            "pk",
                            AttributeValue::S(guard_pk(&user.provider, &user.external_id)),
                        )
                        .key("sk", AttributeValue::S(GUARD_SK.to_string()))
                        .condition_expression("user_id = :user_id")
                        .expression_attribute_values(
                            ":user_id",
                            AttributeValue::S(user.id.clone()),
                        )
                        .build()
                        .map_err(Self::store_err)?;
                    transaction =
                        transaction.transact_items(TransactWriteItem::builder().delete(guard_delete).build());
                }

                let outcome = transaction.send().await;

                match outcome {
                    Ok(_) => Ok(Some(user)),
                    Err(err) => {
                        let is_version_conflict = match err.as_service_error() {
                            Some(TransactWriteItemsError::TransactionCanceledException(tce)) => tce
                                .cancellation_reasons()
                                .iter()
                                .any(|reason| reason.code() == Some(CONDITIONAL_CHECK_FAILED_CODE)),
                            _ => false,
                        };
                        if !is_version_conflict {
                            return Err(Self::store_err(err));
                        }
                        tracing::debug!(
                            attempt = attempt_number,
                            max_attempts = UPDATE_MAX_ATTEMPTS,
                            "update_user (transactional status write) version conflict, retrying"
                        );
                        Ok(None)
                    }
                }
            } else {
                let outcome = self
                    .client
                    .put_item()
                    .table_name(&self.table_name)
                    .set_item(Some(item))
                    .condition_expression(
                        "version = :read_version OR attribute_not_exists(version)",
                    )
                    .expression_attribute_values(
                        ":read_version",
                        AttributeValue::N(read_version.to_string()),
                    )
                    .send()
                    .await;

                match outcome {
                    Ok(_) => Ok(Some(user)),
                    Err(err) => {
                        let is_version_conflict = matches!(
                            err.as_service_error(),
                            Some(
                                aws_sdk_dynamodb::operation::put_item::PutItemError::ConditionalCheckFailedException(_)
                            )
                        );
                        if !is_version_conflict {
                            return Err(Self::store_err(err));
                        }
                        tracing::debug!(
                            attempt = attempt_number,
                            max_attempts = UPDATE_MAX_ATTEMPTS,
                            "update_user version conflict, retrying"
                        );
                        Ok(None)
                    }
                }
            }
        })
        .await
    }

    #[instrument(skip(self), fields(user_id))]
    async fn delete_user(&self, user_id: &str) -> Result<()> {
        self.update_user(
            user_id,
            &UserPatch {
                email: None,
                display_name: None,
                metadata: None,
                claims: None,
                status: Some(UserStatus::Deleted),
            },
        )
        .await?;
        Ok(())
    }

    /// Read the user-status counters from the transactionally-maintained
    /// `STATS#USERS`/`COUNTS` item — a single strongly-consistent `GetItem`,
    /// never a table walk.
    ///
    /// A missing item (a table that has seen no user write since the counter
    /// shipped) reads as all-zero rather than triggering a fallback scan: the
    /// fallback would reintroduce exactly the unbounded admin read this
    /// adapter removed, and the counters are maintained transactionally with
    /// every write that changes a status going forward. Pre-existing rows
    /// written before the counter item existed are reflected only after their
    /// next write; deployments migrating an existing table should backfill
    /// `STATS#USERS`/`COUNTS` once out of band.
    #[instrument(skip(self))]
    async fn count_by_status(&self) -> Result<HashMap<String, u64>> {
        let result = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(STATS_COUNTER_PK.to_string()))
            .key("sk", AttributeValue::S(STATS_COUNTER_SK.to_string()))
            .consistent_read(true)
            // Report the single-item read's capacity so callers and tests can
            // verify the stats path costs one GetItem, not a table scan.
            .return_consumed_capacity(ReturnConsumedCapacity::Total)
            .send()
            .await
            .map_err(Self::store_err)?;

        let mut counts: HashMap<String, u64> = HashMap::new();
        if let Some(item) = result.item {
            for status in ["active", "suspended", "deleted"] {
                let value = match item.get(status).and_then(|v| v.as_n().ok()) {
                    Some(n) => n.parse::<u64>().map_err(|e| Error::StoreError {
                        detail: format!("counter attribute {status} is not a count: {e}"),
                    })?,
                    None => 0,
                };
                counts.insert(status.to_string(), value);
            }
            assert_eq!(
                counts.len(),
                3,
                "the counter item must report exactly the three UserStatus buckets"
            );
        }

        Ok(counts)
    }

    /// One bounded `Scan` per page — never a full-table materialization.
    ///
    /// The page size rides to DynamoDB as the scan's `Limit` and the caller's
    /// cursor (the previous response's encoded `LastEvaluatedKey`) as its
    /// `ExclusiveStartKey`. Because DynamoDB applies `Limit` *before* the
    /// `sk = PROFILE` filter, a page may be shorter than the requested limit
    /// while more matching rows remain — the source-specified short-page /
    /// non-null-cursor behaviour — so the next cursor is derived purely from
    /// whether DynamoDB returned a `LastEvaluatedKey`, never from how many
    /// rows happened to survive the filter. Ordering is scan order (store
    /// key distribution), not `created_at`; cursor paging is stable and
    /// complete regardless.
    #[instrument(skip(self))]
    async fn list_users(&self, cursor: Option<&str>, limit: u32) -> Result<UserPage> {
        // Defense in depth alongside the core's clamp: this adapter is public
        // to any embedder that may not have passed the value through
        // `admin_list_users`, so the bound is re-asserted at the boundary.
        assert!(
            (1..=MAX_ADMIN_PAGE_SIZE).contains(&limit),
            "list_users limit must arrive pre-clamped within 1..={MAX_ADMIN_PAGE_SIZE}, got {limit}"
        );
        let exclusive_start_key = match cursor {
            Some(raw) => Some(ScanCursor::decode(raw)?.to_key_map()),
            None => None,
        };

        let mut scan = self
            .client
            .scan()
            .table_name(&self.table_name)
            .filter_expression("sk = :sk")
            .expression_attribute_values(":sk", AttributeValue::S("PROFILE".to_string()))
            // Report the page's consumed read capacity so callers and tests
            // can verify the read stayed bounded (the spec's capacity-based
            // verification, not wall-clock timing).
            .return_consumed_capacity(ReturnConsumedCapacity::Total)
            .limit(limit as i32);
        if let Some(start_key) = exclusive_start_key {
            scan = scan.set_exclusive_start_key(Some(start_key));
        }

        let result = scan.send().await.map_err(Self::store_err)?;
        let items = result.items.unwrap_or_default();

        let mut users = Vec::with_capacity(items.len());
        for item in &items {
            users.push(item_to_user(item)?);
        }
        assert!(
            users.len() <= limit as usize,
            "a single bounded scan returned {} rows against a Limit of {limit}",
            users.len()
        );

        // A present LastEvaluatedKey means "more pages may follow", even when
        // the filtered page came back empty; its absence is the only
        // exhaustion signal.
        let next_cursor = match result.last_evaluated_key {
            Some(key) => {
                assert!(
                    !key.is_empty(),
                    "DynamoDB must never report an empty LastEvaluatedKey"
                );
                Some(ScanCursor::from_key_map(&key).encode())
            }
            None => None,
        };

        Ok(UserPage { users, next_cursor })
    }
}

impl DynamoRepository {
    /// The uncached walk behind [`Self::count_active_sessions`]: one paginated
    /// scan of the session items, counting those not yet expired.
    async fn scan_active_session_count(&self) -> Result<u64> {
        let now = Utc::now();
        let mut count: u64 = 0;
        let mut exclusive_start_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let mut scan = self
                .client
                .scan()
                .table_name(&self.table_name)
                .filter_expression("sk = :sk")
                .expression_attribute_values(":sk", AttributeValue::S("SESSION".to_string()))
                // Capacity reporting for the same reason as the admin reads:
                // the cache-miss walk is bounded by nothing but the table, so
                // its cost must stay observable.
                .return_consumed_capacity(ReturnConsumedCapacity::Total);

            if let Some(ref start_key) = exclusive_start_key {
                scan = scan.set_exclusive_start_key(Some(start_key.clone()));
            }

            let result = scan.send().await.map_err(Self::store_err)?;
            let items = result.items.unwrap_or_default();

            for item in &items {
                if let Ok(session) = item_to_session(item) {
                    if session.expires_at > now {
                        count += 1;
                    }
                }
            }

            match result.last_evaluated_key {
                Some(key) => exclusive_start_key = Some(key),
                None => break,
            }
        }

        Ok(count)
    }
}

#[async_trait]
impl SessionRepository for DynamoRepository {
    /// Write one generation-0 row as a single transaction with its roster
    /// maintenance: the session item (conditional on being fresh) plus the
    /// user item gaining the hash in `sessions` and a fresh `families` entry.
    /// The conditional user-item update is what keeps the authoritative
    /// roster trustworthy — storing a credential for a user whose profile
    /// item does not exist is a caller bug, mirroring the SQL adapters'
    /// foreign-key discipline.
    #[instrument(skip(self, session), fields(user_id = %session.user_id))]
    async fn store_refresh_token(&self, session: &Session) -> Result<()> {
        assert!(
            is_valid_family_id(&session.family_id),
            "store_refresh_token: malformed family id {:?}",
            session.family_id
        );
        assert!(
            !session.refresh_token_hash.expose().is_empty(),
            "store_refresh_token: refresh_token_hash must not be empty"
        );

        let session_put = Put::builder()
            .table_name(&self.table_name)
            .set_item(Some(session_to_item(session)))
            .condition_expression("attribute_not_exists(pk)")
            .build()
            .map_err(Self::store_err)?;

        // Roster arm: `sessions` gains the fresh generation and its family
        // entry is created wholesale. Set-typed attribute operands take
        // string-set values even for a single member.
        let mut values = HashMap::new();
        values.insert(
            ":hash".to_string(),
            AttributeValue::Ss(vec![session.refresh_token_hash.expose().clone()]),
        );
        values.insert(
            ":family".to_string(),
            FamilyRoster::new(
                session.refresh_token_hash.expose().clone(),
                vec![session.refresh_token_hash.expose().clone()],
            )
            .to_attribute(),
        );
        let roster_arm = self.roster_update(
            &session.user_id,
            "attribute_exists(pk)",
            "ADD sessions :hash SET families.#f = :family",
            // The placeholder binds the *family id* — the key inside the
            // `families` map under which this session's entry is filed.
            &[("#f", session.family_id.as_str())],
            values,
        )?;

        self.client
            .transact_write_items()
            .transact_items(TransactWriteItem::builder().put(session_put).build())
            .transact_items(roster_arm)
            .send()
            .await
            .map_err(Self::store_err)?;
        Ok(())
    }

    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn get_session_by_refresh_token(
        &self,
        token_hash: &Secret<String>,
    ) -> Result<Option<Session>> {
        let token_hash = token_hash.expose().as_str();
        assert!(
            !token_hash.is_empty(),
            "get_session_by_refresh_token: token_hash must not be empty"
        );
        self.get_session_item(token_hash).await
    }

    /// Classify against live generations and retained retirement records,
    /// every read strongly consistent (SR1): an eventually consistent
    /// `SESSION#` answer could keep a revoked token alive for the width of
    /// replication, and an eventually consistent `RETIRED#` answer would turn
    /// reuse into a silent unknown. A record past its retention deadline
    /// answers `Unknown` until TTL or sweep physically removes it — reuse
    /// detection must not fire on a window that has closed.
    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn resolve_refresh_token(&self, token_hash: &str) -> Result<RefreshResolution> {
        assert!(
            !token_hash.is_empty(),
            "resolve_refresh_token: token_hash must not be empty"
        );

        if let Some(session) = self.get_session_item(token_hash).await? {
            return Ok(RefreshResolution::Live(session));
        }

        let Some(record) = self.get_retired_record(token_hash).await? else {
            return Ok(RefreshResolution::Unknown);
        };
        if record.expires_at <= Utc::now() {
            return Ok(RefreshResolution::Unknown);
        }

        match self.get_session_item(&record.successor_hash).await? {
            Some(successor_live) => Ok(RefreshResolution::Superseded {
                live: successor_live,
                retired_at: record.retired_at,
            }),
            None => Ok(RefreshResolution::Retired {
                family_id: record.family_id,
                user_id: record.user_id,
                retired_at: record.retired_at,
            }),
        }
    }

    /// One `TransactWriteItems` performing the whole swap — delete the live
    /// item (conditioned on still existing: THE compare-and-swap), write the
    /// retirement item, install the replacement, and move both roster entries
    /// — so item storage and the authoritative roster can never disagree
    /// about a rotation. Only the live-generation condition cancelling maps
    /// to `false`; every other failure surfaces as a store error.
    ///
    /// The live row is read first (strongly consistent) to capture its family
    /// identity; a concurrent rotation that moves it between that read and
    /// the transaction still loses cleanly through the delete's condition. A
    /// live row carrying the empty-family sentinel is a pre-rotation legacy
    /// row: its first redemption swaps without writing any retirement record
    /// — there is no prior generation to detect reuse against — and the
    /// replacement carries whatever family the caller minted. Nothing here
    /// synthesizes one.
    #[instrument(skip(self, live_hash, replacement), fields(token_hash, user_id = %replacement.user_id))]
    async fn rotate_refresh_token(&self, live_hash: &str, replacement: &Session) -> Result<bool> {
        assert!(
            is_valid_family_id(&replacement.family_id),
            "rotate_refresh_token: malformed replacement family id {:?}",
            replacement.family_id
        );
        assert!(
            live_hash != replacement.refresh_token_hash.expose().as_str(),
            "rotate_refresh_token: replacement must be a fresh generation"
        );

        // Identity capture for the retirement record and the roster arm. If
        // the row vanishes before the transaction, the delete's condition
        // fails and this returns false — the read only shapes the statements,
        // never the race.
        let Some(live) = self.get_session_item(live_hash).await? else {
            // CAS condition failed: a concurrent redemption moved (or
            // removed) the live generation first. Nothing has been written.
            return Ok(false);
        };
        let legacy_row = live.family_id.is_empty();
        if !legacy_row {
            assert_eq!(
                live.family_id, replacement.family_id,
                "rotate_refresh_token: family mismatch between live and replacement"
            );
        }
        assert_eq!(
            live.user_id, replacement.user_id,
            "rotate_refresh_token: user mismatch between live and replacement"
        );

        let now = Utc::now();
        let retired_record = (!legacy_row).then(|| {
            retirement_record(
                live_hash,
                &live,
                replacement,
                self.reuse_retention_secs,
                now,
            )
        });

        let live_delete = Delete::builder()
            .table_name(&self.table_name)
            .key("pk", Self::session_pk(live_hash))
            .key("sk", Self::session_sk())
            .condition_expression("attribute_exists(pk)")
            .build()
            .map_err(Self::store_err)?;

        let replacement_put = Put::builder()
            .table_name(&self.table_name)
            .set_item(Some(session_to_item(replacement)))
            .condition_expression("attribute_not_exists(pk)")
            .build()
            .map_err(Self::store_err)?;

        // Roster: the presented hash leaves `sessions`, the replacement joins
        // it, and — for a well-formed family — the presented hash becomes a
        // remembered member while `live` moves to the replacement.
        //
        // `sessions` may only be touched by ONE clause per expression (a
        // DELETE and an ADD on the same path are rejected as overlapping), so
        // the new set is computed here from a strongly consistent roster read
        // and installed with a single SET. That read is safe to build from:
        // TransactWriteItems takes an exclusive lock on the user item, so a
        // concurrent session mutation for the same user cannot interleave —
        // it either committed before this read (then the read sees it) or
        // conflicts with this transaction outright. For a legacy row, whose
        // hash predates rosters and may be absent from the set, filtering is
        // simply a no-op.
        let roster = self.get_user_roster(&replacement.user_id).await?;
        if !legacy_row {
            assert!(
                roster.sessions.contains(&live_hash.to_string()),
                "rotate_refresh_token: roster must name the live generation it rotates"
            );
        }
        let mut new_sessions: Vec<String> = roster
            .sessions
            .iter()
            .filter(|hash| *hash != live_hash)
            .cloned()
            .collect();
        assert!(
            !new_sessions.contains(replacement.refresh_token_hash.expose()),
            "rotate_refresh_token: replacement hash already present in the roster"
        );
        new_sessions.push(replacement.refresh_token_hash.expose().clone());

        // The `#f` placeholder binds the family id — the key inside the
        // `families` map this rotation's entry is filed under.
        let names: Vec<(&str, &str)> =
            vec![("#f", replacement.family_id.as_str()), ("#s", "sessions")];
        let mut values = HashMap::new();
        values.insert(
            ":sess".to_string(),
            AttributeValue::Ss(new_sessions.clone()),
        );
        let expression: String = if legacy_row {
            values.insert(
                ":fresh".to_string(),
                FamilyRoster::new(
                    replacement.refresh_token_hash.expose().clone(),
                    vec![replacement.refresh_token_hash.expose().clone()],
                )
                .to_attribute(),
            );
            "SET #s = :sess, families.#f = :fresh".to_string()
        } else {
            // The family's member set remembers every generation the family
            // has held — the presented one now joins it as a retirement
            // record, and the replacement joins as the live pointer's
            // namesake. One ADD carries both.
            let mut joined_members = vec![live_hash.to_string()];
            joined_members.push(replacement.refresh_token_hash.expose().clone());
            values.insert(":joined".to_string(), AttributeValue::Ss(joined_members));
            values.insert(
                ":newhash".to_string(),
                AttributeValue::S(replacement.refresh_token_hash.expose().clone()),
            );
            "SET #s = :sess, families.#f.live = :newhash ADD families.#f.members :joined"
                .to_string()
        };

        let mut items = vec![
            TransactWriteItem::builder().delete(live_delete).build(),
            TransactWriteItem::builder().put(replacement_put).build(),
        ];
        if let Some(record) = &retired_record {
            let retired_put = Put::builder()
                .table_name(&self.table_name)
                .set_item(Some(retired_to_item(record)))
                .condition_expression("attribute_not_exists(pk)")
                .build()
                .map_err(Self::store_err)?;
            items.push(TransactWriteItem::builder().put(retired_put).build());
        }
        items.push(self.roster_update(
            &replacement.user_id,
            "attribute_exists(pk)",
            &expression,
            &names,
            values,
        )?);

        // Only the live-generation condition (statement 0) cancelling maps to
        // a CAS loss; a failed condition anywhere else is a caller bug or
        // corruption and remains a store error.
        match self
            .client
            .transact_write_items()
            .set_transact_items(Some(items))
            .send()
            .await
        {
            Ok(_) => {}
            Err(err) => {
                if Self::lost_race_reason(&err, 0) {
                    return Ok(false);
                }
                return Err(Self::store_err(err));
            }
        }

        Ok(true)
    }

    /// Delete one live session by hash, keeping the roster in the same
    /// transaction. Idempotent: an unknown hash (or one naming a retirement
    /// record, which is not a session) succeeds without effect. Retirement
    /// records are deliberately untouched.
    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn revoke_session(&self, token_hash: &Secret<String>) -> Result<()> {
        let token_hash = token_hash.expose().as_str();
        assert!(
            !token_hash.is_empty(),
            "revoke_session: token_hash must not be empty"
        );

        let Some(session) = self.get_session_item(token_hash).await? else {
            return Ok(());
        };
        assert!(
            session.family_id.is_empty() || is_valid_family_id(&session.family_id),
            "stored session carries a malformed family id {:?}",
            session.family_id
        );

        let session_delete = Delete::builder()
            .table_name(&self.table_name)
            .key("pk", Self::session_pk(token_hash))
            .key("sk", Self::session_sk())
            .condition_expression("attribute_exists(pk)")
            .build()
            .map_err(Self::store_err)?;

        // Mutual consistency in one unit: if the revoked hash was the
        // family's live pointer, the roster must stop naming it. The pointer
        // is set to the empty sentinel rather than removed — a family whose
        // live generation fell while retirement records remain still has
        // members to remember, and a live-less entry would fail the roster's
        // own parse (every entry always carries its live pointer, empty or
        // not).
        let (expression, names, values) = if session.family_id.is_empty() {
            (
                "DELETE sessions :old",
                vec![],
                HashMap::from([(
                    ":old".to_string(),
                    AttributeValue::Ss(vec![token_hash.to_string()]),
                )]),
            )
        } else {
            (
                "DELETE sessions :old SET families.#f.live = :none",
                vec![("#f", session.family_id.as_str())],
                HashMap::from([
                    (
                        ":old".to_string(),
                        AttributeValue::Ss(vec![token_hash.to_string()]),
                    ),
                    (":none".to_string(), AttributeValue::S(String::new())),
                ]),
            )
        };

        self.client
            .transact_write_items()
            .transact_items(TransactWriteItem::builder().delete(session_delete).build())
            .transact_items(self.roster_update(
                &session.user_id,
                "attribute_exists(pk)",
                expression,
                &names,
                values,
            )?)
            .send()
            .await
            .map_err(Self::store_err)?;

        Ok(())
    }

    /// Remove the family's live generation and every retained retirement
    /// record (SR5), enumerating the authoritative user-item roster under a
    /// strongly consistent read rather than the eventually consistent GSI —
    /// an index can omit a session written moments earlier and strand a live
    /// credential with nothing left to find it. Idempotent: an unknown (but
    /// well-formed) family id removes nothing and returns `Ok(0)`.
    ///
    /// Each attempt reads the family's roster entry, deletes exactly the
    /// items it names, then re-reads the roster: only when the entry still
    /// names exactly what was deleted is the family's roster state removed.
    /// A roster that changed underneath the deletes means a transactional
    /// mutation (rotation, store, revoke) landed mid-flight, so the attempt
    /// retries against the fresh roster — bounded by
    /// [`REVOCATION_MAX_ATTEMPTS`], erroring rather than reporting partial
    /// success. Once the live item is deleted, any concurrent rotation loses
    /// its own CAS (its delete condition needs that item), so the confirming
    /// re-read plus roster removal cannot be overtaken: the window the
    /// protocol closes is the one where the mutation committed *before* the
    /// deletes, and the re-read sees exactly that.
    #[instrument(skip(self), fields(family_id))]
    async fn revoke_family(&self, family_id: &str) -> Result<u64> {
        assert!(
            is_valid_family_id(family_id),
            "revoke_family: malformed family id {family_id:?}"
        );

        let user_ids = self.user_ids_for_family(family_id).await?;

        let mut removed: u64 = 0;
        for user_id in user_ids {
            removed += self.revoke_family_for_user(family_id, &user_id).await?;
        }

        Ok(removed)
    }

    /// Active-session count, cached per [`DynamoRepository::stats_cache_ttl`].
    ///
    /// The count still comes from a walk (a scan over `sk = SESSION` counting
    /// unexpired items) — DynamoDB's TTL reaper deletes sessions without
    /// passing through this adapter, so a maintained counter would drift
    /// downward with nothing to correct it — but the walk runs at most once
    /// per configured TTL window per process, however many
    /// `GET /internal/stats` calls arrive.
    ///
    /// The cache guard is never held across the scan (the committed
    /// `clippy.toml` bans tokio guards across awaits — the same cache-lock
    /// discipline the JWKS cache follows). Election is by re-stamping: a
    /// caller that finds a stale entry refreshes its timestamp *before*
    /// scanning, so concurrent callers arriving during the refresh serve the
    /// stale count — one refresh late is exactly what a TTL'd cache promises —
    /// instead of stampeding into their own full-table reads. Only a cold
    /// cache can admit concurrent walks.
    #[instrument(skip(self))]
    async fn count_active_sessions(&self) -> Result<u64> {
        {
            let mut cache = self.session_count_cache.lock().await;
            if let Some((fetched_at, count)) = *cache {
                // Instant is monotonic, so `elapsed` cannot go negative.
                assert!(
                    fetched_at <= std::time::Instant::now(),
                    "cache timestamps must come from the monotonic clock"
                );
                if fetched_at.elapsed() < self.stats_cache_ttl {
                    return Ok(count);
                }
                // Stale: elect this caller by re-stamping the entry under the
                // guard, then release before the walk below.
                *cache = Some((std::time::Instant::now(), count));
            }
        }

        let count = self.scan_active_session_count().await?;
        assert!(
            self.stats_cache_ttl >= MIN_STATS_CACHE_TTL,
            "the cache TTL is validated at construction and never zero"
        );
        *self.session_count_cache.lock().await = Some((std::time::Instant::now(), count));
        Ok(count)
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        // DynamoDB TTL handles cleanup automatically, but this provides
        // a manual sweep for items where TTL hasn't fired yet.
        let now = Utc::now();
        let mut deleted: u64 = 0;
        let mut exclusive_start_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let mut scan = self
                .client
                .scan()
                .table_name(&self.table_name)
                .filter_expression("sk = :sk AND expires_at < :now")
                .expression_attribute_values(":sk", AttributeValue::S("SESSION".to_string()))
                .expression_attribute_values(":now", AttributeValue::S(now.to_rfc3339()))
                .projection_expression("pk, sk");

            if let Some(ref start_key) = exclusive_start_key {
                scan = scan.set_exclusive_start_key(Some(start_key.clone()));
            }

            let result = scan.send().await.map_err(Self::store_err)?;
            let items = result.items.unwrap_or_default();

            for chunk in items.chunks(25) {
                let delete_requests: Vec<_> = chunk
                    .iter()
                    .map(|item| {
                        let pk = item
                            .get("pk")
                            .cloned()
                            .unwrap_or_else(|| AttributeValue::S("UNKNOWN".to_string()));
                        let sk = item
                            .get("sk")
                            .cloned()
                            .unwrap_or_else(|| AttributeValue::S("UNKNOWN".to_string()));

                        aws_sdk_dynamodb::types::WriteRequest::builder()
                            .delete_request(
                                aws_sdk_dynamodb::types::DeleteRequest::builder()
                                    .key("pk", pk)
                                    .key("sk", sk)
                                    .build()
                                    .expect("valid delete request"),
                            )
                            .build()
                    })
                    .collect();

                // Count deletions from the batch actually drained (every unprocessed item
                // retried until it succeeds), not from the number submitted.
                deleted += self.batch_write_with_retry(delete_requests).await?;
            }

            match result.last_evaluated_key {
                Some(key) => exclusive_start_key = Some(key),
                None => break,
            }
        }

        Ok(deleted)
    }

    /// Revoke every family the user holds (SR5 across all of them),
    /// enumerating the authoritative user-item roster under a strongly
    /// consistent read rather than the eventually consistent GSI — an index
    /// can omit a session written moments earlier and strand that credential
    /// forever (`g3-dynamo-revoke-all-gsi-incompleteness`).
    ///
    /// The same converging protocol as `revoke_family`, scoped to the whole
    /// roster: read, delete exactly what is named, confirm the roster did not
    /// change mid-flight, then clear it — retrying bounded times when a
    /// transactional mutation landed between the reads, erroring once the
    /// budget is exhausted rather than reporting incomplete success.
    #[instrument(skip(self), fields(user_id))]
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()> {
        assert!(
            !user_id.is_empty(),
            "revoke_all_user_sessions: user_id must not be empty"
        );

        for _attempt in 1..=REVOCATION_MAX_ATTEMPTS {
            let roster = self.get_user_roster(user_id).await?;
            if roster.sessions.is_empty() && roster.families.is_empty() {
                // Nothing to revoke; also the post-clear steady state, so a
                // repeated call is a cheap no-op.
                return Ok(());
            }

            // Every live generation the roster names, plus a retirement record
            // for every remembered member that is not some family's live hash.
            let live_hashes: std::collections::HashSet<&String> =
                roster.families.values().map(|f| &f.live).collect();
            let mut delete_keys: Vec<(AttributeValue, AttributeValue)> = Vec::new();
            for hash in &roster.sessions {
                delete_keys.push((Self::session_pk(hash), Self::session_sk()));
            }
            for family in roster.families.values() {
                for hash in &family.members {
                    if !live_hashes.contains(hash) {
                        delete_keys.push((
                            Self::retired_pk(hash),
                            AttributeValue::S(RETIRED_SK.to_string()),
                        ));
                    }
                }
            }
            assert!(
                !delete_keys.is_empty(),
                "non-empty roster must name at least one deletable entry"
            );

            for chunk in delete_keys.chunks(BATCH_WRITE_MAX_ITEMS) {
                let requests = chunk
                    .iter()
                    .map(|(pk, sk)| {
                        WriteRequest::builder()
                            .delete_request(
                                DeleteRequest::builder()
                                    .key("pk", pk.clone())
                                    .key("sk", sk.clone())
                                    .build()
                                    .expect("valid delete request"),
                            )
                            .build()
                    })
                    .collect();
                self.batch_write_with_retry(requests).await?;
            }

            // Confirm-then-clear, mirroring revoke_family: only clear the
            // roster once it still names exactly what was deleted.
            let fresh = self.get_user_roster(user_id).await?;
            let unchanged = fresh.sessions.len() == roster.sessions.len()
                && fresh
                    .sessions
                    .iter()
                    .all(|hash| roster.sessions.contains(hash))
                && fresh.families.len() == roster.families.len()
                && roster.families.iter().all(|(id, family)| {
                    fresh.families.get(id).is_some_and(|f| {
                        f.live == family.live && same_members(&f.members, &family.members)
                    })
                });
            if !unchanged {
                continue;
            }

            self.client
                .update_item()
                .table_name(&self.table_name)
                .key("pk", AttributeValue::S(format!("USER#{user_id}")))
                .key("sk", Self::user_sk())
                .update_expression("REMOVE #s, families")
                .expression_attribute_names("#s", "sessions")
                .condition_expression("attribute_exists(pk)")
                .send()
                .await
                .map_err(Self::store_err)?;

            return Ok(());
        }

        Err(Error::StoreError {
            detail: format!(
                "revoke_all_user_sessions for {user_id} exhausted its retry budget \
                 ({REVOCATION_MAX_ATTEMPTS} attempts) racing concurrent session mutations"
            ),
        })
    }

    #[instrument(skip(self, key))]
    async fn put_single_use(&self, key: &str, expires_at: chrono::DateTime<Utc>) -> Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        let now = Utc::now();
        let outcome = self
            .client
            .put_item()
            .table_name(&self.table_name)
            .set_item(Some(single_use_to_item(key, expires_at)))
            // One conditional PutItem is the whole claim: it succeeds only when no item
            // exists at the key, or the one that does has already expired — so exactly
            // one of N racing claims can win, and an expired marker's key stays
            // reusable without any sweep having run.
            .condition_expression("attribute_not_exists(pk) OR expires_at < :now")
            .expression_attribute_values(":now", AttributeValue::N(now.timestamp().to_string()))
            .send()
            .await;

        match outcome {
            Ok(_) => Ok(true),
            Err(err) => {
                let lost_race = matches!(
                    err.as_service_error(),
                    Some(aws_sdk_dynamodb::operation::put_item::PutItemError::ConditionalCheckFailedException(_))
                );
                if lost_race {
                    Ok(false)
                } else {
                    Err(Self::store_err(err))
                }
            }
        }
    }

    #[instrument(skip(self, key))]
    async fn take_single_use(&self, key: &str) -> Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        // Delete-and-inspect in one atomic call: ALL_OLD returns the deleted item, so
        // liveness of what this call removed is decided from its stored `expires_at`.
        // An absent key returns no attributes; an expired one (TTL not yet fired)
        // returns attributes whose expiry has passed — both report false.
        let result = self
            .client
            .delete_item()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(single_use_pk(key)))
            .key("sk", AttributeValue::S(schema::SINGLE_USE_SK.to_string()))
            .return_values(aws_sdk_dynamodb::types::ReturnValue::AllOld)
            .send()
            .await
            .map_err(Self::store_err)?;

        match result.attributes {
            Some(item) => {
                // Re-validate the stored expiry (store reads never trust stored data),
                // then report liveness: an item whose expiry has passed but which TTL
                // has not yet reaped counts as absent.
                let expires_at = item_single_use_expiry(&item)?;
                Ok(expires_at > Utc::now())
            }
            None => Ok(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests for the BatchWriteItem retry helper (no DynamoDB Local required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod retry_tests {
    use std::cell::Cell;

    use aws_sdk_dynamodb::types::{AttributeValue, DeleteRequest, WriteRequest};

    use super::{drain_unprocessed, BATCH_WRITE_MAX_ATTEMPTS};
    use oidc_exchange_core::error::Error;

    fn write_request(id: &str) -> WriteRequest {
        WriteRequest::builder()
            .delete_request(
                DeleteRequest::builder()
                    .key("pk", AttributeValue::S(format!("SESSION#{id}")))
                    .key("sk", AttributeValue::S("SESSION".to_string()))
                    .build()
                    .expect("valid delete request"),
            )
            .build()
    }

    /// A fake client that reports the last `unprocessed_after` items of a batch as
    /// `unprocessed_items` for its first `attempts_before_drain - 1` calls, then reports the
    /// whole batch as processed (an empty vec) from that attempt onward.
    fn flaky_submit(
        attempts_before_drain: u32,
        calls: &Cell<u32>,
    ) -> impl FnMut(
        Vec<WriteRequest>,
    ) -> std::future::Ready<oidc_exchange_core::error::Result<Vec<WriteRequest>>>
           + '_ {
        move |pending| {
            let attempt = calls.get() + 1;
            calls.set(attempt);
            let outcome = if attempt < attempts_before_drain {
                // Only the last item in the batch is still unprocessed.
                pending.into_iter().rev().take(1).collect()
            } else {
                Vec::new()
            };
            std::future::ready(Ok(outcome))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn drains_within_budget_and_reports_true_deleted_count() {
        let requests = vec![write_request("a"), write_request("b"), write_request("c")];
        let submitted = requests.len() as u64;

        let calls = Cell::new(0u32);
        let attempts_before_drain = 4u32;
        assert!(
            attempts_before_drain <= BATCH_WRITE_MAX_ATTEMPTS,
            "test setup must fit inside the retry budget"
        );

        let result = drain_unprocessed(requests, flaky_submit(attempts_before_drain, &calls)).await;

        assert_eq!(
            result.expect("retry loop should drain within the budget"),
            submitted,
            "should report every originally submitted item as deleted"
        );
        assert_eq!(
            calls.get(),
            attempts_before_drain,
            "should stop retrying as soon as the batch drains, not keep spinning"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn errors_when_retry_budget_is_exhausted_without_draining() {
        let requests = vec![write_request("a")];

        // Every submission reports the item as still unprocessed — it never drains.
        let result = drain_unprocessed(requests, |pending| std::future::ready(Ok(pending))).await;

        match result {
            Err(Error::StoreError { detail }) => {
                assert!(
                    detail.contains("unprocessed"),
                    "error should explain the batch never drained: {detail}"
                );
            }
            other => panic!("expected Error::StoreError on budget exhaustion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_batch_is_a_no_op() {
        let calls = Cell::new(0u32);
        let result = drain_unprocessed(Vec::new(), |pending| {
            calls.set(calls.get() + 1);
            std::future::ready(Ok(pending))
        })
        .await;

        assert_eq!(result.expect("empty batch should succeed"), 0);
        assert_eq!(calls.get(), 0, "an empty batch should never call submit");
    }

    // -----------------------------------------------------------------------
    // `retry_on_version_conflict` (update_user's retry driver)
    // -----------------------------------------------------------------------

    use super::{retry_on_version_conflict, UPDATE_MAX_ATTEMPTS};
    use oidc_exchange_core::domain::{User, UserStatus};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Negative-space: when every attempt's version-conditioned write loses the race (the
    /// item's `version` "keeps changing" out from under it), `retry_on_version_conflict`
    /// must exhaust `UPDATE_MAX_ATTEMPTS` and return `Error::Conflict` — not loop unbounded
    /// or silently report success. Mirrors [`errors_when_retry_budget_is_exhausted_without_draining`]'s
    /// technique for `drain_unprocessed`: inject a closure that always reports a conflict so
    /// budget exhaustion is deterministically testable without a live, racing table.
    #[tokio::test]
    async fn retry_on_version_conflict_errors_when_every_attempt_conflicts() {
        let calls = AtomicU32::new(0);

        let result = retry_on_version_conflict("usr_relentless", |attempt_number| {
            calls.fetch_add(1, Ordering::Relaxed);
            assert!(
                (1..=UPDATE_MAX_ATTEMPTS).contains(&attempt_number),
                "attempt numbers should stay within the bound"
            );
            std::future::ready(Ok(None))
        })
        .await;

        match result {
            Err(Error::Conflict { detail }) => {
                assert!(
                    detail.contains("usr_relentless") && detail.contains("exhausted"),
                    "error should name the user and explain budget exhaustion: {detail}"
                );
            }
            other => panic!("expected Error::Conflict on retry-budget exhaustion, got {other:?}"),
        }
        assert_eq!(
            calls.load(Ordering::Relaxed),
            UPDATE_MAX_ATTEMPTS,
            "should make exactly UPDATE_MAX_ATTEMPTS attempts, no more and no fewer"
        );
    }

    /// The mirror-image happy path: a conflict on the first attempts must not abort the
    /// retry — it should keep trying and return the eventual success once the write stops
    /// losing the race, well within the budget.
    #[tokio::test]
    async fn retry_on_version_conflict_succeeds_once_a_later_attempt_wins() {
        let calls = AtomicU32::new(0);
        let winning_attempt = 3u32;

        let result = retry_on_version_conflict("usr_eventually_wins", |attempt_number| {
            calls.fetch_add(1, Ordering::Relaxed);
            std::future::ready(if attempt_number == winning_attempt {
                Ok(Some(User {
                    id: "usr_eventually_wins".to_string(),
                    external_id: "google|eventual".to_string(),
                    provider: "google".to_string(),
                    email: None,
                    display_name: None,
                    metadata: std::collections::HashMap::new(),
                    claims: std::collections::HashMap::new(),
                    status: UserStatus::Active,
                    version: attempt_number as u64 + 1,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
            } else {
                Ok(None)
            })
        })
        .await
        .expect("should eventually succeed within the budget");

        assert_eq!(result.id, "usr_eventually_wins");
        assert_eq!(
            calls.load(Ordering::Relaxed),
            winning_attempt,
            "should stop retrying as soon as an attempt succeeds, not keep spinning"
        );
    }
}

// ---------------------------------------------------------------------------
// Integration tests (require DynamoDB Local)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_dynamodb::types::{
        AttributeDefinition, GlobalSecondaryIndex, KeySchemaElement, KeyType, Projection,
        ProjectionType, ProvisionedThroughput, ScalarAttributeType,
    };

    /// Reuse-retention window used by every test repository: one hour — short
    /// enough that deadline arithmetic stays inside a test's lifetime, and
    /// positive per the constructor's precondition.
    const TEST_REUSE_RETENTION_SECS: u64 = 3600;

    /// Name of the global secondary index the test tables create. Revocation
    /// paths deliberately do not read it (they enumerate the user-item
    /// roster), but session/retirement items file GSI1 keys under it for the
    /// admin listing paths.
    const GSI1_NAME: &str = "GSI1";

    async fn create_test_client() -> aws_sdk_dynamodb::Client {
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .endpoint_url("http://localhost:8000")
            .region(aws_config::Region::new("us-east-1"))
            .credentials_provider(aws_sdk_dynamodb::config::Credentials::new(
                "fakeAccessKey",
                "fakeSecretKey",
                None,
                None,
                "test",
            ))
            .load()
            .await;

        aws_sdk_dynamodb::Client::new(&config)
    }

    async fn create_test_table(client: &aws_sdk_dynamodb::Client, table_name: &str) {
        // Delete if exists (ignore errors)
        let _ = client.delete_table().table_name(table_name).send().await;

        client
            .create_table()
            .table_name(table_name)
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("pk")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .unwrap(),
            )
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("sk")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .unwrap(),
            )
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("GSI1pk")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .unwrap(),
            )
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("GSI1sk")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .unwrap(),
            )
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name("pk")
                    .key_type(KeyType::Hash)
                    .build()
                    .unwrap(),
            )
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name("sk")
                    .key_type(KeyType::Range)
                    .build()
                    .unwrap(),
            )
            .global_secondary_indexes(
                GlobalSecondaryIndex::builder()
                    .index_name(GSI1_NAME)
                    .key_schema(
                        KeySchemaElement::builder()
                            .attribute_name("GSI1pk")
                            .key_type(KeyType::Hash)
                            .build()
                            .unwrap(),
                    )
                    .key_schema(
                        KeySchemaElement::builder()
                            .attribute_name("GSI1sk")
                            .key_type(KeyType::Range)
                            .build()
                            .unwrap(),
                    )
                    .projection(
                        Projection::builder()
                            .projection_type(ProjectionType::All)
                            .build(),
                    )
                    .provisioned_throughput(
                        ProvisionedThroughput::builder()
                            .read_capacity_units(5)
                            .write_capacity_units(5)
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .provisioned_throughput(
                ProvisionedThroughput::builder()
                    .read_capacity_units(5)
                    .write_capacity_units(5)
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("failed to create test table");
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn dynamo_repository_crud() {
        let table_name = "oidc-exchange-test";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;

        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        // --- User CRUD ---

        // Create user
        let new_user = NewUser {
            external_id: "google|user123".to_string(),
            provider: "google".to_string(),
            email: Some("alice@example.com".to_string()),
            display_name: Some("Alice".to_string()),
        };
        let created = repo.create_user(&new_user).await.expect("create_user");
        assert!(created.id.starts_with("usr_"));
        assert_eq!(created.external_id, "google|user123");
        assert_eq!(created.provider, "google");
        assert_eq!(created.email.as_deref(), Some("alice@example.com"));
        assert_eq!(created.status, UserStatus::Active);
        assert_eq!(created.version, INITIAL_USER_VERSION);

        // Get user by ID
        let fetched = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("user should exist");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.external_id, "google|user123");
        assert_eq!(fetched.version, created.version);

        // Get user by external ID
        let fetched_ext = repo
            .get_user_by_external_id("google|user123", "google")
            .await
            .expect("get_user_by_external_id")
            .expect("user should exist");
        assert_eq!(fetched_ext.id, created.id);

        // Get non-existent user
        let none = repo
            .get_user_by_id("usr_nonexistent")
            .await
            .expect("get_user_by_id");
        assert!(none.is_none());

        // Update user
        let patch = UserPatch {
            email: Some("alice-new@example.com".to_string()),
            display_name: None,
            metadata: Some({
                let mut m = HashMap::new();
                m.insert(
                    "key".to_string(),
                    serde_json::Value::String("val".to_string()),
                );
                m
            }),
            claims: None,
            status: None,
        };
        let updated = repo
            .update_user(&created.id, &patch)
            .await
            .expect("update_user");
        assert_eq!(updated.email.as_deref(), Some("alice-new@example.com"));
        assert_eq!(
            updated.metadata.get("key"),
            Some(&serde_json::Value::String("val".to_string()))
        );

        // Delete user (soft delete)
        repo.delete_user(&created.id).await.expect("delete_user");
        let deleted = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("user should still exist");
        assert_eq!(deleted.status, UserStatus::Deleted);

        // --- Session CRUD ---

        let now = Utc::now();
        let session = Session {
            user_id: created.id.clone(),
            refresh_token_hash: Secret::new("hash_abc123".to_string()),
            family_id: "fam_0000000000000000000000000a".to_string(),
            generation: 0,
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(24),
            rotated_at: None,
            device_id: Some("device-1".to_string()),
            user_agent: Some("test-agent".to_string()),
            ip_address: Some("10.0.0.1".to_string()),
            created_at: now,
        };

        // Store session
        repo.store_refresh_token(&session)
            .await
            .expect("store_refresh_token");

        // Get session
        let fetched_session = repo
            .get_session_by_refresh_token(&Secret::new("hash_abc123".to_string()))
            .await
            .expect("get_session_by_refresh_token")
            .expect("session should exist");
        assert_eq!(fetched_session.user_id, created.id);
        assert!(
            fetched_session.refresh_token_hash == Secret::new("hash_abc123".to_string()),
            "fetched digest must match the stored one"
        );
        assert_eq!(fetched_session.device_id.as_deref(), Some("device-1"));

        // Get non-existent session
        let none = repo
            .get_session_by_refresh_token(&Secret::new("hash_nonexistent".to_string()))
            .await
            .expect("get_session_by_refresh_token");
        assert!(none.is_none());

        // Store a second session for the same user
        let session2 = Session {
            user_id: created.id.clone(),
            refresh_token_hash: Secret::new("hash_def456".to_string()),
            family_id: "fam_0000000000000000000000000b".to_string(),
            generation: 0,
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(24),
            rotated_at: None,
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: now,
        };
        repo.store_refresh_token(&session2)
            .await
            .expect("store second session");

        // Revoke single session
        repo.revoke_session(&Secret::new("hash_abc123".to_string()))
            .await
            .expect("revoke_session");
        let revoked = repo
            .get_session_by_refresh_token(&Secret::new("hash_abc123".to_string()))
            .await
            .expect("get after revoke");
        assert!(revoked.is_none());

        // The other session should still exist
        let still_exists = repo
            .get_session_by_refresh_token(&Secret::new("hash_def456".to_string()))
            .await
            .expect("get other session");
        assert!(still_exists.is_some());

        // Revoke all user sessions
        // First re-create the first session
        repo.store_refresh_token(&session)
            .await
            .expect("re-store session");

        repo.revoke_all_user_sessions(&created.id)
            .await
            .expect("revoke_all_user_sessions");

        let s1 = repo
            .get_session_by_refresh_token(&Secret::new("hash_abc123".to_string()))
            .await
            .expect("get after revoke_all");
        let s2 = repo
            .get_session_by_refresh_token(&Secret::new("hash_def456".to_string()))
            .await
            .expect("get after revoke_all");
        assert!(s1.is_none());
        assert!(s2.is_none());

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn create_user_writes_profile_and_guard_items() {
        let table_name = "oidc-exchange-test-guard-write";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;

        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        let new_user = NewUser {
            external_id: "google|guard_write_test".to_string(),
            provider: "google".to_string(),
            email: Some("guard_write@example.com".to_string()),
            display_name: None,
        };
        let created = repo.create_user(&new_user).await.expect("create_user");

        // The profile item exists under USER#<id> / PROFILE.
        let profile = client
            .get_item()
            .table_name(table_name)
            .key("pk", AttributeValue::S(format!("USER#{}", created.id)))
            .key("sk", AttributeValue::S("PROFILE".to_string()))
            .send()
            .await
            .expect("get profile item")
            .item
            .expect("profile item should exist");
        assert_eq!(
            profile.get("id").and_then(|v| v.as_s().ok()),
            Some(&created.id)
        );

        // The uniqueness-guard item exists under EXT#<provider>#<external_id> / UNIQUE and
        // carries the same user_id as the profile item just written.
        let guard = client
            .get_item()
            .table_name(table_name)
            .key(
                "pk",
                AttributeValue::S("EXT#google#google|guard_write_test".to_string()),
            )
            .key("sk", AttributeValue::S("UNIQUE".to_string()))
            .send()
            .await
            .expect("get guard item")
            .item
            .expect("guard item should exist");
        assert_eq!(
            guard.get("user_id").and_then(|v| v.as_s().ok()),
            Some(&created.id),
            "guard item should carry the owning user_id"
        );

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn concurrent_create_user_same_identity_yields_one_user_and_one_conflict() {
        let table_name = "oidc-exchange-test-guard-conflict";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;

        let new_user = NewUser {
            external_id: "google|guard_conflict_test".to_string(),
            provider: "google".to_string(),
            email: Some("guard_conflict@example.com".to_string()),
            display_name: None,
        };

        let repo_a = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );
        let repo_b = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );
        let user_a = new_user.clone();
        let user_b = new_user.clone();

        // Race two create_user calls for the same (provider, external_id) so the guard's
        // `attribute_not_exists(pk)` condition decides exactly one winner.
        let (result_a, result_b) = tokio::join!(
            tokio::spawn(async move { repo_a.create_user(&user_a).await }),
            tokio::spawn(async move { repo_b.create_user(&user_b).await })
        );
        let result_a = result_a.expect("task a should not panic");
        let result_b = result_b.expect("task b should not panic");

        let outcomes = [result_a, result_b];
        let successes: Vec<_> = outcomes.iter().filter(|r| r.is_ok()).collect();
        let conflicts: Vec<_> = outcomes
            .iter()
            .filter(|r| matches!(r, Err(Error::Conflict { .. })))
            .collect();

        assert_eq!(
            successes.len(),
            1,
            "exactly one racer should create the user"
        );
        assert_eq!(
            conflicts.len(),
            1,
            "exactly one racer should lose to the guard's condition with Conflict"
        );

        let winner = successes[0].as_ref().expect("checked is_ok above");
        assert_eq!(winner.external_id, "google|guard_conflict_test");
        if let Err(Error::Conflict { detail }) = conflicts[0] {
            assert!(
                !detail.is_empty(),
                "Conflict detail should describe the collision"
            );
        }

        // The persisted guard item points at the single winner, not the loser.
        let guard = client
            .get_item()
            .table_name(table_name)
            .key(
                "pk",
                AttributeValue::S("EXT#google#google|guard_conflict_test".to_string()),
            )
            .key("sk", AttributeValue::S("UNIQUE".to_string()))
            .send()
            .await
            .expect("get guard item")
            .item
            .expect("guard item should exist after the race resolves");
        assert_eq!(
            guard.get("user_id").and_then(|v| v.as_s().ok()),
            Some(&winner.id),
            "guard item should carry the winning racer's user_id"
        );

        // Counter integrity under conditional failure: the losing racer's
        // cancelled transaction must not have moved STATS#USERS — one live
        // user means exactly one counted user.
        let repo_after = DynamoRepository::new(client.clone(), table_name.to_string(), 60);
        let counts = repo_after
            .count_by_status()
            .await
            .expect("count_by_status after the create race");
        assert_eq!(
            counts.get("active"),
            Some(&1),
            "the losing create's conditional failure must not double-count"
        );
        assert_eq!(counts.get("deleted").copied().unwrap_or(0), 0);
        assert_eq!(counts.get("suspended").copied().unwrap_or(0), 0);

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn create_user_non_conditional_failure_maps_to_store_error() {
        // Point the repository at a table that was never created, so `TransactWriteItems`
        // fails with `ResourceNotFoundException` — a transaction failure that is not a
        // conditional-check cancellation — and must map to `StoreError`, not `Conflict`.
        let client = create_test_client().await;
        let repo = DynamoRepository::new(
            client,
            "oidc-exchange-test-nonexistent".to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        let new_user = NewUser {
            external_id: "google|guard_store_error_test".to_string(),
            provider: "google".to_string(),
            email: None,
            display_name: None,
        };

        let err = repo
            .create_user(&new_user)
            .await
            .expect_err("create_user against a missing table should fail");

        match err {
            Error::StoreError { detail } => {
                assert!(!detail.is_empty(), "StoreError detail should not be empty");
            }
            other => panic!("expected Error::StoreError, got {other:?}"),
        }
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn backfill_writes_guards_for_legacy_users_and_is_idempotent() {
        let table_name = "oidc-exchange-test-guard-backfill";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;

        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        // Simulate a user written before the guard existed: a profile item written
        // directly, bypassing `create_user`'s transactional guard write.
        let now = Utc::now();
        let legacy_user = User {
            id: "usr_legacy_backfill".to_string(),
            external_id: "google|legacy_backfill_test".to_string(),
            provider: "google".to_string(),
            email: None,
            display_name: None,
            metadata: HashMap::new(),
            claims: HashMap::new(),
            status: UserStatus::Active,
            version: INITIAL_USER_VERSION,
            created_at: now,
            updated_at: now,
        };
        client
            .put_item()
            .table_name(table_name)
            .set_item(Some(user_to_item(&legacy_user)))
            .send()
            .await
            .expect("write legacy profile item without a guard");

        // A user created through the normal path already carries a guard; the backfill
        // must not double-write or error on it.
        let created = repo
            .create_user(&NewUser {
                external_id: "google|backfill_already_guarded".to_string(),
                provider: "google".to_string(),
                email: None,
                display_name: None,
            })
            .await
            .expect("create_user");

        let first_run = repo
            .backfill_uniqueness_guards()
            .await
            .expect("first backfill run");
        assert_eq!(
            first_run, 1,
            "backfill should write exactly one guard, for the legacy user only"
        );

        let guard = client
            .get_item()
            .table_name(table_name)
            .key(
                "pk",
                AttributeValue::S("EXT#google#google|legacy_backfill_test".to_string()),
            )
            .key("sk", AttributeValue::S("UNIQUE".to_string()))
            .send()
            .await
            .expect("get backfilled guard item")
            .item
            .expect("backfilled guard item should exist");
        assert_eq!(
            guard.get("user_id").and_then(|v| v.as_s().ok()),
            Some(&legacy_user.id),
            "backfilled guard should carry the legacy user's id"
        );

        // Re-running is a no-op: every user now has a guard, so nothing new is written.
        let second_run = repo
            .backfill_uniqueness_guards()
            .await
            .expect("second backfill run should be idempotent");
        assert_eq!(
            second_run, 0,
            "re-running backfill should write nothing new"
        );

        // The already-guarded user's original guard is untouched (still points at its
        // own id, not overwritten by anything from the backfill pass).
        let untouched_guard = client
            .get_item()
            .table_name(table_name)
            .key(
                "pk",
                AttributeValue::S("EXT#google#google|backfill_already_guarded".to_string()),
            )
            .key("sk", AttributeValue::S("UNIQUE".to_string()))
            .send()
            .await
            .expect("get pre-existing guard item")
            .item
            .expect("pre-existing guard item should still exist");
        assert_eq!(
            untouched_guard.get("user_id").and_then(|v| v.as_s().ok()),
            Some(&created.id)
        );

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn get_user_by_external_id_resolves_through_guard_then_profile() {
        let table_name = "oidc-exchange-test-guard-lookup";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;

        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        // Create via the normal transactional path (task 06), which writes both the
        // profile item and the uniqueness-guard item in one transaction.
        let created = repo
            .create_user(&NewUser {
                external_id: "google|guard_lookup_test".to_string(),
                provider: "google".to_string(),
                email: Some("guard_lookup@example.com".to_string()),
                display_name: Some("Guard Lookup".to_string()),
            })
            .await
            .expect("create_user");

        // The lookup must resolve the guard item to a user_id, then read the profile at
        // USER#<id>/PROFILE, and return the same user that was created.
        let found = repo
            .get_user_by_external_id("google|guard_lookup_test", "google")
            .await
            .expect("get_user_by_external_id")
            .expect("user should be found via the guard");
        assert_eq!(found.id, created.id);
        assert_eq!(found.external_id, "google|guard_lookup_test");
        assert_eq!(found.provider, "google");
        assert_eq!(found.email.as_deref(), Some("guard_lookup@example.com"));
        assert_eq!(found.display_name.as_deref(), Some("Guard Lookup"));

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn get_user_by_external_id_with_no_guard_item_returns_none() {
        let table_name = "oidc-exchange-test-guard-lookup-miss";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;

        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        // Create an unrelated user so the table is non-empty, but never a guard for the
        // identity being looked up.
        repo.create_user(&NewUser {
            external_id: "google|some_other_identity".to_string(),
            provider: "google".to_string(),
            email: None,
            display_name: None,
        })
        .await
        .expect("create_user for unrelated identity");

        // No guard item exists for this (provider, external_id) pair, so the lookup must
        // return `None` rather than an error or an arbitrary user.
        let missing = repo
            .get_user_by_external_id("google|no_such_identity", "google")
            .await
            .expect("get_user_by_external_id should not error on a missing guard");
        assert!(
            missing.is_none(),
            "lookup for an identity with no guard item must return None"
        );

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn racing_suspend_and_claims_patch_ends_suspended() {
        let table_name = "oidc-exchange-test-version-race";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;

        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        let created = repo
            .create_user(&NewUser {
                external_id: "google|version_race_test".to_string(),
                provider: "google".to_string(),
                email: Some("race@example.com".to_string()),
                display_name: None,
            })
            .await
            .expect("create_user");
        assert_eq!(created.version, INITIAL_USER_VERSION);

        let repo_a = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );
        let repo_b = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );
        let user_id_a = created.id.clone();
        let user_id_b = created.id.clone();

        let suspend_patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
            status: Some(UserStatus::Suspended),
        };
        let mut claims = HashMap::new();
        claims.insert(
            "org_id".to_string(),
            serde_json::Value::String("org_racing".to_string()),
        );
        let claims_patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: Some(claims),
            status: None,
        };

        // Race a suspend patch against an unrelated claims patch, both reading the same
        // starting `version`. The version-conditional write lets exactly one land per
        // attempt; the other retries against the fresh version until it too succeeds.
        let (result_a, result_b) = tokio::join!(
            tokio::spawn(async move { repo_a.update_user(&user_id_a, &suspend_patch).await }),
            tokio::spawn(async move { repo_b.update_user(&user_id_b, &claims_patch).await })
        );

        result_a
            .expect("task a should not panic")
            .expect("suspend patch should eventually succeed");
        result_b
            .expect("task b should not panic")
            .expect("claims patch should eventually succeed");

        let final_user = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("user should exist");

        // Both racing writes landed — neither silently reverted the other — and `version`
        // advanced by exactly one per successful write.
        assert_eq!(final_user.status, UserStatus::Suspended);
        assert_eq!(
            final_user.claims.get("org_id"),
            Some(&serde_json::Value::String("org_racing".to_string()))
        );
        assert_eq!(final_user.version, INITIAL_USER_VERSION + 2);

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn delete_user_removes_guard_and_frees_identity_for_recreation() {
        let table_name = "oidc-exchange-test-delete-frees-identity";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;

        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        let new_user = NewUser {
            external_id: "google|delete_frees_test".to_string(),
            provider: "google".to_string(),
            email: Some("delete_frees@example.com".to_string()),
            display_name: None,
        };
        let original = repo.create_user(&new_user).await.expect("create_user");

        repo.delete_user(&original.id).await.expect("delete_user");

        // The guard item must be removed by the very same `TransactWriteItems` call as
        // the status write — both succeed or neither does.
        let guard_after_delete = client
            .get_item()
            .table_name(table_name)
            .key(
                "pk",
                AttributeValue::S(guard_pk("google", "google|delete_frees_test")),
            )
            .key("sk", AttributeValue::S(GUARD_SK.to_string()))
            .send()
            .await
            .expect("get guard item after delete")
            .item;
        assert!(
            guard_after_delete.is_none(),
            "guard item should be removed in the same transaction as the status write"
        );

        // The profile row is retained (soft delete), not purged.
        let profile_after_delete = repo
            .get_user_by_id(&original.id)
            .await
            .expect("get_user_by_id")
            .expect("deleted row should still exist");
        assert_eq!(profile_after_delete.status, UserStatus::Deleted);

        // A deleted user must not satisfy the external-id lookup.
        let looked_up = repo
            .get_user_by_external_id("google|delete_frees_test", "google")
            .await
            .expect("get_user_by_external_id");
        assert!(
            looked_up.is_none(),
            "deleted user must not be returned by external-id lookup"
        );

        // The identity is free: create_user for the same (provider, external_id) succeeds
        // as a brand-new user with a fresh id and no carried-over claims.
        let recreated = repo
            .create_user(&new_user)
            .await
            .expect("recreate after delete should succeed, not conflict");
        assert_ne!(
            recreated.id, original.id,
            "recreated user must get a fresh id"
        );
        assert!(
            recreated.claims.is_empty(),
            "recreated user must start with no carried-over claims"
        );

        // Lookup now resolves to the recreated user via the fresh guard item.
        let found = repo
            .get_user_by_external_id("google|delete_frees_test", "google")
            .await
            .expect("get_user_by_external_id")
            .expect("recreated user should be found");
        assert_eq!(found.id, recreated.id);

        // Negative-space: a live duplicate against the recreated user still conflicts —
        // deletion frees the id, it does not disable uniqueness among live users.
        let err = repo
            .create_user(&new_user)
            .await
            .expect_err("a second live duplicate must still conflict");
        match err {
            Error::Conflict { .. } => {}
            other => panic!("expected Error::Conflict, got {other:?}"),
        }

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn retried_delete_of_already_deleted_user_does_not_evict_a_recreated_users_guard() {
        // Regression test: create A -> delete A -> recreate B for the same identity ->
        // delete_user(A.id) again (simulating a retried DELETE /internal/users/:id or a
        // duplicate PATCH status=deleted). The second delete must be a no-op with respect
        // to B's guard: it must NOT remove the guard that now belongs to the recreated,
        // still-`Active` user B. Before the fix, `is_delete_transition` was computed from
        // `patch.status` alone (ignoring the user's pre-patch status), so the second
        // delete unconditionally deleted the `EXT#<provider>#<external_id>` guard item —
        // which by then belonged to B — freeing the identity while B was still live and
        // allowing a third `create_user` to produce a second, simultaneously-live user for
        // the same `(provider, external_id)`.
        let table_name = "oidc-exchange-test-retried-delete-guard-safety";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;

        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        let new_user = NewUser {
            external_id: "google|retried_delete_test".to_string(),
            provider: "google".to_string(),
            email: Some("retried_delete@example.com".to_string()),
            display_name: None,
        };

        let user_a = repo.create_user(&new_user).await.expect("create A");
        repo.delete_user(&user_a.id).await.expect("delete A");

        let user_b = repo
            .create_user(&new_user)
            .await
            .expect("recreate B after A's delete freed the identity");
        assert_ne!(user_b.id, user_a.id, "B must be a fresh user, not A");

        // Re-delete A. This must succeed (or at least not corrupt state) but must not
        // touch B's guard, since the pre-patch status read for A is already `Deleted`.
        let _ = repo.delete_user(&user_a.id).await;

        // B's guard must be intact: B is still resolvable by external-id lookup.
        let looked_up = repo
            .get_user_by_external_id("google|retried_delete_test", "google")
            .await
            .expect("get_user_by_external_id after repeated delete of A");
        assert!(
            looked_up.is_some(),
            "B's guard must survive a repeated delete of the unrelated, already-deleted A"
        );
        let looked_up = looked_up.expect("checked above");
        assert_eq!(
            looked_up.id, user_b.id,
            "lookup must resolve to B, not a ghost"
        );
        assert_eq!(
            looked_up.status,
            UserStatus::Active,
            "B must still be Active — the repeated delete of A must not have deleted B"
        );

        // A third create for the same identity must conflict with the still-live B, not
        // silently create a second live user sharing the identity.
        let err = repo
            .create_user(&new_user)
            .await
            .expect_err("a third create while B is still live must conflict, not succeed");
        match err {
            Error::Conflict { .. } => {}
            other => panic!("expected Error::Conflict, got {other:?}"),
        }

        // Counter integrity across the whole delete/recreate cycle: one live
        // user and one soft-deleted user must be exactly what the
        // transactionally-maintained counter reports.
        let counts = repo.count_by_status().await.expect("count_by_status");
        assert_eq!(counts.get("active").copied().unwrap_or(0), 1);
        assert_eq!(counts.get("deleted").copied().unwrap_or(0), 1);
        assert_eq!(
            counts.get("suspended").copied().unwrap_or(0),
            0,
            "no suspended users exist in this scenario"
        );

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    // -----------------------------------------------------------------
    // Bounded admin reads, measured by consumed capacity (task 08)
    // -----------------------------------------------------------------

    use std::sync::Mutex;

    /// Cache TTL used by the session-cache test, with its expiry probe set
    /// just past it. Named constants because the TTL *is* the behaviour under
    /// test; the minimum the builder accepts is [`MIN_STATS_CACHE_TTL`].
    const TEST_STATS_CACHE_TTL: Duration = Duration::from_secs(1);
    /// How long the cache test waits to observe TTL expiry (TTL plus margin).
    const TEST_STATS_CACHE_EXPIRY_WAIT: Duration = Duration::from_millis(1100);

    /// The `[internal_api] stats_cache_ttl` bounds validated by
    /// `AppConfig` must stay within the range this adapter's builder accepts:
    /// a config value that passes validation must never panic at wiring time.
    #[test]
    fn config_stats_cache_bounds_stay_within_adapter_bounds() {
        use oidc_exchange_core::config::{MAX_STATS_CACHE_TTL_SECS, MIN_STATS_CACHE_TTL_SECS};

        assert!(
            Duration::from_secs(MIN_STATS_CACHE_TTL_SECS) >= MIN_STATS_CACHE_TTL,
            "the config minimum must satisfy the builder's own floor"
        );
        assert_eq!(
            Duration::from_secs(MAX_STATS_CACHE_TTL_SECS),
            MAX_STATS_CACHE_TTL,
            "the config maximum must equal the adapter's documented ceiling"
        );
        // The documented default sits inside the accepted window too.
        let default = Duration::from_secs(
            oidc_exchange_core::service::parse_duration_secs(
                oidc_exchange_core::config::DEFAULT_STATS_CACHE_TTL,
            )
            .expect("the documented default parses"),
        );
        assert!(default >= MIN_STATS_CACHE_TTL && default <= MAX_STATS_CACHE_TTL);
    }

    /// Records every request the SDK issues and every ConsumedCapacity the
    /// responses report, so tests can assert how much an admin read cost —
    /// the spec's capacity-based verification — instead of wall-clock time.
    #[derive(Debug, Default)]
    struct CapacityProbe {
        requests: std::sync::atomic::AtomicUsize,
        units: Mutex<Vec<f64>>,
    }

    impl CapacityProbe {
        fn reset(&self) {
            self.requests.store(0, std::sync::atomic::Ordering::Relaxed);
            self.units.lock().expect("capacity mutex").clear();
        }

        fn request_count(&self) -> usize {
            self.requests.load(std::sync::atomic::Ordering::Relaxed)
        }

        fn captured_units(&self) -> Vec<f64> {
            self.units.lock().expect("capacity mutex").clone()
        }
    }

    /// Shared handle registered as an SDK interceptor. The inner probe is
    /// interior-mutable, so the handle's view of traffic matches the
    /// interceptor's exactly.
    #[derive(Debug, Clone)]
    struct ProbeHandle(Arc<CapacityProbe>);

    impl ProbeHandle {
        fn reset(&self) {
            self.0.reset();
        }

        fn request_count(&self) -> usize {
            self.0.request_count()
        }

        fn captured_units(&self) -> Vec<f64> {
            self.0.captured_units()
        }
    }

    impl aws_sdk_dynamodb::config::Intercept for ProbeHandle {
        fn name(&self) -> &'static str {
            "capacity_probe"
        }

        fn read_before_transmit(
            &self,
            _context: &aws_smithy_runtime_api::client::interceptors::context::BeforeTransmitInterceptorContextRef<
                '_,
            >,
            _runtime_components: &aws_sdk_dynamodb::config::RuntimeComponents,
            _cfg: &mut aws_smithy_types::config_bag::ConfigBag,
        ) -> std::result::Result<(), aws_smithy_runtime_api::box_error::BoxError> {
            self.0
                .requests
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        fn read_after_deserialization(
            &self,
            context: &aws_smithy_runtime_api::client::interceptors::context::AfterDeserializationInterceptorContextRef<
                '_,
            >,
            _runtime_components: &aws_sdk_dynamodb::config::RuntimeComponents,
            _cfg: &mut aws_smithy_types::config_bag::ConfigBag,
        ) -> std::result::Result<(), aws_smithy_runtime_api::box_error::BoxError> {
            // The deserialized output carries the response's reported
            // ConsumedCapacity; downcast to the two admin-read operation
            // types this adapter issues.
            if let Ok(output) = context.output_or_error() {
                let units = output
                    .downcast_ref::<aws_sdk_dynamodb::operation::scan::ScanOutput>()
                    .and_then(|out| {
                        out.consumed_capacity
                            .as_ref()
                            .and_then(|c| c.capacity_units)
                    })
                    .or_else(|| {
                        output
                            .downcast_ref::<aws_sdk_dynamodb::operation::get_item::GetItemOutput>()
                            .and_then(|out| {
                                out.consumed_capacity
                                    .as_ref()
                                    .and_then(|c| c.capacity_units)
                            })
                    });
                if let Some(unit) = units {
                    self.0.units.lock().expect("capacity mutex").push(unit);
                }
            }
            Ok(())
        }
    }

    /// A client wired to DynamoDB Local through a [`ProbeHandle`], returned
    /// alongside the handle so tests can reset and read captured traffic.
    async fn create_probed_client() -> (aws_sdk_dynamodb::Client, ProbeHandle) {
        let probe = ProbeHandle(Arc::new(CapacityProbe::default()));
        let config = aws_sdk_dynamodb::config::Builder::new()
            .behavior_version(aws_sdk_dynamodb::config::BehaviorVersion::latest())
            .endpoint_url("http://localhost:8000")
            .region(aws_sdk_dynamodb::config::Region::new("us-east-1"))
            .credentials_provider(aws_sdk_dynamodb::config::Credentials::new(
                "fakeAccessKey",
                "fakeSecretKey",
                None,
                None,
                "test",
            ))
            .interceptor(probe.clone())
            .build();
        let client = aws_sdk_dynamodb::Client::from_conf(config);
        (client, probe)
    }

    /// Total read capacity of one unbounded scan over every `PROFILE` item —
    /// a live measurement of exactly the whole-table walk this task removed.
    async fn full_profile_scan_units(client: &aws_sdk_dynamodb::Client, table: &str) -> f64 {
        let mut total = 0.0;
        let mut exclusive_start_key: Option<HashMap<String, AttributeValue>> = None;
        loop {
            let mut scan = client
                .scan()
                .table_name(table)
                .filter_expression("sk = :sk")
                .expression_attribute_values(":sk", AttributeValue::S("PROFILE".to_string()))
                .return_consumed_capacity(ReturnConsumedCapacity::Total);
            if let Some(start_key) = exclusive_start_key.clone() {
                scan = scan.set_exclusive_start_key(Some(start_key));
            }
            let result = scan.send().await.expect("baseline full scan succeeds");
            if let Some(capacity) = result.consumed_capacity {
                total += capacity.capacity_units.unwrap_or(0.0);
            }
            match result.last_evaluated_key {
                Some(key) => exclusive_start_key = Some(key),
                None => return total,
            }
        }
    }

    async fn seed_users(repo: &DynamoRepository, count: usize, tag: &str) -> Vec<String> {
        let mut ids = Vec::with_capacity(count);
        for i in 0..count {
            let created = repo
                .create_user(&NewUser {
                    external_id: format!("google|{tag}_{i}"),
                    provider: "google".to_string(),
                    email: Some(format!("{tag}_{i}@example.com")),
                    display_name: None,
                })
                .await
                .expect("seed create_user");
            ids.push(created.id);
        }
        ids
    }

    /// Users seeded by the bounded-list test. Sized so the whole listing's
    /// capacity measurably exceeds one page under DynamoDB Local's
    /// size-based accounting (0.5 RCUs per started 4KB of returned items).
    const LIST_TEST_USERS: usize = 120;
    /// Page size requested by the bounded-list test.
    const LIST_TEST_PAGE_SIZE: i32 = 10;
    /// Users seeded by the stats test, sized so a hypothetical table walk
    /// would report at least twice a single-item read's capacity.
    const STATS_TEST_USERS: usize = 80;

    /// Consumed capacity of one scan exactly matching the adapter's first
    /// page — same `Limit`, same filter, same start — for an equality probe:
    /// if `list_users` ever issued more or larger reads than its declared
    /// single bounded scan, this replication would diverge from what the
    /// interceptor captured.
    async fn one_bounded_scan_units(client: &aws_sdk_dynamodb::Client, table: &str) -> f64 {
        client
            .scan()
            .table_name(table)
            .filter_expression("sk = :sk")
            .expression_attribute_values(":sk", AttributeValue::S("PROFILE".to_string()))
            .return_consumed_capacity(ReturnConsumedCapacity::Total)
            .limit(LIST_TEST_PAGE_SIZE)
            .send()
            .await
            .expect("replicated bounded scan succeeds")
            .consumed_capacity
            .and_then(|c| c.capacity_units)
            .unwrap_or(0.0)
    }

    /// The list page must be ONE bounded scan whose reported capacity matches
    /// a replicated page-shaped scan and sits strictly below walking every
    /// profile, and following cursors to exhaustion must be duplicate-free and
    /// skip-free regardless of DynamoDB Local's key order.
    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn list_users_executes_one_bounded_scan_per_page_measured_by_consumed_capacity() {
        let table_name = "oidc-exchange-test-capacity-list";
        let (client, probe) = create_probed_client().await;
        create_test_table(&client, table_name).await;
        let repo = DynamoRepository::new(client.clone(), table_name.to_string(), 60);

        let seeded = seed_users(&repo, LIST_TEST_USERS, "capacity_list").await;
        let full_scan_units = full_profile_scan_units(&client, table_name).await;
        assert!(
            full_scan_units >= 1.0,
            "the baseline walk over {seeded_len} profiles must cost measurable capacity",
            seeded_len = seeded.len()
        );
        let replicated_page_units = one_bounded_scan_units(&client, table_name).await;

        // Page one: exactly one request, capped by Limit, reporting capacity.
        probe.reset();
        let page_one = repo
            .list_users(None, LIST_TEST_PAGE_SIZE as u32)
            .await
            .expect("page one");
        assert_eq!(
            probe.request_count(),
            1,
            "a list page must be exactly one scan request"
        );
        assert!(
            page_one.users.len() <= LIST_TEST_PAGE_SIZE as usize,
            "the page honours the limit"
        );
        assert!(
            page_one.next_cursor.is_some(),
            "{LIST_TEST_USERS} users cannot fit inside one page"
        );
        let page_one_units: f64 = probe.captured_units().iter().sum();
        assert!(
            page_one_units > 0.0,
            "the scan must report its consumed read capacity"
        );
        assert!(
            (page_one_units - replicated_page_units).abs() < 0.01,
            "the adapter page cost {page_one_units} RCUs against \
             {replicated_page_units} for one replicated page-shaped scan"
        );
        assert!(
            page_one_units < full_scan_units / 2.0,
            "one page cost {page_one_units} RCUs against {full_scan_units} for the \
             unbounded walk — bounding by page size must bound capacity"
        );

        // Walk to exhaustion through short pages too: every continuation is
        // still exactly one request, and the union is complete and unique.
        let mut seen: Vec<String> = page_one.users.iter().map(|u| u.id.clone()).collect();
        let mut cursor = page_one.next_cursor;
        let mut pages = 1usize;
        while let Some(c) = cursor {
            probe.reset();
            let page = repo
                .list_users(Some(&c), LIST_TEST_PAGE_SIZE as u32)
                .await
                .expect("continuation page");
            assert_eq!(
                probe.request_count(),
                1,
                "each continuation page must also be one scan request"
            );
            seen.extend(page.users.iter().map(|u| u.id.clone()));
            cursor = page.next_cursor;
            pages += 1;
            assert!(pages <= 1000, "traversal must terminate at a null cursor");
        }

        assert_eq!(seen.len(), seeded.len(), "no duplicates across pages");
        let mut sorted_seen = seen.clone();
        sorted_seen.sort();
        let mut sorted_seeded = seeded.clone();
        sorted_seeded.sort();
        assert_eq!(
            sorted_seen, sorted_seeded,
            "no user may be skipped between adjacent pages"
        );

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    /// The stats path must cost one strongly-consistent GetItem — measured in
    /// consumed capacity, not wall clock — and the transactionally-maintained
    /// counters must track create/status/delete transitions exactly.
    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn stats_counter_costs_one_item_read_and_tracks_status_transitions() {
        let table_name = "oidc-exchange-test-capacity-stats";
        let (client, probe) = create_probed_client().await;
        create_test_table(&client, table_name).await;
        let repo = DynamoRepository::new(client.clone(), table_name.to_string(), 60);

        let seeded = seed_users(&repo, STATS_TEST_USERS, "capacity_stats").await;
        let full_scan_units = full_profile_scan_units(&client, table_name).await;

        probe.reset();
        let counts = repo.count_by_status().await.expect("count_by_status");
        assert_eq!(
            probe.request_count(),
            1,
            "stats must be one GetItem request, never a table walk"
        );
        assert_eq!(
            counts.get("active"),
            Some(&(STATS_TEST_USERS as u64)),
            "the counter reflects every seeded create"
        );
        let units = probe.captured_units();
        assert_eq!(
            units.len(),
            1,
            "one request must produce exactly one ConsumedCapacity entry"
        );
        assert!(
            (units[0] - 1.0).abs() < 0.05,
            "a consistent read of one sub-4KB item costs {} RCUs, expected ~1.0",
            units[0]
        );
        assert!(
            units[0] * 2.0 < full_scan_units,
            "counter read ({}) must stay below half a full-table walk \
             ({full_scan_units}) at this table size — a scan would cost more",
            units[0]
        );

        // Suspend: active drains, suspended fills — atomically with the write.
        repo.update_user(
            &seeded[0],
            &UserPatch {
                email: None,
                display_name: None,
                metadata: None,
                claims: None,
                status: Some(UserStatus::Suspended),
            },
        )
        .await
        .expect("suspend first user");
        let counts = repo.count_by_status().await.expect("counts after suspend");
        assert_eq!(counts.get("active"), Some(&(STATS_TEST_USERS as u64 - 1)));
        assert_eq!(counts.get("suspended"), Some(&1));

        // Delete an active user: the transition into Deleted carries the
        // counter adjustment in the same transaction as the status write.
        repo.delete_user(&seeded[1])
            .await
            .expect("delete second user");
        let counts = repo.count_by_status().await.expect("counts after delete");
        assert_eq!(counts.get("active"), Some(&(STATS_TEST_USERS as u64 - 2)));
        assert_eq!(counts.get("deleted"), Some(&1));
        assert_eq!(counts.get("suspended"), Some(&1));

        // A same-status patch adjusts nothing.
        repo.update_user(
            &seeded[2],
            &UserPatch {
                email: None,
                display_name: None,
                metadata: None,
                claims: None,
                status: Some(UserStatus::Active),
            },
        )
        .await
        .expect("no-op active patch on third user");
        let counts = repo
            .count_by_status()
            .await
            .expect("counts after no-op patch");
        assert_eq!(
            counts.get("active"),
            Some(&(STATS_TEST_USERS as u64 - 2)),
            "same-status patch moves nothing"
        );

        // Reactivating the suspended user restores the buckets symmetrically.
        repo.update_user(
            &seeded[0],
            &UserPatch {
                email: None,
                display_name: None,
                metadata: None,
                claims: None,
                status: Some(UserStatus::Active),
            },
        )
        .await
        .expect("reactivate first user");
        let counts = repo
            .count_by_status()
            .await
            .expect("counts after reactivate");
        assert_eq!(counts.get("active"), Some(&(STATS_TEST_USERS as u64 - 1)));
        assert_eq!(
            counts.get("suspended").copied().unwrap_or(0),
            0,
            "the suspended bucket must be empty again"
        );
        assert_eq!(counts.get("deleted"), Some(&1));

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    /// The active-session count obeys its named TTL: one walk populates the
    /// cache, calls inside the window are served without touching DynamoDB,
    /// and expiry forces exactly one refresh that sees newer sessions.
    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn active_session_cache_serves_within_ttl_and_refreshes_after_expiry() {
        let table_name = "oidc-exchange-test-capacity-session-cache";
        let (client, probe) = create_probed_client().await;
        create_test_table(&client, table_name).await;

        let repo = DynamoRepository::new(client.clone(), table_name.to_string(), 60)
            .with_stats_cache_ttl(TEST_STATS_CACHE_TTL);

        let user = repo
            .create_user(&NewUser {
                external_id: "google|session_cache_test".to_string(),
                provider: "google".to_string(),
                email: Some("session_cache@example.com".to_string()),
                display_name: None,
            })
            .await
            .expect("create_user");
        let session_for = |hash: String| Session {
            user_id: user.id.clone(),
            refresh_token_hash: oidc_exchange_core::Secret::new(hash),
            family_id: oidc_exchange_core::domain::new_family_id(),
            generation: 0,
            rotated_at: None,
            provider: "google".to_string(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: Utc::now(),
        };
        repo.store_refresh_token(&session_for("cache_hash_a".to_string()))
            .await
            .expect("store session a");
        repo.store_refresh_token(&session_for("cache_hash_b".to_string()))
            .await
            .expect("store session b");

        // Cold cache: the walk runs once.
        probe.reset();
        let count = repo.count_active_sessions().await.expect("first count");
        assert_eq!(count, 2, "both stored sessions are active");
        assert_eq!(
            probe.request_count(),
            1,
            "the cache-miss walk is a single paginated scan at this table size"
        );

        // Inside the TTL window the cached value serves and no request fires,
        // even though the underlying truth has changed.
        repo.store_refresh_token(&session_for("cache_hash_c".to_string()))
            .await
            .expect("store session c");
        probe.reset();
        let cached = repo.count_active_sessions().await.expect("cached count");
        assert_eq!(cached, 2, "within the TTL the cached count serves");
        assert_eq!(
            probe.request_count(),
            0,
            "no walk may run inside the named TTL window"
        );

        // After expiry the next call refreshes and sees the newer session.
        tokio::time::sleep(TEST_STATS_CACHE_EXPIRY_WAIT).await;
        probe.reset();
        let refreshed = repo.count_active_sessions().await.expect("refreshed count");
        assert_eq!(refreshed, 3, "expiry forces the walk to see session c");
        assert_eq!(
            probe.request_count(),
            1,
            "expiry triggers exactly one refresh walk"
        );

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    // -----------------------------------------------------------------------
    // Refresh-token rotation and reuse detection (task 06)
    // -----------------------------------------------------------------------

    use oidc_exchange_test_utils::session_contract;

    /// The roster-maintenance conditions require the fixture users the shared
    /// suite writes sessions for to exist first. Seeds their profile items
    /// directly (their ids are fixed by the harness, while `create_user`
    /// mints its own) — the same pattern `seed_fixture_users` uses on
    /// Postgres.
    async fn seed_fixture_users(client: &aws_sdk_dynamodb::Client, table: &str, user_ids: &[&str]) {
        for user_id in user_ids {
            let user = User {
                id: user_id.to_string(),
                external_id: format!("external|{user_id}"),
                provider: "conformance".to_string(),
                email: None,
                display_name: None,
                metadata: HashMap::new(),
                claims: HashMap::new(),
                status: UserStatus::Active,
                version: INITIAL_USER_VERSION,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            client
                .put_item()
                .table_name(table)
                .set_item(Some(user_to_item(&user)))
                .condition_expression("attribute_not_exists(pk)")
                .send()
                .await
                .expect("seed fixture user item");

            // The dedicated roster item `create_user` would have written (the
            // suite fixes the user ids, so the items are seeded directly).
            client
                .put_item()
                .table_name(table)
                .set_item(Some(HashMap::from([
                    (
                        "pk".to_string(),
                        AttributeValue::S(format!("USER#{user_id}")),
                    ),
                    ("sk".to_string(), DynamoRepository::user_sk()),
                    ("families".to_string(), AttributeValue::M(HashMap::new())),
                ])))
                .condition_expression("attribute_not_exists(pk)")
                .send()
                .await
                .expect("seed fixture roster item");
        }
    }

    /// Write a pre-rotation session item exactly as a pre-rotation deployment
    /// would have: no `family_id`, no `generation`, no `rotated_at`.
    async fn seed_legacy_session(
        client: &aws_sdk_dynamodb::Client,
        table: &str,
        token_hash: &str,
        user_id: &str,
    ) {
        seed_fixture_users(client, table, &[user_id]).await;
        let mut session = session_contract::generation_session(
            user_id,
            &session_contract::fixture_family_id("dynamo-legacy:placeholder"),
            0,
            token_hash.to_string(),
            Utc::now() + chrono::Duration::hours(24),
            Utc::now(),
            None,
        );
        session.family_id = String::new();
        let mut item = schema::session_to_item(&session);
        assert!(item.remove("family_id").is_some());
        assert!(item.remove("generation").is_some());
        assert!(item.remove("rotated_at").is_none(), "gen 0 has none");
        client
            .put_item()
            .table_name(table)
            .set_item(Some(item))
            .condition_expression("attribute_not_exists(pk)")
            .send()
            .await
            .expect("seed legacy session item");
    }

    async fn retired_item_count(client: &aws_sdk_dynamodb::Client, table: &str) -> u64 {
        let result = client
            .scan()
            .table_name(table)
            .filter_expression("#sk = :sk")
            .expression_attribute_names("#sk", "sk")
            .expression_attribute_values(":sk", AttributeValue::S(schema::RETIRED_SK.to_string()))
            .select(aws_sdk_dynamodb::types::Select::Count)
            .send()
            .await
            .expect("count retirement items");
        result.count().try_into().expect("non-negative count")
    }

    /// The full SR1–SR5 shared suite against the DynamoDB store. One tag
    /// namespaces every fixture the suite creates; each test gets its own
    /// table so concurrent ignored runs cannot collide.
    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn dynamo_session_store_meets_sr1_through_sr5() {
        let table_name = "oidc-exchange-test-session-conformance";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;
        seed_fixture_users(&client, table_name, &["usr_conformance", "usr_shared"]).await;

        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );
        session_contract::assert_full_conformance(&repo, "dynamo-session-conformance").await;

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    /// A legacy row's first redemption swaps atomically but writes no
    /// retirement record — there is no prior generation to detect reuse
    /// against — and the presented hash reads Unknown afterwards. The
    /// replacement carries the caller's newly-minted family; nothing here
    /// synthesizes one.
    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn legacy_row_first_redemption_swaps_without_retirement_record() {
        let table_name = "oidc-exchange-test-dynamo-legacy-first-redemption";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;
        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        let legacy_hash = session_contract::fixture_hash("dynamo-legacy:first-redemption");
        seed_legacy_session(&client, table_name, &legacy_hash, "usr_legacy").await;

        // Classification is storage-factual: the sentinel-carrying row is Live.
        let live = repo
            .resolve_refresh_token(&legacy_hash)
            .await
            .expect("resolve legacy row");
        match &live {
            RefreshResolution::Live(session) => {
                assert_eq!(session.family_id, "", "sentinel family on read");
                assert_eq!(session.generation, 0);
                assert_eq!(session.rotated_at, None);
            }
            other => panic!("the stored legacy row must resolve Live, got {other:?}"),
        }

        let base = Utc::now();
        let new_family = session_contract::fixture_family_id("dynamo-legacy:new-fam");
        assert!(is_valid_family_id(&new_family));
        if let RefreshResolution::Live(legacy) = &live {
            let replacement = Session {
                refresh_token_hash: Secret::new(format!("{legacy_hash}-next")),
                family_id: new_family.clone(),
                generation: 0,
                rotated_at: None,
                expires_at: base + chrono::Duration::hours(24),
                created_at: base,
                ..legacy.clone()
            };

            let won = repo
                .rotate_refresh_token(&legacy_hash, &replacement)
                .await
                .expect("legacy first-redemption swap");
            assert!(won, "an uncontended legacy redemption must win its CAS");

            assert_eq!(
                repo.resolve_refresh_token(&legacy_hash)
                    .await
                    .expect("resolve consumed hash"),
                RefreshResolution::Unknown,
                "a consumed legacy row has no retained record and must read Unknown"
            );
            match repo
                .resolve_refresh_token(replacement.refresh_token_hash.expose())
                .await
                .expect("resolve replacement")
            {
                RefreshResolution::Live(session) => {
                    assert_eq!(session.family_id, new_family);
                    assert_eq!(session.user_id, "usr_legacy");
                }
                other => panic!("replacement must be Live, got {other:?}"),
            }
            assert_eq!(
                retired_item_count(&client, table_name).await,
                0,
                "a legacy first redemption must not leave a retirement record"
            );
        } else {
            unreachable!("checked Live above");
        }

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    /// Negative space: a losing CAS against a legacy row writes nothing at all.
    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn legacy_row_failed_cas_leaves_store_untouched() {
        let table_name = "oidc-exchange-test-dynamo-legacy-failed-cas";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;
        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        let legacy_hash = session_contract::fixture_hash("dynamo-legacy:cas-failure");
        seed_legacy_session(&client, table_name, &legacy_hash, "usr_legacy").await;

        let base = Utc::now();
        let replacement = Session {
            refresh_token_hash: Secret::new(format!("{legacy_hash}-next")),
            family_id: session_contract::fixture_family_id("dynamo-legacy:cas-fam"),
            user_id: "usr_legacy".to_string(),
            provider: "google".to_string(),
            expires_at: base + chrono::Duration::hours(24),
            rotated_at: None,
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: base,
            generation: 0,
        };

        let won = repo
            .rotate_refresh_token("no-such-live-hash", &replacement)
            .await
            .expect("rotation against an unknown live hash");
        assert!(!won, "a missing live generation must lose the CAS");
        assert!(
            repo.get_session_by_refresh_token(&Secret::new(legacy_hash.clone()))
                .await
                .expect("read legacy row")
                .is_some(),
            "the legacy row must survive the lost race"
        );
        assert_eq!(
            retired_item_count(&client, table_name).await,
            0,
            "no retirement record may appear when the CAS fails"
        );
        assert!(
            repo.get_session_by_refresh_token(&replacement.refresh_token_hash)
                .await
                .expect("read replacement")
                .is_none(),
            "the loser's replacement must never be installed"
        );

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    /// Transaction rollback: when the replacement's condition fails
    /// mid-transaction (its hash colliding with an existing live row), the
    /// whole unit cancels — the live generation is untouched, no orphaned
    /// retirement record exists, and — per the DoD — only the *live*
    /// generation's cancellation maps to `false`; this one is a store error.
    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn failed_replacement_insert_rolls_back_delete_and_retirement() {
        let table_name = "oidc-exchange-test-dynamo-rollback";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;
        seed_fixture_users(&client, table_name, &["usr_rollback", "usr_blocker"]).await;
        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        let chain = session_contract::family_chain("dynamo:rollback", 0, "usr_rollback");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");

        // An unrelated live row already occupying the replacement's hash: the
        // mid-transaction collision that forces the rollback.
        let blocker = Session {
            refresh_token_hash: chain.gen1.refresh_token_hash.clone(),
            family_id: session_contract::fixture_family_id("dynamo:rollback:blocker-fam"),
            user_id: "usr_blocker".to_string(),
            ..chain.gen1.clone()
        };
        repo.store_refresh_token(&blocker)
            .await
            .expect("store blocker");

        let result = repo
            .rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
            .await;
        // The collision fails statement 1 (the replacement put), not
        // statement 0 (the live delete), so it must surface as an error —
        // never silently as a lost race.
        match &result {
            Err(Error::StoreError { detail }) => {
                assert!(!detail.is_empty(), "StoreError detail should explain it");
            }
            Ok(won) => panic!("the colliding insert must fail, got rotate={won}"),
            Err(other) => panic!("expected Error::StoreError, got {other:?}"),
        }

        // Rollback completeness: the live generation is still there with its
        // roster entry, no retirement record was written, the blocker is
        // untouched.
        assert_eq!(
            repo.get_session_by_refresh_token(&chain.gen0.refresh_token_hash)
                .await
                .expect("read gen0 after rollback"),
            Some(chain.gen0),
            "the cancelled transaction must have left the live generation intact"
        );
        assert_eq!(
            retired_item_count(&client, table_name).await,
            0,
            "no orphaned retirement record may survive the rollback"
        );
        assert!(
            repo.get_session_by_refresh_token(&blocker.refresh_token_hash)
                .await
                .expect("read blocker after rollback")
                .is_some(),
            "the blocking row must be untouched"
        );

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    /// `revoke_all_user_sessions` removes the user's live generations *and*
    /// their retained retirement records from the authoritative roster,
    /// leaving other users untouched.
    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn revoke_all_user_sessions_sweeps_retired_records_of_that_user_only() {
        let table_name = "oidc-exchange-test-dynamo-revoke-all";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;
        seed_fixture_users(&client, table_name, &["usr_mine", "usr_theirs"]).await;
        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        let mine = session_contract::family_chain("dynamo:revoke-all", 0, "usr_mine");
        let theirs = session_contract::family_chain("dynamo:revoke-all", 1, "usr_theirs");
        repo.store_refresh_token(&mine.gen0)
            .await
            .expect("store mine");
        repo.store_refresh_token(&theirs.gen0)
            .await
            .expect("store theirs");
        assert!(
            repo.rotate_refresh_token(mine.gen0.refresh_token_hash.expose(), &mine.gen1)
                .await
                .expect("rotate mine"),
            "my rotation must win"
        );
        assert!(
            repo.rotate_refresh_token(theirs.gen0.refresh_token_hash.expose(), &theirs.gen1)
                .await
                .expect("rotate theirs"),
            "their rotation must win"
        );
        assert_eq!(
            retired_item_count(&client, table_name).await,
            2,
            "one retirement record per rotation"
        );

        repo.revoke_all_user_sessions("usr_mine")
            .await
            .expect("revoke all mine");

        assert_eq!(
            repo.resolve_refresh_token(mine.gen1.refresh_token_hash.expose())
                .await
                .expect("resolve my live"),
            RefreshResolution::Unknown,
            "my live generation must be gone"
        );
        assert_eq!(
            repo.resolve_refresh_token(mine.gen0.refresh_token_hash.expose())
                .await
                .expect("resolve my retired"),
            RefreshResolution::Unknown,
            "my retirement record must be gone"
        );
        match repo
            .resolve_refresh_token(theirs.gen1.refresh_token_hash.expose())
            .await
            .expect("resolve their live")
        {
            RefreshResolution::Live(session) => assert_eq!(session.user_id, "usr_theirs"),
            other => panic!("another user's family must survive my revocation, got {other:?}"),
        }
        assert_eq!(
            retired_item_count(&client, table_name).await,
            1,
            "only the other user's retirement record remains"
        );

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    /// **GSI-staleness regression.** Revocation runs immediately after the
    /// writes it revokes — inside the window where GSI1 replication lag could
    /// hide them from an index query — and must still remove everything and
    /// report it honestly. This pins the sequencing that breaks a GSI-only
    /// implementation (`g3-dynamo-revoke-all-gsi-incompleteness`): owner
    /// discovery scans the base table under strong consistency and member
    /// enumeration reads the user-item roster, so index freshness is
    /// irrelevant to completeness. (DynamoDB Local cannot *manufacture* real
    /// replication lag; what it proves is that nothing in the implementation
    /// depends on waiting one out.)
    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn revoke_immediately_after_writes_is_complete_despite_gsi_lag_window() {
        let table_name = "oidc-exchange-test-dynamo-gsi-staleness";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;
        seed_fixture_users(&client, table_name, &["usr_stale_gsi"]).await;
        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        let chain = session_contract::family_chain("dynamo:stale-gsi", 0, "usr_stale_gsi");
        let sibling = session_contract::family_chain("dynamo:stale-gsi", 1, "usr_stale_gsi");

        // Store and rotate back-to-back, then revoke with zero delay: the
        // freshest possible state, i.e. maximal staleness for any index read.
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");
        assert!(
            repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
                .await
                .expect("rotate"),
            "the rotation must win"
        );
        repo.store_refresh_token(&sibling.gen0)
            .await
            .expect("store sibling");

        // Family revocation: one live generation + one retirement record.
        let removed = repo
            .revoke_family(&chain.family_id)
            .await
            .expect("revoke family immediately after writes");
        assert_eq!(
            removed, 2,
            "revocation must count exactly the roster-named entries despite the GSI window"
        );
        assert_eq!(
            repo.resolve_refresh_token(chain.gen1.refresh_token_hash.expose())
                .await
                .expect("resolve revoked live"),
            RefreshResolution::Unknown,
            "a just-written generation must not survive an immediate family revocation"
        );
        assert_eq!(
            repo.resolve_refresh_token(chain.gen0.refresh_token_hash.expose())
                .await
                .expect("resolve revoked retired"),
            RefreshResolution::Unknown,
            "a just-written retirement record must not survive either"
        );

        // All-user revocation over the sibling, same freshness regime.
        repo.revoke_all_user_sessions("usr_stale_gsi")
            .await
            .expect("revoke all immediately after write");
        assert_eq!(
            repo.resolve_refresh_token(sibling.gen0.refresh_token_hash.expose())
                .await
                .expect("resolve sibling after revoke-all"),
            RefreshResolution::Unknown,
            "an immediately-revoked fresh session must be removed by revoke_all too"
        );
        assert_eq!(
            repo.count_active_sessions().await.expect("active count"),
            0,
            "nothing survives the two immediate revocations"
        );

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    /// Every session mutation leaves the item and the authoritative roster
    /// mutually consistent: storing files both, rotating moves both, and a
    /// strongly consistent roster read reflects each step. Negative space:
    /// storing for a user whose profile item does not exist is refused (the
    /// roster arm's condition cancels the transaction) rather than writing an
    /// unfindable credential.
    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn session_mutations_leave_storage_and_roster_mutually_consistent() {
        let table_name = "oidc-exchange-test-dynamo-roster-consistency";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;
        seed_fixture_users(&client, table_name, &["usr_roster"]).await;
        let repo = DynamoRepository::new(
            client.clone(),
            table_name.to_string(),
            TEST_REUSE_RETENTION_SECS,
        );

        let chain = session_contract::family_chain("dynamo:roster", 0, "usr_roster");

        // Negative space first: no profile item exists for a phantom user.
        let phantom = session_contract::generation_session(
            "usr_phantom",
            &chain.family_id,
            0,
            format!("{}-phantom", chain.gen0.refresh_token_hash.expose()),
            chain.gen0.expires_at,
            chain.gen0.created_at,
            None,
        );
        let err = repo
            .store_refresh_token(&phantom)
            .await
            .expect_err("storing for a nonexistent user must fail");
        match err {
            Error::StoreError { .. } => {}
            other => panic!("expected Error::StoreError for phantom-user store, got {other:?}"),
        }
        assert!(
            repo.get_session_by_refresh_token(&phantom.refresh_token_hash)
                .await
                .expect("read phantom session")
                .is_none(),
            "the refused store must not have written the session item"
        );

        // Store gen0 → the roster names it as the family's only member/live.
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");
        let roster = repo
            .get_user_roster("usr_roster")
            .await
            .expect("read roster after store");
        assert_eq!(
            roster.sessions,
            vec![chain.gen0.refresh_token_hash.expose().clone()]
        );
        let family = roster
            .families
            .get(&chain.family_id)
            .expect("store must create the family entry");
        assert!(family.live == *chain.gen0.refresh_token_hash.expose());
        assert_eq!(
            family.members,
            vec![chain.gen0.refresh_token_hash.expose().clone()]
        );

        // Rotate → the roster swaps live to gen1 and remembers gen0.
        assert!(
            repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
                .await
                .expect("rotate"),
            "the rotation must win"
        );
        let roster = repo
            .get_user_roster("usr_roster")
            .await
            .expect("read roster after rotation");
        assert_eq!(
            roster.sessions,
            vec![chain.gen1.refresh_token_hash.expose().clone()]
        );
        let family = roster
            .families
            .get(&chain.family_id)
            .expect("rotation keeps the family entry");
        assert!(family.live == *chain.gen1.refresh_token_hash.expose());
        assert_eq!(family.members.len(), 2, "gen0 joins the remembered members");
        assert!(
            family
                .members
                .contains(chain.gen0.refresh_token_hash.expose()),
            "the retired generation must join the family's member set"
        );
        assert!(
            repo.revoke_family(&chain.family_id)
                .await
                .expect("final revoke")
                >= 2,
            "the family now holds one live plus one retained record to remove"
        );

        // After revocation the roster is clean again.
        let roster = repo
            .get_user_roster("usr_roster")
            .await
            .expect("read roster after revoke");
        assert!(
            !roster.families.contains_key(&chain.family_id),
            "revocation removes the family's roster entry"
        );

        // Clean up
        let _ = client.delete_table().table_name(table_name).send().await;
    }

    // -- Single-use conformance (shared suite in test-utils) --------------------
    // DynamoDB expires single-use records natively via the numeric `ttl` attribute, so
    // the cleanup-sweep scenario does not apply and is deliberately not invoked here.

    use oidc_exchange_test_utils::single_use_conformance as conformance;

    async fn create_single_use_test_repo() -> DynamoRepository {
        let table_name = "oidc-exchange-single-use-test";
        let client = create_test_client().await;
        create_test_table(&client, table_name).await;
        DynamoRepository::new(client, table_name.to_string(), 3600)
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn single_use_first_claim_wins_duplicate_loses() {
        let repo = create_single_use_test_repo().await;
        conformance::first_claim_wins_duplicate_loses(&repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn single_use_consume_live_record_exactly_once() {
        let repo = create_single_use_test_repo().await;
        conformance::consume_live_record_exactly_once(&repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn single_use_expired_record_is_absent_to_put_and_take() {
        let repo = create_single_use_test_repo().await;
        conformance::expired_record_is_absent_to_put_and_take(&repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn single_use_concurrent_put_has_exactly_one_winner() {
        let repo = std::sync::Arc::new(create_single_use_test_repo().await);
        conformance::concurrent_put_has_exactly_one_winner(repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
    async fn single_use_concurrent_take_has_exactly_one_winner() {
        let repo = std::sync::Arc::new(create_single_use_test_repo().await);
        conformance::concurrent_take_has_exactly_one_winner(repo).await;
    }
}
