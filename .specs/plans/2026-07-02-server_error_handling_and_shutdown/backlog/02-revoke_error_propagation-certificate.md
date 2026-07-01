# Done Certificate — Task 02: fail-safe `/revoke` error propagation

**Task:** [02-revoke_error_propagation.md](02-revoke_error_propagation.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> Verification protocol for Task 02. A validating agent discharges it: for each obligation,
> collect the named evidence, run the named checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** `/revoke` returns 503 when the session repository fails, while still returning
  200 for a revoked, invalid, or unknown token — a stolen-token client is never falsely told the
  session is dead.
- **P2 — Obligations.** Done iff O1…O6 all hold, in DoD order; O6 is the Reviewable item.
- **P3 — Invariants.** Must not break the token-verification best-effort path (an `access_token`
  whose signature fails still returns 200) or the idempotent-delete semantics of the
  `SessionRepository` adapters.

## Obligations

- **O1 — 503 on store failure, 200 on success.**
  - *Claim:* `revoke_handler` returns 503 with the standard body when `revoke` yields
    `Err(StoreError)`, and 200 when it yields `Ok(())`.
  - *Evidence to collect:* read `crates/core/src/service/revoke.rs:20,28,34` — confirm each
    `let _ =` became `?`. Read `crates/server/src/routes/revoke.rs` — confirm the result is
    matched (`Ok` → 200, `Err` → 503 with `{"error","error_description"}`). Run the new
    `/revoke` E2E tests — expect the store-failure case PASS at 503 and the success case at 200.
  - *Checks:* resolve `revoke_all_user_sessions` / `revoke_session` in `revoke.rs` to the
    `session_repo` port methods (`crates/core/src/ports/repository.rs:27-28`), confirming the
    propagated `Err` is `Error::StoreError`, not a shadowed local.
  - *Status:* ☐ unverified

- **O2 — Negative-space: token-verification failure still returns 200.**
  - *Claim:* an `access_token`-hint request with a malformed/unsigned token returns 200 and never
    propagates.
  - *Evidence to collect:* run the E2E case driving `/revoke` with a bad-signature token —
    expect 200. Trace `verify_and_extract_sub` returning `None` → the `Ok(())` arm, confirming no
    `?` fires on that path.
  - *Status:* ☐ unverified

- **O3 — The 503 path logs the detail; the client body leaks nothing.**
  - *Claim:* the `Err` arm emits `tracing::error!` with the error (captured under the request
    span from task 01) and the client body carries no infrastructure detail.
  - *Evidence to collect:* read the handler's `Err` arm; confirm the `tracing::error!` call and
    that the response body is a fixed generic message. Optionally capture the event with a tracing
    subscriber and confirm `request_id` is present.
  - *Status:* ☐ unverified

- **O4 — Two meaningful assertions each in `revoke` and `revoke_handler`.**
  - *Claim:* both functions carry two or more non-trivial assertions.
  - *Evidence to collect:* read both functions; count and confirm each assertion guards a real
    property (e.g. non-empty token, 64-char hash).
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass and lint/format are clean.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D
    warnings`, `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O6 — Reviewable: 503 on store failure, 200 on success, 200 on verification failure.**
  - *Claim:* a reviewer runs the `/revoke` E2E tests and sees the three outcomes.
  - *Evidence to collect:* run the `/revoke` E2E suite and read the three assertions (503, 200,
    200) passing.
  - *Status:* ☐ unverified

## Regression check

- `crates/server/src/routes/revoke.rs:revoke_handler` is the sole caller of
  `AppService::revoke`; trace it with a success token → expect 200 unchanged (the RFC 7009
  always-200-for-token-state contract preserved) : ☐ (PRESERVED / REGRESSION)
- Existing revoke tests in `crates/core` / `crates/server` that assumed a swallowed error still
  pass under the new propagation (updated where they asserted the old 200-on-failure) : ☐
  (PRESERVED / REGRESSION)

## Residue

- The 503 uses `503 Service Unavailable` per RFC 7009 §2.2.1, not 500 — confirm the status
  constant is `SERVICE_UNAVAILABLE`. Outside the DoD but load-bearing for the Decision.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
