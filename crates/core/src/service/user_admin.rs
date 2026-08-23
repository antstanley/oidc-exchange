use std::collections::HashMap;

use serde_json::Value;

use crate::domain::{
    clamp_admin_page_limit, AuditEvent, AuditEventType, AuditOutcome, AuditSeverity, NewUser,
    OperatorPrincipal, User, UserPage, UserPatch, UserStatus, DEFAULT_ADMIN_PAGE_SIZE,
    MAX_ADMIN_PAGE_SIZE,
};
use crate::error::{Error, Result};
use crate::service::{claims::find_reserved_claim_key, create_audit_event, AppService};

/// Key under which the claims-mutation operation name (`set_claims` /
/// `merge_claims` / `clear_claims`) is recorded in a `UserUpdated` audit
/// event's `detail` map.
const CLAIMS_OPERATION_DETAIL_KEY: &str = "operation";

/// Stamp the acting [`OperatorPrincipal`] onto an admin mutation's audit
/// event.
///
/// Attribution is applied by exactly this function and nowhere else, so it can
/// never happen silently: every admin mutation chooses to pass its principal
/// through here, and an event that somehow arrived pre-attributed is a
/// programmer error. The principal's invariants are re-checked at this
/// boundary (write-time validation of what the audit stream will record) —
/// in particular a shared-secret principal must be the reserved
/// `unattributed` shape, never a name.
fn attributed(mut event: AuditEvent, operator: &OperatorPrincipal) -> AuditEvent {
    operator.assert_invariants();
    assert!(
        event.operator.is_none(),
        "attribution must be applied exactly once, by attributed()"
    );
    event.operator = Some(operator.clone());
    assert_eq!(
        event.operator.as_ref(),
        Some(operator),
        "attribution must carry the acting principal verbatim"
    );
    event
}

/// Reject a caller-supplied claims map carrying a reserved protocol claim
/// name.
///
/// Enforced *before* persistence: a reserved name accepted here would live on
/// the user record and stay re-exportable through a `{{ user.claims.KEY }}`
/// template even though token build filters it — the write boundary is what
/// keeps the stored map and the signed token in agreement. The offending key
/// is named so the operator can fix the payload; the reason is a fixed,
/// non-secret string plus the claim name itself.
fn ensure_no_reserved_claims<V>(claims: &HashMap<String, V>) -> Result<()> {
    if let Some(key) = find_reserved_claim_key(claims) {
        return Err(Error::InvalidRequest {
            reason: format!(
                "claim name {key:?} is reserved by the token protocol and cannot be written"
            ),
        });
    }
    Ok(())
}

impl AppService {
    /// Create a new user via admin API.
    ///
    /// Calls `repo.create_user()`, emits a blocking `UserCreated` audit event
    /// attributed to `operator` (admin operations carry no client
    /// `ip`/`user_agent` context, unlike the client-facing flows), then
    /// notifies user sync (non-blocking).
    pub async fn admin_create_user(
        &self,
        operator: &OperatorPrincipal,
        new_user: &NewUser,
    ) -> Result<User> {
        let user = self.user_repo.create_user(new_user).await?;

        self.emit_audit(attributed(
            create_audit_event(
                AuditEventType::UserCreated,
                AuditSeverity::Notice,
                AuditOutcome::Success,
                Some(user.id.clone()),
                Some(user.provider.clone()),
                None,
                None,
            ),
            operator,
        ))
        .await?;

        if let Err(e) = self.user_sync.notify_user_created(&user).await {
            tracing::warn!(error = %e, user_id = %user.id, "user sync notify_user_created failed");
        }

        Ok(user)
    }

