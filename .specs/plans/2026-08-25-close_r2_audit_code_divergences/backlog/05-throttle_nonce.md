# Task 05 — Throttle `/nonce`

**Plan:** [plan.md](../plan.md) · **Certificate:** [05-throttle_nonce-certificate.md](05-throttle_nonce-certificate.md)

**Implements:** [04-http-api.md](../../../service/specs/04-http-api.md) §Middleware stack (per-IP throttle); change spec §The delta → S11
**Depends on:** —
**Produces:** `/nonce` shares the server-established per-IP throttle budget with `/token`/`/revoke`; over-budget returns `429 slow_down` with `Retry-After` and emits the mandatory `ThrottleExceeded`.
**Pointers:** `crates/server/src/middleware/public_throttle.rs:61` (path set); `crates/server/tests/e2e.rs` (throttle e2e, `build_throttled_router` at `:129`, `ThrottleExceeded` assertions around `:775`)

## Steps

- [ ] Add `"/nonce"` to the throttled path set in `public_throttle_layer` (`public_throttle.rs:61`): `matches!(path, "/token" | "/revoke" | "/nonce")`.
- [ ] Confirm no mounting change is needed — the layer is router-wide with a path early-return, and `/nonce` is still mounted only when `grants.id_token` is enabled.
- [ ] Confirm the failed-attempt budget (`per_ip_failures`) is untouched — `/nonce` renders no authentication failure, so only the normal per-IP budget applies.

## Definition of done

- [ ] Throttle e2e beside the existing tests: exhausting the per-IP budget against `/nonce` returns `429` with `error == "slow_down"` and `Retry-After >= 1` and emits the mandatory `ThrottleExceeded` event.
- [ ] Negative-space / sharing: the `/nonce` budget is shared with `/token` (same `RateLimitKey::ClientAddr`), and a request with no server-established address is not throttled.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [ ] Reviewable: a reviewer floods `/nonce` from one peer in the e2e and sees a `429 slow_down` with a `ThrottleExceeded` audit event once the shared budget is spent.
