# Done Certificate — Task 05: Verify the discovered issuer matches the configured one

**Task:** [05-discovery_issuer_check.md](05-discovery_issuer_check.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 05. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 05) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `discovery::discover` rejects a discovery document whose `issuer` field does not equal the configured issuer (trailing slash normalised), per RFC 8414 §3.3.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing parse/return behaviour on a matching issuer (tests `discover_parses_openid_configuration`, `discover_handles_missing_optional_fields`, `discover_returns_error_on_invalid_json`, `discover_strips_trailing_slash_from_issuer_url`).

## Obligations

- **O1 — An issuer mismatch is rejected; a match still succeeds.**
  - *Claim:* after parsing, `discover` compares `doc.issuer` to `issuer_url` (trailing slash normalised the same way the fetch URL is built) and returns `Error::ProviderError` on mismatch, before returning the document.
  - *Evidence to collect:* read `crates/adapters/src/shared/discovery.rs:24-31`; confirm the comparison uses `trim_end_matches('/')` on both sides consistent with the URL build at `:16-19`. Run the new `wiremock` issuer-mismatch test — expect an error; run `discover_strips_trailing_slash_from_issuer_url` — expect still PASS.
  - *Checks:* trace the mismatch branch — confirm it returns the error before `Ok(doc)`.
  - *Status:* ☑ SATISFIED — `discovery.rs:39-47`: after parsing, `doc.issuer.trim_end_matches('/')` is compared to `normalised_issuer` (itself `issuer_url.trim_end_matches('/')` at `:21`, the same value used to build the fetch URL at `:22`); on mismatch `Error::ProviderError` is returned at `:40-46`, before `Ok(doc)` at `:55`. `discover_rejects_mismatched_issuer` → PASS (error observed); `discover_strips_trailing_slash_from_issuer_url` → PASS.

- **O2 — Negative-space test: mismatch yields a `ProviderError` naming both issuers.**
  - *Claim:* the mismatch error's detail names the configured and the discovered issuer.
  - *Evidence to collect:* run the issuer-mismatch test and inspect the error detail — expect both issuer strings present.
  - *Status:* ☑ SATISFIED — the detail is `"discovered issuer '{doc.issuer}' does not match configured issuer '{normalised_issuer}'"` (`discovery.rs:42-45`); the test asserts `detail.contains(&server.uri())` (configured) and `detail.contains("evil.example.com")` (discovered), plus `provider == server.uri()` — PASS.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, ≥2 assertions on `discover`.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-adapters` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` clean; `cargo clippy --workspace -- -D warnings` clean; `cargo nextest run -p oidc-exchange-adapters` 100/100 passed (full workspace: 251/251 passed). ≥2 assertions present: `assert!(!issuer_url.is_empty())` at `:19` and `assert_eq!` on the normalised issuer on the success path at `:49-53`.

- **O4 — Reviewable: an issuer-mismatch document is rejected while existing discovery tests stay green.**
  - *Claim:* a reviewer runs the mismatch test and the existing discovery suite and sees the mismatch rejected and the rest passing.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters shared::discovery` — expect the new mismatch test and the four existing tests all green.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange-adapters -E 'test(shared::discovery)'` → 5/5 PASS: `discover_rejects_mismatched_issuer` (new) plus the four existing tests (`discover_parses_openid_configuration`, `discover_handles_missing_optional_fields`, `discover_returns_error_on_invalid_json`, `discover_strips_trailing_slash_from_issuer_url`).

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `OidcProvider::from_config` / `AppleProvider::from_config` call `discovery::discover` to resolve endpoints → on a provider whose document's `issuer` matches, expect discovery still returns the endpoints unchanged : ☑ PRESERVED — `OidcProvider::from_config` (`crates/adapters/src/oidc/mod.rs:78`) calls `discover(&config.issuer)`; on a matching issuer the new check compares equal normalised strings and falls through to `Ok(doc)`, so `token_endpoint`/`jwks_uri`/`revocation_endpoint` resolve unchanged. Full workspace suite 251/251 PASS. (Note: `AppleProvider::from_config` does not actually call `discover` — Apple uses hard-coded well-known endpoints, `crates/providers/src/apple.rs:131-132` — so it is unaffected.)

## Residue

- Timeout wiring for the discovery GET is Task 01; this task assumes the shared client and only adds the issuer check.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O4 all SATISFIED with direct evidence (mismatch rejected with a ProviderError naming both issuers before Ok(doc), trailing-slash normalisation identical to the fetch-URL build, fmt/clippy/nextest all clean, 5/5 discovery tests green) and the sole downstream caller `OidcProvider::from_config` is PRESERVED.
