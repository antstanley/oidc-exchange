# Task 05 — Route provider boundaries through the safe helper

**Status:** Backlog · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §05-provider-system](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#specsservicespecs05-provider-systemmd--oidcprovider-behaviour-modify); [§Implementation notes step 4](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`05-provider-system.md`](../../../service/specs/05-provider-system.md)
**Depends on:** 03, 04
**Produces:** all three upstream non-2xx paths use `read_bounded` and `upstream::error_detail`; OIDC credentials and Apple-generated assertions remain unprintable through request construction.
**Pointers:** `crates/adapters/src/shared/token_endpoint.rs`; `crates/adapters/src/oidc/mod.rs`; `crates/providers/src/apple.rs`; their existing token-exchange/revocation tests.

## Steps

- [ ] Replace token-endpoint direct `response.text()`/raw-body fallback with the bounded reader and shared error-detail constructor, preserving its conformant OAuth-error test.
- [ ] Convert OIDC revocation’s non-2xx body handling to the same pair; verify an upstream echo of the revoked token cannot appear in the returned internal detail/log path.
- [ ] Change Apple `generate_client_secret` to return `Secret<String>`, expose only to build the outbound form, and route Apple revocation non-2xx handling through the shared pair.
- [ ] Ensure configured OIDC `Option<Secret<String>>` is exposed only when building outbound forms and cannot be captured by traces/errors.
- [ ] Add provider-boundary tests for raw and percent-encoded echoed token/client assertion/client secret/code values, structured OAuth errors, oversize response behavior, and existing success/revocation contracts.

## Task-specific definition of done

- [ ] No `response.text()` remains on these three non-2xx paths, and no raw upstream body reaches `ProviderError.detail`.
- [ ] The existing `exchange_code_surfaces_oauth_error_on_non_2xx` regression remains true for safe structured content.
- [ ] OIDC and Apple revoke failures redact submitted token and credential sentinels, including encoded echo cases.
- [ ] Apple assertion generation and all outbound forms compile with non-formatting types.
- [ ] No certificate file is created; test output is the completion evidence.
