# Task 05 — exchange flow emission

**Plan:** [plan.md](../plan.md) · **Certificate:** [05-exchange_flow_emission-certificate.md](05-exchange_flow_emission-certificate.md)

**Implements:** [03-service-flows.md](../../../service/specs/03-service-flows.md) §Token exchange (the success-path and rejection-branch audit events); resolves the §Open questions `Unauthorized` vs `UserSuspended` item
**Depends on:** 01, 03, 04
**Produces:** the exchange flow emits `UserSuspended` (warning, failure), `RegistrationDenied` (warning, failure) at each denial branch, `UserCreated` (notice, success), and `TokenExchange` (info, success) — each recording the request's ip/ua.
**Pointers:** `crates/core/src/service/exchange.rs:92` (`UserSuspended`), `:105`/`:110`/`:116`/`:126` (`RegistrationDenied` branches), `:137` (`UserCreated`), `:168` (`TokenExchange`, after the token response is assembled); `crates/core/src/service/mod.rs:102` (`emit_audit`), `mod.rs:132` (`create_audit_event`)

## Steps

- [x] At the suspended-user branch (`exchange.rs:92`), emit `UserSuspended` (warning severity, failure outcome) before returning the error.
- [x] At each registration-policy denial (`:105`/`:110`/`:116`/`:126`), emit `RegistrationDenied` (warning, failure) before returning.
- [x] After a new user is created (`:137`), emit `UserCreated` (notice, success); after the token response is assembled (`:168`), emit `TokenExchange` (info, success).
- [x] Build each event via `create_audit_event`, passing `request.ip_address`/`request.user_agent`, the actor (user id where known), and the provider; propagate `emit_audit`'s `Result` per the flow's blocking semantics.
- [x] Add emission tests via `MockAuditLog`: a suspended user yields exactly one `UserSuspended` event; a successful exchange yields `TokenExchange` (and `UserCreated` for a new user) carrying the request's ip/ua.

## Definition of done

- [x] Each named exchange point emits its named event with the correct severity/outcome, actor, provider, and the request's `ip_address`/`user_agent`; suspension emits `UserSuspended` (not `Unauthorized`).
- [x] Emission is gated by task 01's `emit_threshold` and follows `emit_audit`'s blocking rules; a failing audit under the blocking threshold propagates as `Err`.
- [x] Negative-space test: an exchange that rejects on the domain allowlist emits `RegistrationDenied` and no `TokenExchange`; a suspended user emits only `UserSuspended`.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run the exchange emission tests (`cargo nextest run -p oidc-exchange-core exchange`) and observe each named event recorded with its ip/ua via `MockAuditLog`.
