# Done Certificate — Task 03: enforce lifecycle in admin_update

**Task:** [03-enforce_lifecycle_in_admin_update.md](03-enforce_lifecycle_in_admin_update.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** `admin_update_user` fetches-first, validates the status transition, applies the patch, and revokes all sessions only when the status changed to `Suspended` or `Deleted`; `admin_delete_user` is routed through the same validated path.
- **P2 — Obligations.** Done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing `admin_update_user` changed-fields diff / `notify_user_updated` ordering (`crates/core/src/service/user_admin.rs:32-61`), the existing `admin_delete_user_revokes_sessions` behaviour, or the `admin_update_user_partial_patch_reports_changed_fields` test.

## Obligations

- **O1 — Entering `Suspended` or `Deleted` revokes all sessions; a reactivated user has no surviving refresh sessions.**
  - *Claim:* a status patch (or `admin_delete_user`) to `Suspended`/`Deleted` calls `session_repo.revoke_all_user_sessions`; after suspend-then-reactivate the user has no sessions.
  - *Evidence to collect:* read `admin_update_user` in `crates/core/src/service/user_admin.rs`; run the new core tests in `crates/core/tests/user_admin.rs` for patch-to-`Suspended`/`Deleted` session revocation and the reactivation-has-no-sessions case (assert `get_all_sessions().is_empty()`) — expect PASS.
  - *Checks:* resolve `revoke_all_user_sessions` to `self.session_repo` (the `SessionRepository` port), not `user_repo`; confirm the revoke call is ordered after `update_user`, matching `admin_delete_user`.
  - *Evidence collected:* `apply_validated_patch` (`crates/core/src/service/user_admin.rs:36-70`) fetches first, validates, calls `update_user` (line 56), then `self.session_repo.revoke_all_user_sessions(user_id)` (line 66) when the status changed into `Suspended`/`Deleted`; `admin_update_user` (line 78) and `admin_delete_user` (line 122) both route through it. Tests `patch_to_suspended_revokes_sessions`, `patch_to_deleted_revokes_sessions`, `reactivated_user_has_no_surviving_sessions` (asserting `get_all_sessions().is_empty()` after reactivation) all PASS. Check: `revoke_all_user_sessions` resolves to `session_repo: Box<dyn SessionRepository>` (`ports/repository.rs:28`), not `user_repo`, and is ordered after `update_user`. No shadowing.
  - *Status:* ✅ SATISFIED

- **O2 — `Suspended → Suspended` is a no-op with no revocation; `Deleted → Active`, `Deleted → Deleted`, and a second `DELETE` are `InvalidRequest`.**
  - *Claim:* a same-status `Suspended` patch does not call `revoke_all_user_sessions`; every transition out of `Deleted` and a second delete return `Error::InvalidRequest`.
  - *Evidence to collect:* run the negative-space core tests asserting (a) `Suspended → Suspended` records no revoke call on the mock session repo, (b) `Deleted → Active`, `Deleted → Deleted`, and a second `admin_delete_user` each return `Err(Error::InvalidRequest { .. })` — expect PASS.
  - *Checks:* resolve `can_transition_to` to `UserStatus` (from Task 02); confirm the revoke guard compares the fetched current status against the target rather than firing on any `Some(status)`.
  - *Evidence collected:* tests `suspended_to_suspended_is_a_noop_and_does_not_re_revoke` (plants a sentinel session after suspension and asserts it survives the no-op patch), `deleted_to_active_is_rejected`, `deleted_to_deleted_is_rejected`, `second_delete_on_already_deleted_user_is_rejected` (also asserts the repo version did not advance) all PASS with `Err(Error::InvalidRequest { .. })`. Checks: `current.status.can_transition_to(target)` resolves to `UserStatus::can_transition_to` (`crates/core/src/domain/user.rs:52`, Task 02); the revoke guard (`user_admin.rs:58-64`) requires `*target != current.status && matches!(target, Suspended | Deleted)` — it does not fire on any `Some(status)`.
  - *Status:* ✅ SATISFIED

- **O3 — Unknown id on update/delete returns `NotFound`; touched functions carry ≥2 meaningful assertions.**
  - *Claim:* `admin_update_user` and `admin_delete_user` return `Error::NotFound` for an id not in the repo.
  - *Evidence to collect:* run core tests asserting `admin_update_user`/`admin_delete_user` on a missing id return `Err(Error::NotFound { .. })`; read the two functions and count meaningful assertions (≥2 each, across code and its tests).
  - *Checks:* resolve `Error::NotFound` to the variant from Task 01; confirm the fetch-first `get_user_by_id(...).ok_or_else(...)` precedes any write.
  - *Evidence collected:* tests `admin_update_user_unknown_id_returns_not_found` (both a status patch and a non-status patch) and `admin_delete_user_unknown_id_returns_not_found` (also asserts no user row written and no sync call fired) PASS with `Err(Error::NotFound { .. })`. Checks: `Error::NotFound { detail }` resolves to the Task 01 variant via `use crate::error::{Error, Result}` (`user_admin.rs:6`); the fetch-first `get_user_by_id(...).ok_or_else(...)` (`user_admin.rs:37-43`) precedes the `update_user` write (line 56). Assertions: the two unknown-id tests carry 5 assertions between them, and every lifecycle test carries ≥2 (status + sessions/version) — ≥2 meaningful assertions per touched function.
  - *Status:* ✅ SATISFIED

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, any new bound named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect clean.
  - *Evidence collected:* `cargo fmt --check` — clean (exit 0). `cargo clippy --workspace -- -D warnings` — clean. `cargo nextest run --workspace` — 297 tests run: 297 passed, 27 skipped. No new numeric bounds introduced; the change adds only control flow and tests.
  - *Status:* ✅ SATISFIED

- **O5 — Reviewable: suspend-then-delete succeeds, `Suspended → Suspended` skips revocation, re-delete is `InvalidRequest`, unknown id is `NotFound`.**
  - *Claim:* a reviewer running the `user_admin` core suite sees all four behaviours.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core user_admin` (or the workspace suite) and read the assertions for suspend-then-delete (final status `Deleted`), `Suspended → Suspended` (no revoke), second delete (`InvalidRequest`), and unknown id (`NotFound`).
  - *Evidence collected:* ran `cargo nextest run -p oidc-exchange-core -E 'binary(user_admin)'` — 16 tests run: 16 passed. Read the assertions: `suspend_then_delete_succeeds_and_leaves_user_deleted` asserts final status `Deleted` and empty sessions; `suspended_to_suspended_is_a_noop_and_does_not_re_revoke` asserts the sentinel session survives; `second_delete_on_already_deleted_user_is_rejected` asserts `InvalidRequest` and an unchanged version; `admin_update_user_unknown_id_returns_not_found` / `admin_delete_user_unknown_id_returns_not_found` assert `NotFound`. All four reviewable behaviours exercised and observed.
  - *Status:* ✅ SATISFIED

## Regression check

- `admin_delete_user_revokes_sessions` (`crates/core/tests/user_admin.rs:281-334`) exercises the delete path → after routing delete through the validated path, expect it still ends with status `Deleted`, empty sessions, and a `Deleted` sync call : ✅ PRESERVED — test PASSes unmodified; `admin_delete_user` still revokes (via the guard, since `Active → Deleted` is a change) and still fires `notify_user_deleted` after the validated patch.
- `admin_update_user_partial_patch_reports_changed_fields` (`:119-161`) exercises a non-status patch → expect the changed-fields diff and `notify_user_updated` still fire unchanged with fetch-first added : ✅ PRESERVED — test PASSes unmodified; the changed-fields diff and `notify_user_updated` ordering in `admin_update_user` (`user_admin.rs:80-103`) are untouched, only the write is now routed through `apply_validated_patch`.

## Residue

- The adapters' `update_user` still map an unknown id to `StoreError`; from the admin path this is now unreachable (fetch-first guards it) but stays as a backstop — not an obligation here.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with collected evidence — fetch-first + `can_transition_to` validation + change-guarded revocation implemented in `apply_validated_patch` and shared by both admin paths, all 16 `user_admin` tests and the full 297-test workspace suite pass with fmt/clippy clean, and both named regression tests are PRESERVED.
