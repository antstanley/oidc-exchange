# Done Certificate — Task 03: enforce lifecycle in admin_update

**Task:** [03-enforce_lifecycle_in_admin_update.md](03-enforce_lifecycle_in_admin_update.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — `Suspended → Suspended` is a no-op with no revocation; `Deleted → Active`, `Deleted → Deleted`, and a second `DELETE` are `InvalidRequest`.**
  - *Claim:* a same-status `Suspended` patch does not call `revoke_all_user_sessions`; every transition out of `Deleted` and a second delete return `Error::InvalidRequest`.
  - *Evidence to collect:* run the negative-space core tests asserting (a) `Suspended → Suspended` records no revoke call on the mock session repo, (b) `Deleted → Active`, `Deleted → Deleted`, and a second `admin_delete_user` each return `Err(Error::InvalidRequest { .. })` — expect PASS.
  - *Checks:* resolve `can_transition_to` to `UserStatus` (from Task 02); confirm the revoke guard compares the fetched current status against the target rather than firing on any `Some(status)`.
  - *Status:* ☐ unverified

- **O3 — Unknown id on update/delete returns `NotFound`; touched functions carry ≥2 meaningful assertions.**
  - *Claim:* `admin_update_user` and `admin_delete_user` return `Error::NotFound` for an id not in the repo.
  - *Evidence to collect:* run core tests asserting `admin_update_user`/`admin_delete_user` on a missing id return `Err(Error::NotFound { .. })`; read the two functions and count meaningful assertions (≥2 each, across code and its tests).
  - *Checks:* resolve `Error::NotFound` to the variant from Task 01; confirm the fetch-first `get_user_by_id(...).ok_or_else(...)` precedes any write.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, any new bound named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: suspend-then-delete succeeds, `Suspended → Suspended` skips revocation, re-delete is `InvalidRequest`, unknown id is `NotFound`.**
  - *Claim:* a reviewer running the `user_admin` core suite sees all four behaviours.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core user_admin` (or the workspace suite) and read the assertions for suspend-then-delete (final status `Deleted`), `Suspended → Suspended` (no revoke), second delete (`InvalidRequest`), and unknown id (`NotFound`).
  - *Status:* ☐ unverified

## Regression check

- `admin_delete_user_revokes_sessions` (`crates/core/tests/user_admin.rs:281-334`) exercises the delete path → after routing delete through the validated path, expect it still ends with status `Deleted`, empty sessions, and a `Deleted` sync call : ☐ (PRESERVED / REGRESSION)
- `admin_update_user_partial_patch_reports_changed_fields` (`:119-161`) exercises a non-status patch → expect the changed-fields diff and `notify_user_updated` still fire unchanged with fetch-first added : ☐ (PRESERVED / REGRESSION)

## Residue

- The adapters' `update_user` still map an unknown id to `StoreError`; from the admin path this is now unreachable (fetch-first guards it) but stays as a backstop — not an obligation here.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
