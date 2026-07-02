# Task 04 — Surface token-endpoint OAuth errors and require id_token

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-token_endpoint_errors-certificate.md](04-token_endpoint_errors-certificate.md)

**Implements:** [`02-ports-and-adapters.md` §Shared OIDC utilities](../../../service/specs/02-ports-and-adapters.md) (`token_endpoint::exchange_code` — a non-2xx response is parsed as an OAuth error body and propagated as a `ProviderError` naming `error` and `error_description`; a 2xx without an `id_token` is an error, not an empty string)
**Depends on:** 01
**Produces:** a non-2xx token-endpoint response surfaces its OAuth `error`/`error_description` as a `ProviderError` (so a `400 {"error":"invalid_grant"}` no longer reappears downstream as "Invalid JWT header"); a 2xx response missing `id_token` is an error rather than an empty-string default
**Pointers:** `crates/adapters/src/shared/token_endpoint.rs:23-42` — response handling; the `id_token` default at `token_endpoint.rs:39` (`unwrap_or_default()`)

## Steps

- [x] After `send()`, capture `response.status()`; on a non-2xx status read the body, parse `{"error", "error_description"}`, and return `Error::ProviderError` naming both fields (falling back to the raw body when it is not a JSON OAuth error).
- [x] On a 2xx response, parse the JSON and return `Error::ProviderError` (or `InvalidGrant`, matching the crate's convention) when `id_token` is absent — drop the `unwrap_or_default()` at `:39` so `id_token` is never silently the empty string.
- [x] Add ≥2 assertions to `exchange_code` (e.g. assert the endpoint string is non-empty; assert the returned `id_token` is non-empty on the success path).
- [x] Add `wiremock` tests: a `400 {"error":"invalid_grant","error_description":"..."}` surfaces both in the error; a `200` body with no `id_token` returns an error.
- [x] Confirm the existing success-path tests (`exchange_code_sends_correct_form_and_parses_response`, `exchange_code_handles_missing_optional_tokens`) still pass, since all three of those responses carry an `id_token`.

## Definition of done

- [x] A non-2xx token-endpoint response returns `Error::ProviderError` whose detail names the OAuth `error` and `error_description` — verified by a `wiremock` 400 `invalid_grant` test.
- [x] A 2xx response without `id_token` returns an error, never an empty-string `id_token` (the `unwrap_or_default()` is gone).
- [x] Negative-space tests cover both new rejection paths (non-2xx OAuth error; 2xx missing `id_token`).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the token-endpoint error tests and confirms a `400 invalid_grant` surfaces `invalid_grant` rather than a JWT-header message.
