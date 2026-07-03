# Done Certificate — Task 08: admin operations emission

**Task:** [08-admin_operations_emission.md](08-admin_operations_emission.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 08. A validating agent discharges it: collect
> each obligation's evidence, run its checks, set the Status, then derive the Conclusion by the rubric.
> Do not mark an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 08) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not by
assertion.

## Premises

- **P1 — Goal.** Admin mutations emit their named audit events and read-only operations emit none.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing admin returns or the best-effort user-sync
  notify (warn-and-continue) in `crates/core/src/service/user_admin.rs`.

## Obligations

- **O1 — Each mutation emits its named event; reads emit none.**
  - *Claim:* `admin_create_user` → `UserCreated`; `admin_update_user` → `UserUpdated` (and
    `UserSuspended` when the patch sets `status = Suspended`); `admin_delete_user` → `UserDeleted`;
    the three claims mutations → `UserUpdated` with the operation in `detail`; reads emit nothing.
  - *Evidence to collect:* read `user_admin.rs:14`, `:33`, `:74`, `:120`, `:152`, `:173`; confirm
    each mutation emits via `create_audit_event` (`None` ip/ua) + `emit_audit`, and that
    `admin_get_user`/`admin_list_users`/`admin_stats`/`admin_get_claims` have no `emit_audit` call.
  - *Checks:* resolve the `AuditEventType` variants to `crates/core/src/domain/audit.rs`; trace the
    `admin_update_user` suspend branch selects `UserSuspended` when `patch.status == Some(Suspended)`.
  - *Status:* SATISFIED — `user_admin.rs:25` (`UserCreated`), `:119-133` (`UserUpdated`/`UserSuspended`),
    `:164` (`UserDeleted`), and the shared `emit_claims_audit_event` at `:300-315` (`UserUpdated` +
    `detail["operation"]` = `set_claims`/`merge_claims`/`clear_claims` from `:224`/`:262`/`:290`) each
    build via `create_audit_event` with `None` ip/ua and call `emit_audit`. `admin_get_user:44`,
    `admin_list_users:340`, `admin_stats:318`, `admin_get_claims:185` carry no `emit_audit` call.
    `create_audit_event` resolves to the `use crate::service::{create_audit_event}` import →
    `service/mod.rs:146` (not a shadow); `emit_audit` is the `AppService` method at `mod.rs:102`.
    `AuditEventType::{UserCreated,UserUpdated,UserSuspended,UserDeleted}` all exist at
    `domain/audit.rs:45-48`. The suspend branch keys on `patch.status == Some(UserStatus::Suspended)`
    (`user_admin.rs:119`), reached only after `apply_validated_patch` accepts the transition.

- **O2 — Admin audit follows blocking rules.**
  - *Claim:* an admin audit failure under the blocking threshold propagates as `Err`, unlike the
    best-effort user-sync notify that only logs a warning.
  - *Evidence to collect:* trace one mutation's emission — confirm it propagates `emit_audit`'s
    `Result` (via `?`), distinct from the `if let Err(e) = self.user_sync…` warn-only pattern at
    `user_admin.rs:16-18`.
  - *Status:* SATISFIED — every mutation propagates the emission via `.await?` (`user_admin.rs:34`,
    `:133`, `:173`, and `emit_claims_audit_event` at `:314`), so a blocking-threshold failure returns
    `Err`. `emit_audit` (`mod.rs:112-136`) returns `Err(e)` when `event.severity <= blocking_threshold`.
    This is distinct from the `if let Err(e) = self.user_sync.notify_… { tracing::warn!(…) }`
    warn-and-continue at `user_admin.rs:36-38`/`:135-141`/`:175-177`. Exercised by
    `admin_create_user_blocking_audit_failure_propagates_err_and_skips_sync` (PASS): with
    `blocking_threshold="info"` and a failing `MockAuditLog`, the call returns `Error::AuditError`
    and the best-effort sync notify never fires.

