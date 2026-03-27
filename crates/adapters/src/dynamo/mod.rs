pub mod schema;

use std::collections::HashMap;

use async_trait::async_trait;
use aws_sdk_dynamodb::types::AttributeValue;
use chrono::Utc;
use tracing::instrument;

use oidc_exchange_core::domain::{NewUser, Session, User, UserPatch, UserStatus};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::{SessionRepository, UserRepository};

use schema::{item_to_session, item_to_user, session_to_item, user_to_item};

const GSI1_NAME: &str = "GSI1";

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

    #[instrument(skip(self), fields(external_id))]
    async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<User>> {
        let result = self
            .client
            .query()
            .table_name(&self.table_name)
            .index_name(GSI1_NAME)
            .key_condition_expression("GSI1pk = :pk AND GSI1sk = :sk")
            .expression_attribute_values(
                ":pk",
                AttributeValue::S(format!("EXT#{external_id}")),
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
            created_at: now,
            updated_at: now,
        };

        let item = user_to_item(&full_user);

        self.client
            .put_item()
            .table_name(&self.table_name)
            .set_item(Some(item))
            .condition_expression("attribute_not_exists(pk)")
            .send()
            .await
            .map_err(Self::store_err)?;

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
        all_users.sort_by(|a, b| b.created_at.cmp(&a.created_at));

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
            .key(
                "pk",
                AttributeValue::S(format!("SESSION#{token_hash}")),
            )
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
            .key(
                "pk",
                AttributeValue::S(format!("SESSION#{token_hash}")),
            )
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
                .expression_attribute_values(
                    ":pk",
                    AttributeValue::S(format!("USER#{user_id}")),
                )
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
                            let pk = item.get("pk").cloned().unwrap_or_else(|| {
                                AttributeValue::S("UNKNOWN".to_string())
                            });
                            let sk = item.get("sk").cloned().unwrap_or_else(|| {
                                AttributeValue::S("UNKNOWN".to_string())
                            });

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

                    self.client
                        .batch_write_item()
                        .request_items(&self.table_name, delete_requests)
                        .send()
                        .await
                        .map_err(Self::store_err)?;
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
        let _ = client
            .delete_table()
            .table_name(table_name)
            .send()
            .await;

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

        // Get user by ID
        let fetched = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("user should exist");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.external_id, "google|user123");

        // Get user by external ID
        let fetched_ext = repo
            .get_user_by_external_id("google|user123")
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
                m.insert("key".to_string(), serde_json::Value::String("val".to_string()));
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
        repo.delete_user(&created.id)
            .await
            .expect("delete_user");
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
}
