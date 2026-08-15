# Task 02 — Add bounded provider transport

**Plan:** [plan.md](../plan.md) · **Status:** Backlog · **Depends on:** [01](01-prerequisite_contracts_and_guard_lint.md)

**Implements:** source `Shared OIDC utilities`; implementation notes 3–4; success-body ceiling tests.

**Scope:** Add `shared::transport::ProviderTransport` and its `UpstreamBody` abstraction around the process-wide provider client. Migrate discovery, JWKS, token exchange, OIDC revocation, and Apple revocation—five call sites—so no adapter directly issues provider HTTP. Integrate the externally confirmed bounded-read and safe error-detail helpers without reimplementing sibling-owned behavior.

## Steps

- [ ] Export the transport from `crates/adapters/src/shared/mod.rs`; use only the shared provider client with its existing 5s connect, 10s total, and no-redirect policy.
- [ ] Implement typed `get_json` and form-post operations that inspect status before reading a body; success parsing consumes the bounded body, while non-success uses the confirmed error-detail path.
- [ ] Migrate `shared/discovery.rs`, `shared/jwks.rs`, `shared/token_endpoint.rs`, `oidc/mod.rs` revocation, and `providers/src/apple.rs` revocation. Remove direct provider `reqwest` issuance from those call sites.
- [ ] Route discovery and JWKS success parsing through the shared ceiling; retain token endpoint's single already-bounded body read rather than reading twice.
- [ ] Add wiremock tests for over-limit bodies with both honest `Content-Length` and chunked transfer, distinctive cap errors, and no cache population after JWKS failure.

## Definition of done

- [ ] Repository scan finds no direct provider `reqwest` request outside `ProviderTransport`; webhook keeps its independently owned client.
- [ ] Both discovery and JWKS reject oversized success responses before JSON materialization; status is evaluated before any body read.
- [ ] A non-2xx provider response uses bounded/safe detail handling and remains an error.
- [ ] All five call sites and their existing success/error tests compile and pass; no done certificate is produced.
