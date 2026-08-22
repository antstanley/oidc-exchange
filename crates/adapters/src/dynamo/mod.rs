pub mod schema;

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use async_trait::async_trait;
use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
use aws_sdk_dynamodb::types::{AttributeValue, Delete, Put, TransactWriteItem, WriteRequest};
use chrono::Utc;
use tracing::instrument;

use oidc_exchange_core::domain::{
    NewUser, Session, User, UserPatch, UserStatus, INITIAL_USER_VERSION,
};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::{SessionRepository, UserRepository};

use schema::{
    guard_pk, guard_to_item, item_single_use_expiry, item_to_session, item_to_user,
    session_to_item, single_use_pk, single_use_to_item, user_to_item, GUARD_SK,
};

/// DynamoDB cancellation-reason code reported for a failed `attribute_not_exists(pk)`
/// condition inside a `TransactWriteItems` call — the signal that a `create_user` lost a
/// uniqueness race, mapped to `Error::Conflict` rather than `Error::StoreError`.
const CONDITIONAL_CHECK_FAILED_CODE: &str = "ConditionalCheckFailed";

const GSI1_NAME: &str = "GSI1";

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

pub struct DynamoRepository {
    client: aws_sdk_dynamodb::Client,
    table_name: String,
}

impl DynamoRepository {
    pub fn new(client: aws_sdk_dynamodb::Client, table_name: String) -> Self {
        Self { client, table_name }
    }

    fn store_err(e: impl std::fmt::Display) -> Error {
        Error::StoreError {
            detail: e.to_string(),
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

        self.client
            .transact_write_items()
            .transact_items(TransactWriteItem::builder().put(user_put).build())
            .transact_items(TransactWriteItem::builder().put(guard_put).build())
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

            if is_delete_transition {
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

                let outcome = self
                    .client
                    .transact_write_items()
                    .transact_items(TransactWriteItem::builder().put(user_put).build())
                    .transact_items(TransactWriteItem::builder().delete(guard_delete).build())
                    .send()
                    .await;

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
                            "update_user (delete transition) version conflict, retrying"
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

    #[instrument(skip(self))]
    async fn count_by_status(&self) -> Result<HashMap<String, u64>> {
        let mut counts: HashMap<String, u64> = HashMap::new();
        let mut exclusive_start_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let mut scan = self
                .client
                .scan()
                .table_name(&self.table_name)
                .filter_expression("sk = :sk")
                .expression_attribute_values(":sk", AttributeValue::S("PROFILE".to_string()))
                .projection_expression("#s")
                .expression_attribute_names("#s", "status");

            if let Some(ref start_key) = exclusive_start_key {
                scan = scan.set_exclusive_start_key(Some(start_key.clone()));
            }

            let result = scan.send().await.map_err(Self::store_err)?;
            let items = result.items.unwrap_or_default();

            for item in &items {
                let status = item
                    .get("status")
                    .and_then(|v| v.as_s().ok())
                    .unwrap_or(&"unknown".to_string())
                    .clone();
                *counts.entry(status).or_insert(0) += 1;
            }

            match result.last_evaluated_key {
                Some(key) => exclusive_start_key = Some(key),
                None => break,
            }
        }

        Ok(counts)
    }

    #[instrument(skip(self))]
    async fn list_users(&self, offset: u64, limit: u64) -> Result<Vec<User>> {
        let mut all_users: Vec<User> = Vec::new();
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
                all_users.push(item_to_user(item)?);
            }

            match result.last_evaluated_key {
                Some(key) => exclusive_start_key = Some(key),
                None => break,
            }
        }

        // Sort by created_at descending
        all_users.sort_by_key(|b| std::cmp::Reverse(b.created_at));

        // Apply offset and limit
        let start = offset as usize;
        let end = std::cmp::min(start + limit as usize, all_users.len());
        if start >= all_users.len() {
            return Ok(Vec::new());
        }

        Ok(all_users[start..end].to_vec())
    }
}

#[async_trait]
impl SessionRepository for DynamoRepository {
    #[instrument(skip(self, session), fields(user_id = %session.user_id))]
    async fn store_refresh_token(&self, session: &Session) -> Result<()> {
        let item = session_to_item(session);

        self.client
            .put_item()
            .table_name(&self.table_name)
            .set_item(Some(item))
            .send()
            .await
            .map_err(Self::store_err)?;

        Ok(())
    }

    #[instrument(skip(self), fields(token_hash))]
    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>> {
        let result = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(format!("SESSION#{token_hash}")))
            .key("sk", AttributeValue::S("SESSION".to_string()))
            .send()
            .await
            .map_err(Self::store_err)?;

