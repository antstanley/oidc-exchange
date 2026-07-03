# Task 02 — Webhook retry backoff cap

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-webhook_backoff_cap-certificate.md](02-webhook_backoff_cap-certificate.md)

**Implements:** [`02-ports-and-adapters.md` §Webhook adapter contract](../../../service/specs/02-ports-and-adapters.md) — "5xx or timeout retries up to `retries` with exponential backoff capped at 5s per attempt"
**Depends on:** —
**Produces:** the per-attempt retry delay is bounded — exponential up to a named 5s cap, with the left shift clamped so a large `retries` can no longer overflow (`1 << 32` panic in debug) or accumulate hours of sleep inside a request
**Pointers:** `crates/adapters/src/webhook/mod.rs:51` (`std::time::Duration::from_millis(100 * (1 << (attempt - 1)))` — the uncapped, overflow-prone delay); retry loop `mod.rs:48-53`

## Steps

- [x] Add named constants for the delay bound in the webhook module: the base delay (100ms), the maximum shift so `1 << shift` cannot overflow, and the per-attempt cap (5000ms) — e.g. `BASE_BACKOFF_MS`, `MAX_BACKOFF_SHIFT`, `MAX_BACKOFF_MS`.
- [x] Extract the delay computation into a pure helper (e.g. `fn backoff_delay(attempt: u32) -> Duration`) that computes `BASE_BACKOFF_MS * (1u64 << (attempt - 1).min(MAX_BACKOFF_SHIFT))` and clamps the result to `MAX_BACKOFF_MS`, so it can be unit-tested without sleeping.
- [x] Replace the inline delay expression at `mod.rs:51` with a call to the helper.
- [x] Add a unit test that calls `backoff_delay` for attempts across a `retries = 20` range and asserts every returned delay is `<= MAX_BACKOFF_MS` and that no attempt panics (the shift never overflows).

## Definition of done

- [x] The per-attempt delay is exponential but never exceeds the named 5s cap; the base, shift bound, and cap are named constants in the webhook module, not literals.
- [x] Negative-space test: for `retries = 20` the computed delay for every attempt is `<= MAX_BACKOFF_MS` and no shift overflows (proving both the hours-long sleep and the `1 << 32` panic are removed).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits, ≥2 assertions per touched function — see plan.md baseline).
- [x] Reviewable: a reviewer runs the backoff-cap unit test and confirms `backoff_delay` returns a capped, non-overflowing delay for high attempt counts.