    /// Get a user by ID via admin API.
    ///
    /// Reads take no `OperatorPrincipal`: attribution exists only where an
    /// audit event is emitted, and the read paths emit none — there is no
    /// attribution surface for a principal to attach to.
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
        // Claims patches are write-path ingress like set/merge claims: reject a
        // reserved name before anything is fetched or persisted.
        if let Some(ref patch_claims) = patch.claims {
            ensure_no_reserved_claims(patch_claims)?;
        }

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
    pub async fn admin_update_user(
        &self,
        operator: &OperatorPrincipal,
        user_id: &str,
        patch: &UserPatch,
    ) -> Result<User> {
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

        let event_type = if patch.status == Some(UserStatus::Suspended) {
            AuditEventType::UserSuspended
        } else {
            AuditEventType::UserUpdated
        };
        self.emit_audit(attributed(
            create_audit_event(
                event_type,
                AuditSeverity::Notice,
                AuditOutcome::Success,
                Some(user.id.clone()),
                Some(user.provider.clone()),
                None,
                None,
            ),
            operator,
        ))
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
    pub async fn admin_delete_user(
        &self,
        operator: &OperatorPrincipal,
        user_id: &str,
    ) -> Result<()> {
        let patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
            status: Some(UserStatus::Deleted),
        };
        let user = self.apply_validated_patch(user_id, &patch).await?;

        self.emit_audit(attributed(
            create_audit_event(
                AuditEventType::UserDeleted,
                AuditSeverity::Notice,
                AuditOutcome::Success,
                Some(user.id.clone()),
                Some(user.provider.clone()),
                None,
                None,
            ),
            operator,
        ))
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
    /// Reserved claim names are rejected with `InvalidRequest` before the map
    /// replaces the stored one. On success, emits a blocking `UserUpdated`
    /// audit event recording the `set_claims` operation in `detail`.
    pub async fn admin_set_claims(
        &self,
        operator: &OperatorPrincipal,
        user_id: &str,
        claims: HashMap<String, Value>,
    ) -> Result<()> {
        // Validate before touching the store: an invalid payload must fail at
        // the boundary, deterministically, regardless of user existence.
        ensure_no_reserved_claims(&claims)?;

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

        self.emit_claims_audit_event(operator, &existing, "set_claims")
            .await?;

        Ok(())
    }

    /// Merge new claims into existing user claims.
    ///
    /// New keys override existing keys; existing keys not in the patch are
    /// preserved. Only the incoming delta is validated — a record written
    /// before the reserved-name rule existed can still receive merges, and its
    /// legacy names remain token build's defensive filter's problem, not every
    /// future merge's. On success, emits a blocking `UserUpdated` audit event
    /// recording the `merge_claims` operation in `detail`.
    pub async fn admin_merge_claims(
        &self,
        operator: &OperatorPrincipal,
        user_id: &str,
        claims: HashMap<String, Value>,
    ) -> Result<()> {
        // Validate the delta before the store roundtrip so a reserved key is
        // rejected without depending on whether the user exists.
        ensure_no_reserved_claims(&claims)?;

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

        self.emit_claims_audit_event(operator, &user, "merge_claims")
            .await?;

        Ok(())
    }

    /// Clear all custom claims for a user (set to empty map).
    ///
    /// On success, emits a blocking `UserUpdated` audit event recording the
    /// `clear_claims` operation in `detail`.
    pub async fn admin_clear_claims(
        &self,
        operator: &OperatorPrincipal,
        user_id: &str,
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
            claims: Some(HashMap::new()),
            status: None,
        };
        self.user_repo.update_user(user_id, &patch).await?;

        self.emit_claims_audit_event(operator, &existing, "clear_claims")
            .await?;

        Ok(())
    }

    /// Emit the shared `UserUpdated` audit event for a claims mutation,
    /// recording `operation` in `detail` so `set_claims`/`merge_claims`/
    /// `clear_claims` are distinguishable in the audit trail, and attributing
    /// the event to `operator`. Admin operations carry no client
    /// `ip`/`user_agent` context.
    async fn emit_claims_audit_event(
        &self,
        operator: &OperatorPrincipal,
        user: &User,
        operation: &str,
    ) -> Result<()> {
        let mut event = attributed(
            create_audit_event(
                AuditEventType::UserUpdated,
                AuditSeverity::Notice,
                AuditOutcome::Success,
                Some(user.id.clone()),
                Some(user.provider.clone()),
                None,
                None,
            ),
            operator,
        );
        event.detail.insert(
            CLAIMS_OPERATION_DETAIL_KEY.to_string(),
            Value::String(operation.to_string()),
        );
        self.emit_audit(event).await
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

    /// List users as one bounded, cursor-paginated page.
    ///
    /// The default and the clamp are applied *here*, in the core, not in an
    /// HTTP handler: `limit = None` means [`DEFAULT_ADMIN_PAGE_SIZE`], and any
    /// caller-supplied value above [`MAX_ADMIN_PAGE_SIZE`] is clamped down
    /// before the repository port is reached. Clamping in the core (rather
    /// than the handler) is what bounds every path to an adapter, including
    /// non-HTTP callers; a zero limit is rejected rather than clamped because
    /// the published schema documents `minimum: 1`.
    pub async fn admin_list_users(
        &self,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<UserPage> {
        let effective_limit = match limit {
            Some(requested) => clamp_admin_page_limit(requested)?,
            None => DEFAULT_ADMIN_PAGE_SIZE,
        };
        assert!(
            effective_limit >= 1,
            "the resolved page limit must never fall below the documented minimum"
        );
        assert!(
            effective_limit <= MAX_ADMIN_PAGE_SIZE,
            "the resolved page limit must never exceed MAX_ADMIN_PAGE_SIZE"
        );

        let page = self.user_repo.list_users(cursor, effective_limit).await?;

        // Postcondition on what the adapter returned: a page never exceeds the
        // bound that was sent downstream, and an exhausted listing carries no
        // cursor. Both would silently break the wire contract's completion
        // signal if an adapter ever violated them.
        assert!(
            page.users.len() <= effective_limit as usize,
            "adapter returned {} rows for a limit of {effective_limit}",
            page.users.len()
        );
        Ok(page)
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
