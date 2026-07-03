# Done Certificate — Task 03: Exchange flow conflict → re-lookup

**Task:** [03-exchange_re_lookup.md](03-exchange_re_lookup.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** A first login racing a concurrent first login returns a token, not a 500: on `create_user` → `Conflict`, the exchange re-runs `get_user_by_external_id` and continues on the found-user branch, re-applying the suspended check.
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not change the found-user, suspended, or registration-policy behaviour for the non-racing path in `exchange.rs`; must not emit a second `create_user` or a duplicate `UserCreated` on the losing racer.

## Obligations

- **O1 — Concurrent first logins both return a token, none a 500.**
  - *Claim:* two `exchange` calls for one subject, where the second `create_user` conflicts, both return `Ok(TokenResponse)`.
  - *Evidence to collect:* run the new core test in `crates/core/tests/exchange.rs` that drives the race against a shared mock — expect both `Ok`, no `StoreError`/500, and exactly one user in the mock afterward.
  - *Checks:* resolve the re-lookup call to `self.user_repo.get_user_by_external_id` at `exchange.rs:85-89`, and confirm the `Conflict` match arm is on `Error::Conflict`, not a catch-all.
  - *Status:* SATISFIED — ran `exchange_conflict_on_create_re_lookups_and_returns_token` (crates/core/tests/exchange.rs:746) → PASS: both racers return `Ok(TokenResponse)`, `repo.get_all_users().len() == 1`, and both access tokens' `sub` decode to the single user's id. Check: the match arm at `exchange.rs:139` is `Err(Error::Conflict { .. })` (non-Conflict handled separately at :179); the re-lookup at `exchange.rs:145-148` resolves to `self.user_repo.get_user_by_external_id` — the same `UserRepository` port method as the initial lookup at `exchange.rs:85-88`, no shadowing.

- **O2 — The re-lookup path re-applies the suspended check.**
  - *Claim:* when the winning user is `Suspended`, the losing racer's re-lookup returns `UserSuspended`, not a token.
  - *Evidence to collect:* run the test where the pre-existing user found on re-lookup is `Suspended` — expect `Err(Error::UserSuspended { .. })`.
  - *Status:* SATISFIED — ran `exchange_conflict_re_lookup_reapplies_suspended_check` (crates/core/tests/exchange.rs:804) → PASS: the losing racer gets `Err(Error::UserSuspended { user_id })` matching the winner's id, no second user, no extra session. The re-applied check is at `exchange.rs:159-161` (`user.status != UserStatus::Active → UserSuspended`).

- **O3 — Negative-space: a non-Conflict create error propagates.**
  - *Claim:* a `StoreError` from `create_user` is returned as an error, not swallowed into a re-lookup.
  - *Evidence to collect:* run the test injecting a non-`Conflict` `create_user` failure — expect the error propagates (no re-lookup, no token).
  - *Checks:* confirm the match arm distinguishes `Error::Conflict` from other `Err` values (only `Conflict` triggers re-lookup).
  - *Status:* SATISFIED — ran `exchange_non_conflict_create_error_propagates_without_relookup` (crates/core/tests/exchange.rs:869) → PASS: an injected `StoreError` from `create_user` propagates as `Err(Error::StoreError { .. })`, no user/session created, and the instrumented lookup counter reads exactly 1 (the initial miss — no re-lookup occurred). Check: `exchange.rs:137-180` matches `Ok(created)` / `Err(Error::Conflict { .. })` / `Err(other) => return Err(other)` — only `Conflict` triggers the re-lookup.

- **O4 — MockRepository enforces non-deleted uniqueness and excludes deleted from lookup.**
  - *Claim:* `MockRepository::create_user` returns `Conflict` on a duplicate live `(provider, external_id)`, and `get_user_by_external_id` skips `Deleted` users.
  - *Evidence to collect:* read `crates/test-utils/src/lib.rs` `create_user` and `get_user_by_external_id`; run a mock unit test asserting the duplicate → `Conflict` and a deleted user → `None`.
  - *Status:* SATISFIED — read `crates/test-utils/src/lib.rs:82-101` (`create_user` rejects a live duplicate `(provider, external_id)` — `status != UserStatus::Deleted` — with `Error::Conflict`) and `:71-80` (`get_user_by_external_id` filters `u.status != UserStatus::Deleted`). Ran `tests::create_user_rejects_duplicate_live_external_id` and `tests::deleted_user_frees_external_id_for_lookup_and_recreation` → both PASS (duplicate → `Conflict` with state unmutated; deleted user → lookup `None` and identity re-registrable).

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; `03-service-flows.md` prose updated.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean; confirm the 03-service-flows step-3 bullet carries the conflict → re-lookup prose with the `(audited UserCreated)` annotation retained.
  - *Status:* SATISFIED — `cargo fmt --all --check` clean (exit 0); `cargo clippy --workspace -- -D warnings` clean (exit 0); `cargo nextest run --workspace` → 264/264 passed, 12 skipped. `.specs/service/specs/03-service-flows.md:31-35` carries the conflict → re-lookup prose ("if creation returns `Conflict` … re-run `get_user_by_external_id` and continue with the existing user, re-applying the suspended-status check … emits no `UserCreated` event") with `(audited UserCreated)` retained on line 31. No new magic numbers introduced.

- **O6 — Reviewable: two tokens issued, exactly one user created.**
  - *Claim:* a reviewer runs the concurrent-first-login test and observes two `TokenResponse`s and one created user.
  - *Evidence to collect:* run the race test; inspect the mock's user count (1) and both returns (`Ok`).
  - *Status:* SATISFIED — exercised as reviewer: `cargo nextest run -p oidc-exchange-core -E 'test(exchange_conflict)'` → PASS. The test asserts both exchanges return `Ok` with non-empty access tokens, `repo.get_all_users().len() == 1`, and both tokens' `sub` claims equal the single user's id — two tokens issued, exactly one user created.

## Regression check

- `exchange()` non-racing found-user path — trace an existing active user → expect a token as before, no re-lookup taken : PRESERVED — the found-user branch at `exchange.rs:90-95` is untouched (the diff only adds inside the `create_user` result match); pre-existing tests `exchange_existing_user_does_not_create_new` and `exchange_suspended_user_is_rejected` PASS.
- `exchange()` registration-policy branches (allowlist / existing_users_only) — trace a denied registration → expect `AccessDenied` unchanged : PRESERVED — `exchange.rs:100-129` unchanged; `exchange_domain_allowlist_rejects_non_matching_domain`, `exchange_existing_users_only_rejects_new_user`, `exchange_no_email_rejected_when_allowlist_configured`, `exchange_existing_user_bypasses_domain_allowlist` all PASS.

## Residue

- Wiring the actual `UserCreated` audit emission is owned by the audit-emission change spec; this task only ensures no second create/event on the losing racer.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with direct evidence (three new core race tests, two new mock unit tests, fmt/clippy/nextest 264-of-264 clean, spec prose updated with the `(audited UserCreated)` annotation retained), and both named regression surfaces are PRESERVED — the Conflict → re-lookup branch at `exchange.rs:137-180` satisfies every DoD item. One note (not a defect): the re-lookup-absent branch surfaces `Error::StoreError` with a descriptive detail as its "distinct error", which the task's step 2 permits ("surface a distinct error rather than panicking").
