use std::collections::HashMap;

use serde_json::Value;

use crate::domain::{NewUser, User, UserPatch, UserStatus};
use crate::error::{Error, Result};
use crate::service::AppService;

impl AppService {
    /// Create a new user via admin API.
    ///
    /// Calls `repo.create_user()`, then notifies user sync (non-blocking).
    pub async fn admin_create_user(&self, new_user: &NewUser) -> Result<User> {
        let user = self.repo.create_user(new_user).await?;

        if let Err(e) = self.user_sync.notify_user_created(&user).await {
            tracing::warn!(error = %e, user_id = %user.id, "user sync notify_user_created failed");
        }

        Ok(user)
    }

    /// Get a user by ID via admin API.
    pub async fn admin_get_user(&self, user_id: &str) -> Result<Option<User>> {
        self.repo.get_user_by_id(user_id).await
    }

    /// Update a user via admin API with a partial patch.
    ///
    /// Calls `repo.update_user()`, then notifies user sync with the list of
    /// changed fields (non-blocking).
    pub async fn admin_update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User> {
        let user = self.repo.update_user(user_id, patch).await?;

        let mut changed_fields: Vec<&str> = Vec::new();
        if patch.email.is_some() {
            changed_fields.push("email");
        }
        if patch.display_name.is_some() {
            changed_fields.push("display_name");
        }
        if patch.metadata.is_some() {
            changed_fields.push("metadata");
        }
        if patch.claims.is_some() {
            changed_fields.push("claims");
        }
        if patch.status.is_some() {
            changed_fields.push("status");
        }

        if let Err(e) = self
            .user_sync
            .notify_user_updated(&user, &changed_fields)
            .await
        {
            tracing::warn!(error = %e, user_id = %user.id, "user sync notify_user_updated failed");
        }

        Ok(user)
    }

    /// Soft-delete a user via admin API.
    ///
    /// Sets user status to `Deleted`, revokes all sessions, and notifies user sync.
    pub async fn admin_delete_user(&self, user_id: &str) -> Result<()> {
        let patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
            status: Some(UserStatus::Deleted),
        };
        self.repo.update_user(user_id, &patch).await?;
        self.repo.revoke_all_user_sessions(user_id).await?;

        if let Err(e) = self.user_sync.notify_user_deleted(user_id).await {
            tracing::warn!(error = %e, user_id = %user_id, "user sync notify_user_deleted failed");
        }

        Ok(())
    }

    /// Get custom claims for a user.
    ///
    /// Returns `Error::InvalidRequest` if user not found.
    pub async fn admin_get_claims(&self, user_id: &str) -> Result<HashMap<String, Value>> {
        let user = self
            .repo
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| Error::InvalidRequest {
                reason: format!("user not found: {}", user_id),
            })?;

        Ok(user.claims)
    }

    /// Replace all custom claims for a user.
    pub async fn admin_set_claims(
        &self,
        user_id: &str,
        claims: HashMap<String, Value>,
    ) -> Result<()> {
        // Verify user exists
        self.repo
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| Error::InvalidRequest {
                reason: format!("user not found: {}", user_id),
            })?;

        let patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: Some(claims),
            status: None,
        };
        self.repo.update_user(user_id, &patch).await?;
        Ok(())
    }

    /// Merge new claims into existing user claims.
    ///
    /// New keys override existing keys; existing keys not in the patch are preserved.
    pub async fn admin_merge_claims(
        &self,
        user_id: &str,
        claims: HashMap<String, Value>,
    ) -> Result<()> {
        let user = self
            .repo
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| Error::InvalidRequest {
                reason: format!("user not found: {}", user_id),
            })?;

        let mut merged = user.claims;
        for (k, v) in claims {
            merged.insert(k, v);
        }

        let patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: Some(merged),
            status: None,
        };
        self.repo.update_user(user_id, &patch).await?;
        Ok(())
    }

    /// Clear all custom claims for a user (set to empty map).
    pub async fn admin_clear_claims(&self, user_id: &str) -> Result<()> {
        // Verify user exists
        self.repo
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| Error::InvalidRequest {
                reason: format!("user not found: {}", user_id),
            })?;

        let patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: Some(HashMap::new()),
            status: None,
        };
        self.repo.update_user(user_id, &patch).await?;
        Ok(())
    }
}
