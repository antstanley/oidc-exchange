# Task 06 — refresh flow emission

**Plan:** [plan.md](../plan.md) · **Certificate:** [06-refresh_flow_emission-certificate.md](06-refresh_flow_emission-certificate.md)

**Implements:** [03-service-flows.md](../../../service/specs/03-service-flows.md) §Token refresh (the refresh-flow audit events and the `ValidationFailed`-behind-`emit_threshold` behaviour)
**Depends on:** 01, 03, 04
**Produces:** the refresh flow emits `ValidationFailed` (debug, failure) on unknown/expired token or unknown user, `UserSuspended` on a suspended user, and `TokenRefresh` (info, success) on success — recording the request's ip/ua.
**Pointers:** `crates/core/src/service/refresh.rs:22` (unknown token), `:28` (expired token), `:38` (unknown user) → `ValidationFailed`; `:43` (suspended user) → `UserSuspended`; `:50` (success) → `TokenRefresh`; `crates/core/src/service/mod.rs:102`/`:132`

## Steps

- [x] Emit `ValidationFailed` (debug, failure) at the unknown-token (`:22`), expired-token (`:28`), and unknown-user (`:38`) branches before returning `InvalidToken`.
- [x] Emit `UserSuspended` at the suspended-user branch (`:43`).
- [x] Emit `TokenRefresh` (info, success) after the access token is built (`:50`).
- [x] Build each event via `create_audit_event` with `request.ip_address`/`request.user_agent`; propagate `emit_audit`'s `Result` per the blocking semantics.
- [x] Add tests: an unknown-token refresh emits `ValidationFailed` but the default `info` threshold suppresses dispatch (no event on `MockAuditLog`); lowering `emit_threshold` to `debug` surfaces it; a successful refresh emits `TokenRefresh`.

## Definition of done

- [x] Each named refresh point emits its named event with correct severity/outcome and the request's ip/ua; suspension emits `UserSuspended`.
- [x] `ValidationFailed` is emitted at `debug` severity and is suppressed under the default `info` `emit_threshold`, appearing only when the threshold is lowered to `debug`.
- [x] Negative-space test: an unknown/expired refresh under the default threshold records nothing on `MockAuditLog`; the same under a `debug` threshold records `ValidationFailed`.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run `cargo nextest run -p oidc-exchange-core refresh` and observe the threshold-gated `ValidationFailed` behaviour and the `TokenRefresh` success event.
