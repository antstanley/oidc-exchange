# Done Certificate — Task 05: Throttle `/nonce`

**Task:** [05-throttle_nonce.md](05-throttle_nonce.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-25 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 05. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive the
> Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do not
> record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 05) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation names
(a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `/nonce` shares the server-established per-IP throttle budget with `/token`/`/revoke`; over-budget returns `429 slow_down` with `Retry-After` and emits the mandatory `ThrottleExceeded`.
- **P2 — Obligations.** Done iff O1…O4 all hold. One Oi per definition-of-done item, in DoD order; O4 is the Reviewable item.
- **P3 — Invariants.** Must not extend the failed-attempt budget (`per_ip_failures`) to `/nonce`, must leave `/keys` and `/health` un-throttled, and must not change `/nonce`'s mount condition (`grants.id_token` enabled).

## Obligations

- **O1 — `/nonce` over-budget returns 429 and audits.**
  - *Claim:* exhausting the per-IP budget against `/nonce` returns `429` with `error == "slow_down"` and `Retry-After >= 1` and emits the mandatory `ThrottleExceeded` event.
  - *Evidence to collect:* run the new throttle e2e (beside `crates/server/tests/e2e.rs`, using `build_throttled_router` at `:129`) flooding `/nonce` from one peer — expect a `429`, body `error == "slow_down"`, `Retry-After >= 1`, and one `ThrottleExceeded` audit event.
  - *Checks:* resolve the path guard in `public_throttle_layer` (`public_throttle.rs:61`) — confirm it is `matches!(path, "/token" | "/revoke" | "/nonce")`, the early-return path, not a per-route mount change.
  - *Status:* ☐ unverified

- **O2 — Budget shared; no-address requests unthrottled.**
  - *Claim:* the `/nonce` budget is shared with `/token` (same `RateLimitKey::ClientAddr`), and a request with no server-established address is not throttled.
  - *Evidence to collect:* run the shared-budget test — spend budget on `/token`, then expect `/nonce` from the same peer already throttled; run the no-address test — expect a request without a `Peer`/`Forwarded` address to pass the throttle (concurrency guard only).
  - *Checks:* resolve the rate-limit key built for `/nonce` — confirm it is `RateLimitKey::ClientAddr` (the normal per-IP budget), not `per_ip_failures`.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* the behaviour is tested with meaningful assertions and format/lint/test gates pass.
  - *Evidence to collect:* run `cargo fmt` (check), `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean (per [development-guidelines.md](../../../development-guidelines.md) §Definition of done).
  - *Status:* ☐ unverified

- **O4 — Reviewable: flooding `/nonce` yields 429 + ThrottleExceeded (Reviewable).**
  - *Claim:* a reviewer floods `/nonce` from one peer in the e2e and sees a `429 slow_down` with a `ThrottleExceeded` audit event once the shared budget is spent.
  - *Evidence to collect:* run the `/nonce` throttle e2e and read the `429` response plus the emitted `ThrottleExceeded` event.
  - *Status:* ☐ unverified

## Regression check

- The existing `/token`/`/revoke` throttle e2e (`e2e.rs` around `:775`): expect it still passes — the added `/nonce` arm must not change existing throttled-path behaviour : ☐ (PRESERVED / REGRESSION)
- `/keys` and `/health` requests: trace one through `public_throttle_layer` → expect the early-return (concurrency guard only), still un-throttled : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: `/nonce` clients sharing a NAT egress with heavy `/token` traffic now share the default 60/min per-IP budget — an intended, documented behaviour change per the change spec.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
