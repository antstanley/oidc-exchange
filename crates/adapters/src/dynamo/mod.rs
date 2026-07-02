pub mod schema;

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use async_trait::async_trait;
use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
use aws_sdk_dynamodb::types::{AttributeValue, Put, TransactWriteItem, WriteRequest};
use chrono::Utc;
use tracing::instrument;

use oidc_exchange_core::domain::{
    NewUser, Session, User, UserPatch, UserStatus, INITIAL_USER_VERSION,
};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::{SessionRepository, UserRepository};

use schema::{guard_to_item, item_to_session, item_to_user, session_to_item, user_to_item};

/// DynamoDB cancellation-reason code reported for a failed `attribute_not_exists(pk)`
/// condition inside a `TransactWriteItems` call — the signal that a `create_user` lost a
/// uniqueness race, mapped to `Error::Conflict` rather than `Error::StoreError`.
const CONDITIONAL_CHECK_FAILED_CODE: &str = "ConditionalCheckFailed";

const GSI1_NAME: &str = "GSI1";

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
    /// unguarded — before `get_user_by_external_id` is switched to resolve through the
    /// guard item instead of the GSI1 query; a guard-less pre-existing user would
    /// otherwise become invisible to that lookup.
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

    #[instrument(skip(self), fields(external_id, provider))]
    async fn get_user_by_external_id(
        &self,
        external_id: &str,
        provider: &str,
    ) -> Result<Option<User>> {
        let result = self
            .client
            .query()
            .table_name(&self.table_name)
            .index_name(GSI1_NAME)
            .key_condition_expression("GSI1pk = :pk AND GSI1sk = :sk")
            .expression_attribute_values(
                ":pk",
                AttributeValue::S(format!("EXT#{provider}#{external_id}")),
            )
            .expression_attribute_values(":sk", AttributeValue::S("USER".to_string()))
            .limit(1)
            .send()
            .await
            .map_err(Self::store_err)?;

        match result.items {
            Some(items) if !items.is_empty() => Ok(Some(item_to_user(&items[0])?)),
            _ => Ok(None),
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
        // Get-modify-put pattern for v1 simplicity
        let mut user = self
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| Error::StoreError {
                detail: format!("user not found: {user_id}"),
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

        let item = user_to_item(&user);

        self.client
            .put_item()
            .table_name(&self.table_name)
            .set_item(Some(item))
            .send()
            .await
            .map_err(Self::store_err)?;

        Ok(user)
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
}
