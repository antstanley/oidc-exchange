# Done Certificate — Task 02: Webhook retry backoff cap

**Task:** [02-webhook_backoff_cap.md](02-webhook_backoff_cap.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — `mod.rs:11,16,19` define `BASE_BACKOFF_MS = 100`, `MAX_BACKOFF_SHIFT: u32 = 6`, `MAX_BACKOFF_MS: u64 = 5_000` as named constants. `backoff_delay` (`mod.rs:28-32`) computes `BASE_BACKOFF_MS.saturating_mul(1u64 << shift)` then `.min(MAX_BACKOFF_MS)`, clamping to the cap. The retry loop at `mod.rs:76` calls `tokio::time::sleep(backoff_delay(attempt))`; the old inline `100 * (1 << (attempt - 1))` literal is gone (confirmed via `jj diff`). `backoff_delay` at :76 resolves to the module-level `fn backoff_delay` at :28 — no shadowing.

- **O2 — Negative-space test: for `retries = 20`, every delay is capped and no shift overflows.**
  - *Claim:* `backoff_delay` returns a delay `<= MAX_BACKOFF_MS` for every attempt across a `retries = 20` range, and the shift never overflows.
  - *Evidence to collect:* run the new backoff-cap unit test — expect PASS, asserting each `backoff_delay(attempt)` is `<= MAX_BACKOFF_MS` for attempts up to 20 (and beyond the shift bound), with no panic. Confirm the test does not actually sleep (it calls the pure helper).
  - *Checks:* confirm the shift operand is clamped with `.min(MAX_BACKOFF_SHIFT)` on a `u64` so `1u64 << shift` cannot reach the width of the type.
  - *Status:* ☑ SATISFIED — `test_backoff_delay_is_capped_and_never_overflows` (`mod.rs:182-207`) PASSES: it loops `attempt in 1..=20` asserting every `backoff_delay(attempt) <= MAX_BACKOFF_MS`, no panic. Shift is clamped at `mod.rs:29` `let shift = (attempt - 1).min(MAX_BACKOFF_SHIFT)` (max 6), and `1u64 << shift` is at most `1u64 << 6 = 64` — far below the 64-bit width — with `saturating_mul` further guarding the product. The test calls the pure helper only; it does not sleep.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named, the touched functions keep ≥2 meaningful assertions.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-adapters` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` exit 0 (clean); `cargo clippy --workspace -- -D warnings` exit 0 (clean); `cargo nextest run -p oidc-exchange-adapters` all pass (126 tests); full `cargo nextest run --workspace` = 339 passed, 0 failed. Limits are named constants (O1); the backoff-cap test carries 24 assertions and the touched retry path is covered by `test_retry_on_5xx`.

- **O4 — Reviewable: `backoff_delay` returns a capped, non-overflowing delay for high attempt counts.**
  - *Claim:* a reviewer can confirm the delay is bounded and safe at large attempt counts.
  - *Evidence to collect:* run the backoff-cap unit test and read `backoff_delay`; observe the returned delay is `<= 5000ms` at attempt 20 and that the shift is clamped.
  - *Status:* ☑ SATISFIED — the test PASSES and asserts `backoff_delay(20) == Duration::from_millis(MAX_BACKOFF_MS)` (5000ms) and `backoff_delay(MAX_BACKOFF_SHIFT + 1) == 5000ms` (`mod.rs:199-206`); reading `backoff_delay` (`mod.rs:28-32`) confirms the shift clamp `.min(MAX_BACKOFF_SHIFT)` and the result clamp `.min(MAX_BACKOFF_MS)`. Delay is bounded and non-overflowing at high attempt counts.

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `WebhookUserSync::send_webhook` on a 5xx-then-200 sequence still retries and eventually succeeds → expect the existing `test_retry_on_5xx` test still passes (retry behaviour preserved, only the delay is bounded) : ☑ PRESERVED — `test_retry_on_5xx` PASSES (3 requests: 2×500 then 200). The loop still iterates `0..=self.retries` (`mod.rs:73`), still sleeps before attempts > 0 (`mod.rs:74-77`), still retries on 5xx (`:93-95`) and timeout/connect (`:102-105`); only the delay *value* now flows through `backoff_delay`. `test_4xx_no_retry` and `test_redirect_is_not_followed_and_is_not_retried` also still PASS.

## Residue

- The 2xx-only / no-redirect delivery semantics (task 01) and the config `retries` clamp (task 03) are out of scope here. Not obligations of Task 02.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O4 all SATISFIED with evidence — named constants + `backoff_delay` clamp the per-attempt delay to a 5s cap and the shift to 6, the `retries = 20` negative-space test PASSES with no overflow, fmt/clippy/tests are clean, and the `test_retry_on_5xx` regression caller is PRESERVED.