        match result.item {
            Some(item) => Ok(Some(item_to_session(&item)?)),
            None => Ok(None),
        }
    }

    #[instrument(skip(self), fields(token_hash))]
    async fn revoke_session(&self, token_hash: &str) -> Result<()> {
        self.client
            .delete_item()
            .table_name(&self.table_name)
            .key("pk", AttributeValue::S(format!("SESSION#{token_hash}")))
            .key("sk", AttributeValue::S("SESSION".to_string()))
            .send()
            .await
            .map_err(Self::store_err)?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn count_active_sessions(&self) -> Result<u64> {
        let now = Utc::now();
        let mut count: u64 = 0;
        let mut exclusive_start_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let mut scan = self
                .client
                .scan()
                .table_name(&self.table_name)
                .filter_expression("sk = :sk")
                .expression_attribute_values(":sk", AttributeValue::S("SESSION".to_string()));

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

    #[instrument(skip(self), fields(user_id))]
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()> {
        // Query GSI1 for all sessions belonging to this user
        let mut exclusive_start_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let mut query = self
                .client
                .query()
                .table_name(&self.table_name)
                .index_name(GSI1_NAME)
                .key_condition_expression("GSI1pk = :pk AND begins_with(GSI1sk, :sk_prefix)")
                .expression_attribute_values(":pk", AttributeValue::S(format!("USER#{user_id}")))
                .expression_attribute_values(
                    ":sk_prefix",
                    AttributeValue::S("SESSION#".to_string()),
                )
                // Only need the primary key attributes to delete
                .projection_expression("pk, sk");

            if let Some(ref start_key) = exclusive_start_key {
                query = query.set_exclusive_start_key(Some(start_key.clone()));
            }

            let result = query.send().await.map_err(Self::store_err)?;

            let items = result.items.unwrap_or_default();

            if !items.is_empty() {
                // BatchWriteItem supports up to 25 items per call
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

                    self.batch_write_with_retry(delete_requests).await?;
                }
            }

            match result.last_evaluated_key {
                Some(key) => exclusive_start_key = Some(key),
                None => break,
            }
        }

        Ok(())
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

        let repo = DynamoRepository::new(client.clone(), table_name.to_string());

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
            refresh_token_hash: "hash_abc123".to_string(),
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(24),
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
            .get_session_by_refresh_token("hash_abc123")
            .await
            .expect("get_session_by_refresh_token")
            .expect("session should exist");
        assert_eq!(fetched_session.user_id, created.id);
        assert_eq!(fetched_session.refresh_token_hash, "hash_abc123");
        assert_eq!(fetched_session.device_id.as_deref(), Some("device-1"));

        // Get non-existent session
        let none = repo
            .get_session_by_refresh_token("hash_nonexistent")
            .await
            .expect("get_session_by_refresh_token");
        assert!(none.is_none());

        // Store a second session for the same user
        let session2 = Session {
            user_id: created.id.clone(),
            refresh_token_hash: "hash_def456".to_string(),
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(24),
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: now,
        };
        repo.store_refresh_token(&session2)
            .await
            .expect("store second session");

        // Revoke single session
        repo.revoke_session("hash_abc123")
            .await
            .expect("revoke_session");
        let revoked = repo
            .get_session_by_refresh_token("hash_abc123")
            .await
            .expect("get after revoke");
        assert!(revoked.is_none());

        // The other session should still exist
        let still_exists = repo
            .get_session_by_refresh_token("hash_def456")
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
            .get_session_by_refresh_token("hash_abc123")
            .await
            .expect("get after revoke_all");
        let s2 = repo
            .get_session_by_refresh_token("hash_def456")
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

        let repo = DynamoRepository::new(client.clone(), table_name.to_string());

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

        let repo_a = DynamoRepository::new(client.clone(), table_name.to_string());
        let repo_b = DynamoRepository::new(client.clone(), table_name.to_string());
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
        let repo = DynamoRepository::new(client, "oidc-exchange-test-nonexistent".to_string());

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

        let repo = DynamoRepository::new(client.clone(), table_name.to_string());

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

        let repo = DynamoRepository::new(client.clone(), table_name.to_string());

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

        let repo = DynamoRepository::new(client.clone(), table_name.to_string());

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

        let repo = DynamoRepository::new(client.clone(), table_name.to_string());

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

        let repo_a = DynamoRepository::new(client.clone(), table_name.to_string());
        let repo_b = DynamoRepository::new(client.clone(), table_name.to_string());
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

        let repo = DynamoRepository::new(client.clone(), table_name.to_string());

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

        let repo = DynamoRepository::new(client.clone(), table_name.to_string());

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
        DynamoRepository::new(client, table_name.to_string())
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
