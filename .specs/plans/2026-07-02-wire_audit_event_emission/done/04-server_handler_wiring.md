# Task 04 — server handler wiring

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-server_handler_wiring-certificate.md](04-server_handler_wiring-certificate.md)

**Implements:** [04-http-api.md](../../../service/specs/04-http-api.md) §Middleware stack (the `AuditContext` extension consumed by the `/token` and `/revoke` handlers)
**Depends on:** 03
**Produces:** the `/token` and `/revoke` handlers read `Extension<AuditContext>` and pass its `ip_address`/`user_agent`/`device_id` into the core request structs, so a real request threads client context into the stored session.
**Pointers:** `crates/server/src/routes/token.rs:23-55` (`token_handler`, builds `ExchangeRequest`/`RefreshRequest`); `crates/server/src/routes/revoke.rs:16-29` (`revoke_handler`, builds `RevokeRequest`); `crates/server/src/middleware/audit_context.rs` (`AuditContext`); layer already installed at `crates/server/src/bootstrap.rs:135`

## Steps

- [x] Add an `Extension<AuditContext>` extractor to `token_handler` and `revoke_handler`.
- [x] In `token_handler`, populate `ExchangeRequest` and `RefreshRequest` `ip_address`/`user_agent`/`device_id` from the `AuditContext` (clone the fields).
- [x] In `revoke_handler`, populate `RevokeRequest`'s client-context fields from the `AuditContext`.
- [x] Add an end-to-end handler test that a `POST /token` (authorization_code grant) with `X-Forwarded-For`, `User-Agent`, and `X-Device-Id` headers stores a session whose `ip_address`/`user_agent`/`device_id` equal the header values.

## Definition of done

- [x] `token_handler` and `revoke_handler` consume `Extension<AuditContext>` and thread its three fields into the core request structs.
- [x] A request carrying the audit headers results in a stored session populated with those exact values; a request without them stores `None` for each.
- [x] Negative-space test: a `/token` request with no audit headers stores a session with `None` ip/ua/device (the handler passes through the middleware's `None` defaults, not empty strings).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run the server handler test suite (`cargo nextest run -p oidc-exchange-server`) and observe the header-to-session test pass; inspect that the stored session carries the header values.
