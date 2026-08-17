use std::collections::HashMap;

use serde_json::Value;

use crate::domain::{
    AdminMutationKind, AuditOutcome, ClientAddr, NewUser, SecurityEvent, User, UserPatch,
    UserStatus,
};
use crate::error::{Error, Result};
use crate::service::AppService;

/// Key under which the claims-mutation operation name (`set_claims` /
/// `merge_claims` / `clear_claims`) is recorded in a `UserUpdated` audit
/// event's `detail` map.
const CLAIMS_OPERATION_DETAIL_KEY: &str = "operation";

impl AppService {
    /// Create a new user via admin API.
    ///
    /// Calls `repo.create_user()`, emits a blocking `UserCreated` audit event
    /// (admin operations carry no client `ip`/`user_agent` context, unlike
    /// the client-facing flows), then notifies user sync (non-blocking).
    pub async fn admin_create_user(&self, new_user: &NewUser) -> Result<User> {
        let user = self.user_repo.create_user(new_user).await?;

        self.emit_admin_mutation_audit_event(
            AdminMutationKind::Created,
            &user,
            AuditOutcome::Success,
        )
        .await?;

        if let Err(e) = self.user_sync.notify_user_created(&user).await {
            tracing::warn!(error = %e, user_id = %user.id, "user sync notify_user_created failed");
        }

        Ok(user)
    }

    /// Get a user by ID via admin API.
    pub async fn admin_get_user(&self, user_id: &str) -> Result<Option<User>> {
        self.user_repo.get_user_by_id(user_id).await
    }

