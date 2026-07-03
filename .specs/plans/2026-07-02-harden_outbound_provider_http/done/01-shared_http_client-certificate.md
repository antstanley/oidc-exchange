# Done Certificate — Task 01: Shared timed-out HTTP client

**Task:** [01-shared_http_client.md](01-shared_http_client.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The task routes every outbound provider call through a single shared `reqwest::Client` with a 5s connect / 10s total timeout and redirects disabled, so a delayed provider fails rather than stalling `/token`.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing behaviour of `jwks::fetch_keys`, `discovery::discover`, `token_endpoint::exchange_code`, `OidcProvider::revoke_token`, and `AppleProvider::revoke_token` on the success path — each still performs the same GET/POST and parses the same body; only the client construction changes.

## Obligations

- **O1 — All five outbound call sites use the shared client.**
  - *Claim:* `shared::jwks.rs`, `shared::discovery.rs`, `shared::token_endpoint.rs`, `oidc/mod.rs`, and `providers/src/apple.rs` obtain their client from `shared::http::client()`; no `reqwest::get` or `reqwest::Client::new()` remains for a provider call in those files.
  - *Evidence to collect:* read `crates/adapters/src/shared/http.rs` and confirm a `OnceLock<reqwest::Client>` accessor. `grep -rn "reqwest::get\|reqwest::Client::new" crates/adapters/src/shared crates/adapters/src/oidc crates/providers/src` — expect no provider-call hits (the `webhook` client is out of scope).
  - *Checks:* resolve the client used at `jwks.rs:72`, `discovery.rs:20`, `token_endpoint.rs:12`, `oidc/mod.rs:175`, `apple.rs:300` — confirm each is `shared::http::client()`, not a freshly built local client.
  - *Status:* ☑ SATISFIED — `crates/adapters/src/shared/http.rs` defines `static SHARED_CLIENT: OnceLock<reqwest::Client>` with `pub fn client() -> &'static reqwest::Client`. Grep over the three trees returns zero hits for `reqwest::get`/`reqwest::Client::new` (the only `Client::builder()` is inside `http.rs` itself). All five call sites resolved: `jwks.rs:72`, `discovery.rs:20`, `token_endpoint.rs:12` use `crate::shared::http::client()`; `oidc/mod.rs:202` uses `crate::shared::http::client()`; `apple.rs:304` uses `oidc_exchange_adapters::shared::http::client()` (adapters exports `pub mod shared`, providers depends on `oidc-exchange-adapters`).

- **O2 — Timeouts are named constants and redirects are disabled.**
  - *Claim:* the 5s connect and 10s total timeouts are named constants with units in the identifier, and the builder sets `redirect::Policy::none()`.
  - *Evidence to collect:* read `crates/adapters/src/shared/http.rs`; confirm two `const` timeout values (units in the name), `connect_timeout`/`timeout` set from them, and `redirect(reqwest::redirect::Policy::none())`. Grep the builder for numeric-literal durations — expect none.
  - *Status:* ☑ SATISFIED — `http.rs:13` `const CONNECT_TIMEOUT_SECS: u64 = 5;`, `http.rs:16` `const REQUEST_TIMEOUT_SECS: u64 = 10;` (units in both identifiers). `build_client()` sets `.connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))`, `.timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))`, `.redirect(reqwest::redirect::Policy::none())`; no numeric-literal durations in the builder. A `redirects_are_not_followed` wiremock test additionally proves a 302 is returned unfollowed.

- **O3 — Negative-space test: a delayed response times out.**
  - *Claim:* a `wiremock` endpoint that delays past the total timeout makes the outbound call fail (a `ProviderError`/`ProviderTimeout`) instead of hanging.
  - *Evidence to collect:* run the new delayed-response test in `crates/adapters` (e.g. under `shared::http` or `shared::jwks` tests) — expect PASS, with a `ResponseTemplate` delay greater than the total-timeout constant and an assertion on `is_err()`.
  - *Checks:* confirm the test's delay exceeds the `timeout` constant so the failure is the timeout, not another error.
  - *Status:* ☑ SATISFIED — `shared::http::tests::delayed_response_past_total_timeout_fails_the_call` PASSED (11.99s runtime, consistent with the 10s timeout firing). The delay is `Duration::from_secs(REQUEST_TIMEOUT_SECS + 5)` = 15s > 10s total timeout; the test asserts `expect_err(...)` and `err.is_timeout()` — stronger than the required `is_err()`, and specifically proves the failure is the timeout. (The test exercises the shared client directly; the `ProviderError` mapping at the jwks/discovery/token-endpoint call sites is unchanged and covered by their existing error-path tests.)

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-adapters -p oidc-exchange-providers` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` exit 0; `cargo clippy --workspace -- -D warnings` exit 0 (the exact command CI runs in `.github/workflows/ci.yml:23`); `cargo nextest run --workspace` 240 passed / 0 failed / 10 skipped. Limits are named constants (O2). Caveat (outside the named check): `cargo clippy --workspace --all-targets -- -D warnings` flags `clippy::assertions_on_constants` in the new `timeouts_are_smaller_than_a_generous_upper_bound` test — not a baseline/CI failure, noted in Residue.

- **O5 — Reviewable: every provider outbound call resolves to the shared client and the timeout test passes.**
  - *Claim:* a reviewer can confirm the grep is clean and the delayed-response test passes.
  - *Evidence to collect:* run the delayed-response test and re-run the O1 grep across the three crates — expect a passing test and no residual `reqwest::get`/`Client::new()` provider call.
  - *Status:* ☑ SATISFIED — exercised, not assumed: the delayed-response test was run and passed (O3), and `grep -rn "reqwest::get\|reqwest::Client::new" crates/adapters/src/shared crates/adapters/src/oidc crates/providers/src` returned no hits (exit 1). Every provider outbound call resolves to `shared::http::client()` (O1 resolution table).

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `OidcProvider::exchange_code` / `revoke_token` still complete a successful POST via the shared client → expect the existing `oidc` success-path tests still pass : ☑ PRESERVED — `oidc::tests::exchange_code_returns_provider_tokens`, all `oidc::tests::*` (16) and `token_endpoint::tests::*` (4) PASS; `apple::tests::revoke_token_posts_with_client_secret` and `apple::tests::exchange_and_validate_flow` PASS in the same run.
- `discovery::discover` and `jwks::JwksCache::get_keys` still fetch and parse a 200 body via the shared client → expect existing `discovery`/`jwks` tests still pass : ☑ PRESERVED — `shared::discovery::tests::*` (4) and `shared::jwks::tests::*` (3, incl. `first_call_fetches_from_url`, `stale_cache_triggers_refresh`) PASS; full workspace 240/240 green.

## Residue

- Per-endpoint status/error handling (JWKS fail-closed, token-endpoint OAuth errors, discovery issuer) is out of scope here — it lands in tasks 02, 04, 05. Not obligations of Task 01.
- Validator note: `cargo clippy --workspace --all-targets -- -D warnings` (a stricter invocation than the baseline/CI command) flags `clippy::assertions_on_constants` on the `REQUEST_TIMEOUT_SECS >= CONNECT_TIMEOUT_SECS` sanity assertion in the new test module. The contract-named command and CI both pass; fixing (e.g. a `const` block) is optional polish, not a DoD failure.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: All five obligations SATISFIED with direct evidence — all five outbound call sites resolve to the shared `OnceLock` client with named 5s/10s timeout constants and redirects disabled, the wiremock delayed-response test passes proving the timeout is wired, fmt/clippy(CI command)/nextest are clean (240/240), and both regression traces are PRESERVED; only residue is an optional `--all-targets` clippy pedantry note on a test-only const assertion.
