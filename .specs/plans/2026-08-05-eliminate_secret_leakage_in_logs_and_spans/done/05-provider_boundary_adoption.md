# Task 05 — Route provider boundaries through the safe helper

**Status:** Done · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §05-provider-system](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#specsservicespecs05-provider-systemmd--oidcprovider-behaviour-modify); [§Implementation notes step 4](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`05-provider-system.md`](../../../service/specs/05-provider-system.md)
**Depends on:** 03, 04
**Produces:** all three upstream non-2xx paths use `read_bounded` and `upstream::error_detail`; OIDC credentials and Apple-generated assertions remain unprintable through request construction.
**Pointers:** `crates/adapters/src/shared/token_endpoint.rs`; `crates/adapters/src/oidc/mod.rs`; `crates/providers/src/apple.rs`; their existing token-exchange/revocation tests.

## Steps

- [x] Replace token-endpoint direct `response.text()`/raw-body fallback with the bounded reader and shared error-detail constructor, preserving its conformant OAuth-error test.
- [x] Convert OIDC revocation’s non-2xx body handling to the same pair; verify an upstream echo of the revoked token cannot appear in the returned internal detail/log path.
- [x] Change Apple `generate_client_secret` to return `Secret<String>`, expose only to build the outbound form, and route Apple revocation non-2xx handling through the shared pair.
- [x] Ensure configured OIDC `Option<Secret<String>>` is exposed only when building outbound forms and cannot be captured by traces/errors.
- [x] Add provider-boundary tests for raw and percent-encoded echoed token/client assertion/client secret/code values, structured OAuth errors, oversize response behavior, and existing success/revocation contracts.

## Task-specific definition of done

- [x] No `response.text()` remains on these three non-2xx paths, and no raw upstream body reaches `ProviderError.detail`.
- [x] The existing `exchange_code_surfaces_oauth_error_on_non_2xx` regression remains true for safe structured content.
- [x] OIDC and Apple revoke failures redact submitted token and credential sentinels, including encoded echo cases.
- [x] Apple assertion generation and all outbound forms compile with non-formatting types.
- [x] No certificate file is created; test output is the completion evidence.

**Evidence:** commit `feat(providers)` (task 05). `token_endpoint::exchange_code` now performs one `read_bounded` read for both outcomes; non-2xx detail comes only from `upstream::error_detail`; the success path unwraps the Secret at the JSON parse point (`TokenResponse` stays plain per the plan’s deferred-wrap note). OIDC `revoke_token` non-2xx reads bounded and renders `revocation: {error_detail}`. Apple `generate_client_secret -> Result<Secret<String>>`, exposed only inside outbound form construction in `exchange_code`/`revoke_token`; Apple revoke non-2xx uses the shared pair. No `response.text()` remains anywhere in the workspace. New tests: token-endpoint raw-form echo (code+client_secret sentinels), percent-encoded code echo, structured-error visibility + in-description echo masking, >64 KiB error body → bounded detail (≤512 chars incl. Display prefix), oversize success payload fails closed; OIDC revoke: 200 OK contract, no-endpoint no-op proven network-free via `received_requests`, echo of submitted token raw+encoded never in detail, structured error visible + masked; Apple revoke: two-phase test capturing the *actually generated* assertion from wiremock’s recorded request and echoing it back alongside raw+encoded token — neither appears in the detail; structured variant likewise. Existing `exchange_code_surfaces_oauth_error_on_non_2xx`, `revoke_token_posts_with_client_secret`, and all success/exchange contracts still pass. Focused: providers + adapters nextest — 176 passed, 28 skipped; workspace clippy `-D warnings` clean; `cargo fmt --all --check` clean.
