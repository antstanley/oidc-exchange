# Done Certificate — Task 02: Webhook retry backoff cap

**Task:** [02-webhook_backoff_cap.md](02-webhook_backoff_cap.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The per-attempt retry delay is exponential but capped at a named 5s, and the left shift is clamped so a large `retries` cannot overflow (`1 << 32` panic) or accumulate hours of sleep.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Must not break the retry-loop control flow of `send_webhook` (`webhook/mod.rs:48-97`) — retries still occur on 5xx and timeout/connect, the sleep still precedes the request on attempts > 0, and the attempt count is unchanged; only the delay *value* is bounded.

## Obligations

- **O1 — The delay is exponential but capped at a named 5s, with named constants.**
  - *Claim:* the per-attempt delay grows exponentially yet never exceeds `MAX_BACKOFF_MS` (5000ms); the base, shift bound, and cap are named constants, not literals.
  - *Evidence to collect:* read `crates/adapters/src/webhook/mod.rs` — confirm named constants for the base (100ms), the maximum shift, and the cap (5000ms), and that the delay helper clamps to the cap. Confirm no numeric-literal delay remains at the former `mod.rs:51`.
  - *Checks:* resolve the delay used in the retry loop — confirm the loop calls the new `backoff_delay` helper, not an inline `100 * (1 << (attempt - 1))` expression.
  - *Status:* ☐ unverified

- **O2 — Negative-space test: for `retries = 20`, every delay is capped and no shift overflows.**
  - *Claim:* `backoff_delay` returns a delay `<= MAX_BACKOFF_MS` for every attempt across a `retries = 20` range, and the shift never overflows.
  - *Evidence to collect:* run the new backoff-cap unit test — expect PASS, asserting each `backoff_delay(attempt)` is `<= MAX_BACKOFF_MS` for attempts up to 20 (and beyond the shift bound), with no panic. Confirm the test does not actually sleep (it calls the pure helper).
  - *Checks:* confirm the shift operand is clamped with `.min(MAX_BACKOFF_SHIFT)` on a `u64` so `1u64 << shift` cannot reach the width of the type.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named, the touched functions keep ≥2 meaningful assertions.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-adapters` — expect all clean.
  - *Status:* ☐ unverified

- **O4 — Reviewable: `backoff_delay` returns a capped, non-overflowing delay for high attempt counts.**
  - *Claim:* a reviewer can confirm the delay is bounded and safe at large attempt counts.
  - *Evidence to collect:* run the backoff-cap unit test and read `backoff_delay`; observe the returned delay is `<= 5000ms` at attempt 20 and that the shift is clamped.
  - *Status:* ☐ unverified

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `WebhookUserSync::send_webhook` on a 5xx-then-200 sequence still retries and eventually succeeds → expect the existing `test_retry_on_5xx` test still passes (retry behaviour preserved, only the delay is bounded) : ☐ (PRESERVED / REGRESSION)

## Residue

- The 2xx-only / no-redirect delivery semantics (task 01) and the config `retries` clamp (task 03) are out of scope here. Not obligations of Task 02.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
