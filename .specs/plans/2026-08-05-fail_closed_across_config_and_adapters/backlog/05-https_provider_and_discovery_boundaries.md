# Task 05 — HTTPS provider and discovery boundaries

**Plan:** [plan.md](../plan.md)  
**Status:** Backlog  
**Implements:** [source spec](../../../changes/2026-08-05-fail_closed_across_config_and_adapters.md) → Configuration validation, Ports and adapters Shared OIDC utilities, Provider-system tiers, and Implementation notes 5/8; [ports and adapters canonical page](../../../service/specs/02-ports-and-adapters.md) → Shared OIDC utilities; [provider system canonical page](../../../service/specs/05-provider-system.md) → Tiers  
**Depends on:** 01  
**Produces:** provider and Apple endpoint construction from validated `HttpsUrl` values, and discovery that rejects non-success HTTP status before body parsing while retaining issuer equality validation.  
**Pointers:** `crates/core/src/domain/provider.rs`; `crates/server/src/bootstrap.rs`; `crates/adapters/src/oidc/mod.rs`; `crates/adapters/src/shared/discovery.rs`; `crates/providers/src/apple.rs`; `crates/adapters/src/webhook/mod.rs`; relevant adapter/provider tests.

## Steps

- [ ] Thread `HttpsUrl` from resolved configuration through generic OIDC provider configuration
  and webhook construction, avoiding `.to_string()`/reparse escape hatches at each consumer.
- [ ] Validate configured generic provider `issuer`, `jwks_uri`, `token_endpoint`, and
  `revocation_endpoint` as HTTPS and make discovered endpoints pass the same boundary before use.
- [ ] Make Apple optional endpoint overrides use the common typed URL constructor rather than a
  copy of scheme checks; preserve known Apple HTTPS defaults.
- [ ] Add a `response.status().is_success()` guard in discovery before `json()`, reporting a
  `ProviderError` that identifies issuer and status. Keep RFC 8414 issuer matching after a
  successful parse.
- [ ] Keep production without a loopback exception; adapt Wiremock fixtures only through the
  test-only seam established by Task 01.
- [ ] Add focused tests for `404` and `500` well-formed discovery documents, successful discovery,
  issuer mismatch, generic/Apple configured HTTP endpoint rejection, and valid HTTPS endpoint
  acceptance.

## Definition of done

- [ ] Configured and discovered provider endpoints cannot reach network consumers unless HTTPS.
- [ ] Apple overrides have the same constraint as generic providers without duplicated divergent
  validator logic.
- [ ] A non-success discovery response is rejected before parsing, including a syntactically valid
  JSON body; success still enforces issuer equality.
- [ ] Test-only HTTP support is not callable in production builds.
- [ ] Relevant core/adapters/providers tests and Rust format/lint results are reported.

## Sibling boundaries

- Do not broaden outbound HTTP policy, retry policy, or runtime topology: prior hardening and the
  runtime-parity sibling own those concerns.