    /// Fetch the user (missing id -> `Error::NotFound`), validate any status
    /// change against the lifecycle in [`UserStatus::can_transition_to`]
    /// (invalid transition -> `Error::InvalidRequest`), apply the patch via
    /// `repo.update_user()`, and revoke all the user's sessions when the
    /// status *changed* to `Suspended` or `Deleted` (a same-status patch,
    /// e.g. `Suspended -> Suspended`, does not re-revoke). Shared by
    /// [`Self::admin_update_user`] and [`Self::admin_delete_user`] so both
    /// enforce identical transition and revocation rules.
    async fn apply_validated_patch(&self, user_id: &str, patch: &UserPatch) -> Result<User> {
        let current = self
            .user_repo
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                detail: format!("user not found: {}", user_id),
            })?;

        if let Some(ref target) = patch.status {
            if !current.status.can_transition_to(target) {
                return Err(Error::InvalidRequest {
                    reason: format!(
                        "invalid status transition: {:?} -> {:?}",
                        current.status, target
                    ),
                });
            }
        }

        let user = self.user_repo.update_user(user_id, patch).await?;

        let entered_suspended_or_deleted = match &patch.status {
            Some(target) => {
                *target != current.status
                    && matches!(target, UserStatus::Suspended | UserStatus::Deleted)
            }
            None => false,
        };
        if entered_suspended_or_deleted {
            self.session_repo.revoke_all_user_sessions(user_id).await?;
        }

        Ok(user)
    }

    /// Update a user via admin API with a partial patch.
    ///
    /// See [`Self::apply_validated_patch`] for fetch/validate/revoke
    /// behaviour. On success, emits a blocking audit event — `UserSuspended`
    /// when the applied patch sets `status = Suspended`, `UserUpdated`
    /// otherwise — then notifies user sync with the list of changed fields
    /// (non-blocking).
    pub async fn admin_update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User> {
        let user = self.apply_validated_patch(user_id, patch).await?;

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

        let mutation_kind = if patch.status == Some(UserStatus::Suspended) {
            AdminMutationKind::Suspended
        } else {
            AdminMutationKind::Updated
        };
        self.emit_admin_mutation_audit_event(mutation_kind, &user, AuditOutcome::Success)
            .await?;

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
    /// Routes through [`Self::apply_validated_patch`] so the same lifecycle
    /// validation and session revocation apply: deleting a suspended user
    /// succeeds, a second delete on an already-`Deleted` user is rejected with
    /// `Error::InvalidRequest`, and an unknown id returns `Error::NotFound`.
    /// On success, emits a blocking `UserDeleted` audit event, then notifies
    /// user sync (non-blocking).
    pub async fn admin_delete_user(&self, user_id: &str) -> Result<()> {
        let patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
            status: Some(UserStatus::Deleted),
        };
        let user = self.apply_validated_patch(user_id, &patch).await?;

        self.emit_admin_mutation_audit_event(
            AdminMutationKind::Deleted,
            &user,
            AuditOutcome::Success,
        )
        .await?;

        if let Err(e) = self.user_sync.notify_user_deleted(user_id).await {
            tracing::warn!(error = %e, user_id = %user_id, "user sync notify_user_deleted failed");
        }

        Ok(())
    }

    /// Get custom claims for a user.
    ///
    /// Returns `Error::NotFound` if user not found.
    pub async fn admin_get_claims(&self, user_id: &str) -> Result<HashMap<String, Value>> {
        let user = self
            .user_repo
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                detail: format!("user not found: {}", user_id),
            })?;

        Ok(user.claims)
    }

    /// Replace all custom claims for a user.
    ///
    /// On success, emits a blocking `UserUpdated` audit event recording the
    /// `set_claims` operation in `detail`.
    pub async fn admin_set_claims(
        &self,
        user_id: &str,
        claims: HashMap<String, Value>,
    ) -> Result<()> {
        // Verify user exists
        let existing = self
            .user_repo
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                detail: format!("user not found: {}", user_id),
            })?;

        let patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: Some(claims),
            status: None,
        };
        self.user_repo.update_user(user_id, &patch).await?;

        self.emit_claims_audit_event(&existing, "set_claims")
            .await?;

        Ok(())
    }

    /// Merge new claims into existing user claims.
    ///
    /// New keys override existing keys; existing keys not in the patch are
    /// preserved. On success, emits a blocking `UserUpdated` audit event
    /// recording the `merge_claims` operation in `detail`.
    pub async fn admin_merge_claims(
        &self,
        user_id: &str,
        claims: HashMap<String, Value>,
    ) -> Result<()> {
        let user = self
            .user_repo
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                detail: format!("user not found: {}", user_id),
            })?;

        let mut merged = user.claims.clone();
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
        self.user_repo.update_user(user_id, &patch).await?;

        self.emit_claims_audit_event(&user, "merge_claims").await?;

        Ok(())
    }

    /// Clear all custom claims for a user (set to empty map).
    ///
    /// On success, emits a blocking `UserUpdated` audit event recording the
    /// `clear_claims` operation in `detail`.
    pub async fn admin_clear_claims(&self, user_id: &str) -> Result<()> {
        // Verify user exists
        let existing = self
            .user_repo
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                detail: format!("user not found: {}", user_id),
            })?;

        let patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: Some(HashMap::new()),
            status: None,
        };
        self.user_repo.update_user(user_id, &patch).await?;

        self.emit_claims_audit_event(&existing, "clear_claims")
            .await?;

        Ok(())
    }

    /// Emit the shared `UserUpdated` audit event for a claims mutation,
    /// recording `operation` in `detail` so `set_claims`/`merge_claims`/
    /// `clear_claims` are distinguishable in the audit trail. Admin
    /// operations carry no client `ip`/`user_agent` context.
    async fn emit_claims_audit_event(&self, user: &User, operation: &str) -> Result<()> {
        // Claims mutations add operation detail, so use the mandatory
        // security event emitter and retain the operation-specific detail.
        let mut event = SecurityEvent::AdminMutation {
            kind: AdminMutationKind::Updated,
        }
        .into_audit_event(
            AuditOutcome::Success,
            Some(user.id.clone()),
            Some(user.provider.clone()),
            ClientAddr::Unknown,
            None,
        );
        event.detail.insert(
            CLAIMS_OPERATION_DETAIL_KEY.to_string(),
            Value::String(operation.to_string()),
        );
        self.emit_admin_mutation_with_detail(event).await
    }

    /// Emits a detailed admin mutation through the mandatory durability path.
    async fn emit_admin_mutation_with_detail(
        &self,
        event: crate::domain::AuditEvent,
    ) -> Result<()> {
        match self.audit.emit(&event).await {
            Ok(()) => {
                crate::service::record_mandatory_audit_success();
                Ok(())
            }
            Err(error) => {
                crate::service::record_mandatory_audit_failure();
                self.log_audit_fallback(&event);
                if self.config.audit.durability.eq_ignore_ascii_case("enforce") {
                    Err(Error::SecurityAuditDurability {
                        detail: error.to_string(),
                    })
                } else {
                    tracing::error!(error = %error, audit_durability_degraded = true, "mandatory admin audit provider down");
                    Ok(())
                }
            }
        }
    }

    /// Emits an admin mutation through the mandatory durability path.
    async fn emit_admin_mutation_audit_event(
        &self,
        kind: AdminMutationKind,
        user: &User,
        outcome: AuditOutcome,
    ) -> Result<()> {
        self.emit_security_event(
            SecurityEvent::AdminMutation { kind },
            outcome,
            Some(user.id.clone()),
            Some(user.provider.clone()),
            ClientAddr::Unknown,
            None,
        )
        .await
    }

    /// Get aggregate stats for the dashboard.
    pub async fn admin_stats(&self) -> Result<AdminStats> {
        let user_counts = self.user_repo.count_by_status().await?;
        let active_sessions = self.session_repo.count_active_sessions().await?;

        let active = *user_counts.get("active").unwrap_or(&0);
        let suspended = *user_counts.get("suspended").unwrap_or(&0);
        let deleted = *user_counts.get("deleted").unwrap_or(&0);

        Ok(AdminStats {
            users: UserStats {
                total: active + suspended + deleted,
                active,
                suspended,
                deleted,
            },
            sessions: SessionStats {
                active: active_sessions,
            },
        })
    }

    /// List users with pagination.
    pub async fn admin_list_users(&self, offset: u64, limit: u64) -> Result<Vec<User>> {
        self.user_repo.list_users(offset, limit).await
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminStats {
    pub users: UserStats,
    pub sessions: SessionStats,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UserStats {
    pub total: u64,
    pub active: u64,
    pub suspended: u64,
    pub deleted: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionStats {
    pub active: u64,
}
