# Done Certificate — Task 04: JIT `notify_user_created` in the exchange flow

**Task:** [04-jit_notify_in_exchange.md](04-jit_notify_in_exchange.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 04. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** When the exchange flow JIT-registers a user, it fires exactly one best-effort `notify_user_created` — awaited (not spawned), result discarded, failure logged and never failing the exchange.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing exchange flow (`exchange.rs:52-174`): the found-active-user and found-suspended branches, the registration-policy denials (allowlist, `existing_users_only`), refresh-token minting, session storage, and access-token signing are unchanged; the notify is added only inside the `None` create-user branch.

## Obligations

- **O1 — Exactly one `notify_user_created` on JIT registration, awaited before the response; none for existing users.**
  - *Claim:* the `None` registration branch, after `create_user`, awaits `self.user_sync.notify_user_created(&user)` once before returning the token; the found-active-user branch fires no notification.
  - *Evidence to collect:* read `crates/core/src/service/exchange.rs` around line 137 — confirm the notify is inside the `None` branch after `create_user`, `.await`ed (not `tokio::spawn`ed), before the refresh-token/session/access-token steps. Run the new JIT-notify test with a `MockUserSync` (or `wiremock`) — assert exactly one `user.created` call for a new user and zero for an existing active user.
  - *Checks:* resolve `self.user_sync.notify_user_created` to the `UserSync` port method (`ports/user_sync.rs`), matching the `admin_create_user` call at `user_admin.rs:16`; confirm the notify is not placed in the `Some(user)` active branch.
  - *Status:* ☐ unverified

- **O2 — Negative-space test: a sync failure never fails the exchange.**
  - *Claim:* when the sync backend fails every attempt, `exchange` still returns a `TokenResponse` and the failure is logged, not propagated.
  - *Evidence to collect:* run the JIT-notify failure test — a webhook that 500s on every attempt (or a `MockUserSync` set to fail); assert `exchange` returns `Ok(TokenResponse{..})` and that the error is logged via `tracing::warn!`, not returned.
  - *Checks:* trace the `Err` arm of the notify — confirm it logs and continues (mirrors `admin_create_user`), and that `?` is not used on the notify result (which would propagate the failure).
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, the touched function keeps ≥2 meaningful assertions.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O4 — Reviewable: one `user.created` fires on first login; a webhook that 500s still yields a token.**
  - *Claim:* a reviewer can confirm a JIT exchange fires exactly one `user.created` and that a failing webhook does not block token issuance.
  - *Evidence to collect:* run the JIT-notify test and observe one `user.created` on new-user exchange, zero on existing-user exchange, and a returned token even when the webhook fails every attempt.
  - *Status:* ☐ unverified

## Regression check

For each module the task touched, the validator traces one downstream caller:

- The exchange existing-user path (`Some(user)` active branch) still returns a token without a sync call, and the registration-policy denials still return `AccessDenied` → expect the existing `exchange`/registration-policy tests still pass : ☐ (PRESERVED / REGRESSION)
- `admin_create_user` (`user_admin.rs:13-21`), which shares the `notify_user_created` port, is unchanged → expect its existing admin tests still pass : ☐ (PRESERVED / REGRESSION)

## Residue

- Durable/queue-backed delivery (a future `UserSync` SQS adapter) is out of scope; this task keeps the HTTP webhook request-scoped, as the change spec decides. Not an obligation here.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
