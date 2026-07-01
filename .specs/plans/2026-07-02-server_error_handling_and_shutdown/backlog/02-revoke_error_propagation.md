# Task 02 — fail-safe `/revoke` error propagation

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-revoke_error_propagation-certificate.md](02-revoke_error_propagation-certificate.md)

**Implements:** [03-service-flows.md](../../../service/specs/03-service-flows.md) §Revocation and [04-http-api.md](../../../service/specs/04-http-api.md) §Routes → Public (`/revoke` row) and §Decisions (_200 for token state, 503 for infrastructure_)
**Depends on:** 01 (review — the 503 path's `tracing::error!` detail log is reviewed with real request-id correlation once the span exists)
**Produces:** `/revoke` returns 503 (not a false 200) when the session repository fails, while still returning 200 for a revoked, invalid, or unknown token.
**Pointers:** `crates/core/src/service/revoke.rs:20` (`let _ = ... revoke_all_user_sessions`), `:28` and `:34` (`let _ = ... revoke_session`); `crates/server/src/routes/revoke.rs:20-28` (`let _ = state.service.revoke(...)` then unconditional `StatusCode::OK`).

## Steps

- [ ] In `crates/core/src/service/revoke.rs`, propagate the session-repo `Result` with `?` at lines 20, 28, and 34 — a `StoreError` from `revoke_all_user_sessions` / `revoke_session` now bubbles out of `revoke`, while `verify_and_extract_sub` returning `None` (token-verification failure) still yields `Ok(())` and 200.
- [ ] Add two meaningful assertions to `revoke` (e.g. assert the token is non-empty on entry; assert the token-hash hex length is 64 before the store call).
- [ ] In `crates/server/src/routes/revoke.rs`, match the `revoke` result: `Ok(())` → `StatusCode::OK`; `Err(e)` → `tracing::error!` the error detail (so it is captured under the request span) and return `503 Service Unavailable` with the standard `{"error", "error_description"}` body.
- [ ] Add two meaningful assertions to `revoke_handler` and keep the handler free of business logic (parse, call core, map result — per the Rust conventions).
- [ ] Add server E2E tests in `crates/server/tests/` driving `/revoke` with a mock session repo: success → 200; a `StoreError` from the store → 503 with the error body; a token-verification failure (malformed/unsigned token, `access_token` hint) → 200 with no propagation.

## Definition of done

- [ ] `/revoke` returns 503 with the standard error body when the session repository returns `StoreError`, and 200 when the revoke succeeds.
- [ ] Negative-space: a token-verification failure on the `access_token` path (bad signature or malformed token) is still swallowed and returns 200 — the best-effort carve-out is preserved for token state only.
- [ ] The 503 path logs the error detail via `tracing::error!` (captured under the request span from task 01), and the generic client body leaks no infrastructure detail.
- [ ] Both `revoke` (core) and `revoke_handler` (server) carry at least two meaningful assertions each.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the `/revoke` E2E tests and sees 503 on a store failure, 200 on success, and 200 on a token-verification failure.
