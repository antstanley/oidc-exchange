# Done Certificate — Task 04: Surface token-endpoint OAuth errors and require id_token

**Task:** [04-token_endpoint_errors.md](04-token_endpoint_errors.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 04. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `token_endpoint::exchange_code` surfaces a non-2xx response's OAuth `error`/`error_description` as a `ProviderError`, and errors on a 2xx missing `id_token` instead of defaulting it to the empty string.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the success path — a 2xx body carrying `id_token` (and optionally `access_token`/`refresh_token`) still parses into `ProviderTokens` (tests `exchange_code_sends_correct_form_and_parses_response`, `exchange_code_without_client_secret`, `exchange_code_handles_missing_optional_tokens`).

## Obligations

- **O1 — A non-2xx response surfaces the OAuth error.**
  - *Claim:* on a non-2xx status, `exchange_code` parses `{"error","error_description"}` and returns `Error::ProviderError` naming both (raw body when not a JSON OAuth error).
  - *Evidence to collect:* read `crates/adapters/src/shared/token_endpoint.rs:23-42`; confirm a `response.status()` check precedes the success parse and that the error branch reads and reports `error`/`error_description`. Run the new `wiremock` `400 invalid_grant` test — expect an error whose detail contains `invalid_grant` and the description.
  - *Checks:* trace the status branch — confirm a non-2xx path returns before constructing `ProviderTokens`, so `validate_id_token` never sees an empty `id_token` from this call.
  - *Status:* ☑ SATISFIED — `token_endpoint.rs:38-59`: `response.status()` is captured before any body parse; the `!status.is_success()` branch parses `{"error","error_description"}` (falling back to the raw body when not JSON OAuth) and returns `Err(Error::ProviderError { detail })` at `:55-58`, before `ProviderTokens` is ever constructed. Test `exchange_code_surfaces_oauth_error_on_non_2xx` (wiremock 400 `invalid_grant`) → PASS; the error message contains both `invalid_grant` and the description.

- **O2 — A 2xx response without `id_token` is an error.**
  - *Claim:* the `unwrap_or_default()` at `token_endpoint.rs:39` is gone; a 2xx JSON lacking `id_token` returns an error.
  - *Evidence to collect:* read the success-path construction of `ProviderTokens`; confirm `id_token` is required (no `unwrap_or_default`). Run the new `wiremock` 200-without-`id_token` test — expect an error.
  - *Status:* ☑ SATISFIED — `unwrap_or_default()` is gone; `token_endpoint.rs:67-74` requires `id_token` via `.as_str().filter(|s| !s.is_empty()).ok_or_else(ProviderError "token endpoint response is missing id_token")`. Test `exchange_code_rejects_2xx_response_missing_id_token` → PASS (error is `Error::ProviderError`).

- **O3 — Negative-space tests cover both new rejection paths.**
  - *Claim:* a non-2xx OAuth error and a 2xx missing `id_token` each have a test.
  - *Evidence to collect:* run the two new tests plus the three existing success-path tests — expect all PASS (the existing three all carry an `id_token`).
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange-adapters -E 'test(token_endpoint)'`: 6/6 PASS, including both new rejection tests (`exchange_code_surfaces_oauth_error_on_non_2xx`, `exchange_code_rejects_2xx_response_missing_id_token`) and the three success-path tests (`exchange_code_sends_correct_form_and_parses_response`, `exchange_code_without_client_secret`, `exchange_code_handles_missing_optional_tokens`).

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, ≥2 assertions on `exchange_code`.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-adapters` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` clean; `cargo clippy --workspace -- -D warnings` finished with no warnings; `cargo nextest run -p oidc-exchange-adapters` 99/99 PASS. Two assertions on `exchange_code`: `token_endpoint.rs:12-15` (endpoint non-empty) and `:75-78` (id_token non-empty on the success path).

- **O5 — Reviewable: a `400 invalid_grant` surfaces `invalid_grant`, not a JWT-header message.**
  - *Claim:* a reviewer runs the token-endpoint error test and sees `invalid_grant` in the error rather than "Invalid JWT header".
  - *Evidence to collect:* run the `400 invalid_grant` test and inspect the returned error's detail — expect it names `invalid_grant`.
  - *Status:* ☑ SATISFIED — ran `exchange_code_surfaces_oauth_error_on_non_2xx`: its assertions inspect `err.to_string()` and require both `invalid_grant` and "the authorization code has expired" in the message — PASS. The non-2xx branch returns before any token parsing, so no JWT-header message can be produced for this input.

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `OidcProvider::exchange_code` (`oidc/mod.rs`) delegates to `token_endpoint::exchange_code` → on a valid 2xx with `id_token`, expect it still returns `ProviderTokens` unchanged : ☑ PRESERVED — `oidc/mod.rs:116-117` delegates to `crate::shared::token_endpoint::exchange_code` (resolved: the module function under review, no shadow); test `oidc::tests::exchange_code_returns_provider_tokens` PASS.

## Residue

- Timeout wiring for this POST is Task 01; this task assumes the shared client is already in use and only changes response handling.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence (code read at `token_endpoint.rs:12-84`, 6/6 targeted tests PASS, fmt/clippy clean, 99/99 adapters tests PASS) and the `OidcProvider::exchange_code` downstream caller is PRESERVED.
