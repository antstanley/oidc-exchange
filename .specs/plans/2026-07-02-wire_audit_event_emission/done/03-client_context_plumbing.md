# Task 03 — client context plumbing

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-client_context_plumbing-certificate.md](03-client_context_plumbing-certificate.md)

**Implements:** [01-domain-model.md](../../../service/specs/01-domain-model.md) §Entities → Session (client-context fields populated at issuance; removes the resolved Open question); [03-service-flows.md](../../../service/specs/03-service-flows.md) §Token exchange (the client-context wording, session population)
**Depends on:** —
**Produces:** the core request structs carry `ip_address`/`user_agent`/`device_id`; the exchange flow stores them on the session; `create_audit_event` takes the context instead of hardcoding `None`.
**Pointers:** `crates/core/src/service/exchange.rs:10-15` (`ExchangeRequest`), `exchange.rs:152-162` (session construction, currently `None`); `crates/core/src/service/refresh.rs:8-10` (`RefreshRequest`); `crates/core/src/service/revoke.rs:8-11` (`RevokeRequest`); `crates/core/src/service/mod.rs:132-151` (`create_audit_event`, hardcoded `None` at `mod.rs:146-147`)

## Steps

- [x] Add `ip_address: Option<String>`, `user_agent: Option<String>`, `device_id: Option<String>` to `ExchangeRequest`, `RefreshRequest`, and `RevokeRequest`.
- [x] Populate the stored session at `exchange.rs:158-160` from `request.device_id`/`user_agent`/`ip_address` instead of `None`.
- [x] Extend `create_audit_event` to accept `ip_address: Option<String>` and `user_agent: Option<String>` parameters and set them on the returned `AuditEvent` instead of the hardcoded `None` at `mod.rs:146-147`; update every current caller (core tests) to pass the new arguments.
- [x] Update the existing constructors of these request structs in the core tests and `crates/server` handlers to set the new fields (default `None`) so the workspace still compiles; add a core test that an `ExchangeRequest` carrying an ip/ua/device stores a session with those exact values.

## Definition of done

- [x] `ExchangeRequest`/`RefreshRequest`/`RevokeRequest` each carry the three `Option<String>` client-context fields, and the exchange flow writes all three onto the stored `Session` (no longer `None`).
- [x] `create_audit_event` records the passed `ip_address`/`user_agent` on the event rather than hardcoding `None`; it takes no `device_id` (the `AuditEvent` shape has none).
- [x] Negative-space test: an `ExchangeRequest` with all three fields `None` stores a session with `None` for each (no accidental default substitution), and one with values stores them verbatim.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run `cargo nextest run -p oidc-exchange-core` and observe the new session-population test pass; the workspace builds with every request-struct caller updated.