- **O3 — Negative-space test: reads record nothing.**
  - *Claim:* `admin_get_user`/`admin_list_users`/`admin_stats`/`admin_get_claims` each record zero
    events on `MockAuditLog`.
  - *Evidence to collect:* run the new admin emission tests — expect PASS asserting zero events for
    each read and the expected event for each mutation (including the suspend-path `UserSuspended`).
  - *Status:* SATISFIED — `admin_reads_emit_no_audit_events` (PASS) pre-creates a user directly through
    the repo, then calls `admin_get_user`/`admin_list_users`/`admin_stats`/`admin_get_claims` and
    asserts `audit.events()` is empty. The mutation-side tests (`admin_create_user_emits_user_created…`,
    `admin_update_user_non_status_patch_emits_user_updated`,
    `admin_update_user_suspend_patch_emits_user_suspended_not_user_updated`,
    `admin_delete_user_emits_user_deleted…`,
    `admin_claims_mutations_emit_user_updated_with_operation_in_detail`) all PASS.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* SATISFIED — `cargo fmt --check` clean (exit 0); `cargo clippy --workspace -- -D warnings`
    clean (no warnings); `cargo nextest run --workspace` → 337 passed, 0 failed (27 skipped). The
    claims `detail` key is a named constant (`CLAIMS_OPERATION_DETAIL_KEY`, `user_admin.rs:14`).

- **O5 — Reviewable: mutations emit, reads do not.**
  - *Claim:* each mutation's event (and the suspend-path `UserSuspended`) is recorded while reads
    record nothing.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core user_admin` — expect PASS;
    inspect `MockAuditLog` events per operation.
  - *Status:* SATISFIED — the certificate's literal `-p oidc-exchange-core user_admin` filter matches 0
    tests (the substring `user_admin` is not in any test name; the tests live in a binary of that name),
    so the binary was run instead: `cargo nextest run -p oidc-exchange-core -E 'binary(user_admin)'`
    → 27 passed, 0 failed. The audit tests inspect `MockAuditLog` events per operation and confirm each
    mutation's event (including the suspend-path `UserSuspended` and the claims-op `detail`) while the
    four reads record nothing.

## Regression check

- The best-effort user-sync notify calls (`notify_user_created`/`updated`/`deleted`) → trace that
  adding a blocking `emit_audit` does not change their warn-and-continue behaviour → expect the
  sync path unchanged : PRESERVED — the `notify_user_created/updated/deleted` calls keep their
  `if let Err(e) = … { tracing::warn!(…) }` warn-and-continue shape (`user_admin.rs:36-38`,
  `:135-141`, `:175-177`); the added blocking `emit_audit` sits before them and does not alter their
  failure handling. `admin_create_user_triggers_sync` / `…_partial_patch_reports_changed_fields` /
  `admin_delete_user_revokes_sessions` still PASS.
- `admin_delete_user` revokes all sessions before returning → trace that emission is added after the
  revoke, preserving the delete semantics → expect unchanged : PRESERVED — `apply_validated_patch`
  revokes all sessions (`user_admin.rs:86`) before `admin_delete_user` emits `UserDeleted`
  (`:164`); the emission is strictly after the revoke and does not touch the delete path.
  `admin_delete_user_revokes_sessions` and `suspend_then_delete_succeeds_and_leaves_user_deleted`
  still PASS.

## Residue

- Whether claims mutations should each carry a distinct `detail` operation name (`set`/`merge`/
  `clear`) is specified as "operation in `detail`"; the exact key is an implementation detail, not
  a separate obligation.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence — each admin mutation emits its named event via
`create_audit_event`(None ip/ua)+`emit_audit` (suspend→`UserSuspended`, claims→`UserUpdated` with the
op in `detail`) while the four reads emit nothing, admin audit follows `emit_audit`'s blocking rules
(distinct from the warn-only sync notify), and `fmt`/`clippy -D warnings`/`nextest --workspace`
(337 passed) plus the 27-test `user_admin` binary are all green; the two named regression callers
(warn-and-continue sync, delete-then-revoke ordering) are PRESERVED. Note: the certificate's literal
Reviewable command `-p oidc-exchange-core user_admin` matches 0 tests (substring filter, not the
binary name) — validated via `-E 'binary(user_admin)'` instead.
