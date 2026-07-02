# Task 01 — Shared timed-out HTTP client

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-shared_http_client-certificate.md](01-shared_http_client-certificate.md)

**Implements:** [`02-ports-and-adapters.md` §Shared OIDC utilities](../../../service/specs/02-ports-and-adapters.md) (shared `reqwest::Client` with timeouts and redirects disabled); [`05-provider-system.md` §OidcProvider behaviour](../../../service/specs/05-provider-system.md) (all outbound calls use the shared timed-out client)
**Depends on:** —
**Produces:** every outbound provider call (JWKS, discovery, token endpoint, revocation) goes through a single shared `reqwest::Client` with a 5s connect timeout, a 10s total timeout, and redirects disabled; a delayed provider response fails the request instead of stalling `/token`
**Pointers:** new `crates/adapters/src/shared/http.rs`; `shared/mod.rs:1-3` (add `pub mod http;`); call sites `shared/jwks.rs:72` (`reqwest::get`), `shared/discovery.rs:20` (`reqwest::get`), `shared/token_endpoint.rs:12` (`reqwest::Client::new()`), `oidc/mod.rs:175` (`reqwest::Client::new()`), `crates/providers/src/apple.rs:300` (`reqwest::Client::new()`); reference pattern `webhook/mod.rs:19-24`

## Steps

- [x] Add `crates/adapters/src/shared/http.rs` exposing a process-wide shared client via a `OnceLock<reqwest::Client>` (e.g. `pub fn client() -> &'static reqwest::Client`), built with `connect_timeout`, `timeout`, and `redirect::Policy::none()`.
- [x] Declare the two timeouts as named constants with units in the identifier (e.g. `CONNECT_TIMEOUT_SECS`/`REQUEST_TIMEOUT_SECS` or `Duration` constants), not literals inside the builder.
- [x] Register the module in `shared/mod.rs`.
- [x] Replace `reqwest::get(&self.jwks_uri)` at `jwks.rs:72` and `reqwest::get(&url)` at `discovery.rs:20` with `http::client().get(...).send()`.
- [x] Replace `reqwest::Client::new()` at `token_endpoint.rs:12`, `oidc/mod.rs:175`, and `apple.rs:300` with the shared client.
- [x] Add a `wiremock` test that delays the response past the total timeout and asserts the outbound call returns an error rather than hanging.

## Definition of done

- [x] All five outbound call sites use the shared `http::client()`; no `reqwest::get` or `reqwest::Client::new()` remains under `crates/adapters/src/shared`, `crates/adapters/src/oidc`, or `crates/providers/src` for provider calls.
- [x] The 5s connect and 10s total timeouts are named constants with units in the identifier; the client sets `redirect::Policy::none()`.
- [x] Negative-space test: a `wiremock` endpoint that delays past the total timeout causes the call to fail (a `ProviderError`/`ProviderTimeout`), proving the timeout is wired.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the new delayed-response test and greps the three crates to confirm every provider outbound call resolves to `shared::http::client()`.
