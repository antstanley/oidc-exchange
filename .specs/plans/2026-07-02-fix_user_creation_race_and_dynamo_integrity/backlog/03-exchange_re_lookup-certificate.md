# Done Certificate — Task 03: Exchange flow conflict → re-lookup

**Task:** [03-exchange_re_lookup.md](03-exchange_re_lookup.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — The re-lookup path re-applies the suspended check.**
  - *Claim:* when the winning user is `Suspended`, the losing racer's re-lookup returns `UserSuspended`, not a token.
  - *Evidence to collect:* run the test where the pre-existing user found on re-lookup is `Suspended` — expect `Err(Error::UserSuspended { .. })`.
  - *Status:* ☐ unverified

- **O3 — Negative-space: a non-Conflict create error propagates.**
  - *Claim:* a `StoreError` from `create_user` is returned as an error, not swallowed into a re-lookup.
  - *Evidence to collect:* run the test injecting a non-`Conflict` `create_user` failure — expect the error propagates (no re-lookup, no token).
  - *Checks:* confirm the match arm distinguishes `Error::Conflict` from other `Err` values (only `Conflict` triggers re-lookup).
  - *Status:* ☐ unverified

- **O4 — MockRepository enforces non-deleted uniqueness and excludes deleted from lookup.**
  - *Claim:* `MockRepository::create_user` returns `Conflict` on a duplicate live `(provider, external_id)`, and `get_user_by_external_id` skips `Deleted` users.
  - *Evidence to collect:* read `crates/test-utils/src/lib.rs` `create_user` and `get_user_by_external_id`; run a mock unit test asserting the duplicate → `Conflict` and a deleted user → `None`.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; `03-service-flows.md` prose updated.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean; confirm the 03-service-flows step-3 bullet carries the conflict → re-lookup prose with the `(audited UserCreated)` annotation retained.
  - *Status:* ☐ unverified

- **O6 — Reviewable: two tokens issued, exactly one user created.**
  - *Claim:* a reviewer runs the concurrent-first-login test and observes two `TokenResponse`s and one created user.
  - *Evidence to collect:* run the race test; inspect the mock's user count (1) and both returns (`Ok`).
  - *Status:* ☐ unverified

## Regression check

- `exchange()` non-racing found-user path — trace an existing active user → expect a token as before, no re-lookup taken : ☐ (PRESERVED / REGRESSION)
- `exchange()` registration-policy branches (allowlist / existing_users_only) — trace a denied registration → expect `AccessDenied` unchanged : ☐ (PRESERVED / REGRESSION)

## Residue

- Wiring the actual `UserCreated` audit emission is owned by the audit-emission change spec; this task only ensures no second create/event on the losing racer.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
