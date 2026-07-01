# Done Certificate — Task 01: Shared timed-out HTTP client

**Task:** [01-shared_http_client.md](01-shared_http_client.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — Timeouts are named constants and redirects are disabled.**
  - *Claim:* the 5s connect and 10s total timeouts are named constants with units in the identifier, and the builder sets `redirect::Policy::none()`.
  - *Evidence to collect:* read `crates/adapters/src/shared/http.rs`; confirm two `const` timeout values (units in the name), `connect_timeout`/`timeout` set from them, and `redirect(reqwest::redirect::Policy::none())`. Grep the builder for numeric-literal durations — expect none.
  - *Status:* ☐ unverified

- **O3 — Negative-space test: a delayed response times out.**
  - *Claim:* a `wiremock` endpoint that delays past the total timeout makes the outbound call fail (a `ProviderError`/`ProviderTimeout`) instead of hanging.
  - *Evidence to collect:* run the new delayed-response test in `crates/adapters` (e.g. under `shared::http` or `shared::jwks` tests) — expect PASS, with a `ResponseTemplate` delay greater than the total-timeout constant and an assertion on `is_err()`.
  - *Checks:* confirm the test's delay exceeds the `timeout` constant so the failure is the timeout, not another error.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-adapters -p oidc-exchange-providers` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: every provider outbound call resolves to the shared client and the timeout test passes.**
  - *Claim:* a reviewer can confirm the grep is clean and the delayed-response test passes.
  - *Evidence to collect:* run the delayed-response test and re-run the O1 grep across the three crates — expect a passing test and no residual `reqwest::get`/`Client::new()` provider call.
  - *Status:* ☐ unverified

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `OidcProvider::exchange_code` / `revoke_token` still complete a successful POST via the shared client → expect the existing `oidc` success-path tests still pass : ☐ (PRESERVED / REGRESSION)
- `discovery::discover` and `jwks::JwksCache::get_keys` still fetch and parse a 200 body via the shared client → expect existing `discovery`/`jwks` tests still pass : ☐ (PRESERVED / REGRESSION)

## Residue

- Per-endpoint status/error handling (JWKS fail-closed, token-endpoint OAuth errors, discovery issuer) is out of scope here — it lands in tasks 02, 04, 05. Not obligations of Task 01.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
