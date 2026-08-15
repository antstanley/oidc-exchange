# Task 03 — Provider verification contract

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted at user request

**Implements:** [`.specs/changes/2026-08-05-bind_id_token_grant_replay_protection.md` §Proposed changes → `01-domain-model.md`, `02-ports-and-adapters.md`, and `05-provider-system.md`](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md), [§Type changes](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md), [§Implementation notes 3–4](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md); [01-domain-model.md §Token types](../../../service/specs/01-domain-model.md), [02-ports-and-adapters.md §IdentityProvider and §Mock adapters](../../../service/specs/02-ports-and-adapters.md), [05-provider-system.md §OidcProvider behaviour](../../../service/specs/05-provider-system.md), [canonical-types.schema.json `IdentityClaims`](../../../service/specs/canonical-types.schema.json)
**Depends on:** —
**Produces:** Core receives the configured provider client ID and the actual JWK algorithm used to verify every returned ID token.
**Pointers:** `crates/core/src/ports/identity_provider.rs:7-19`, `crates/core/src/domain/token.rs:72-84`, `crates/adapters/src/oidc/mod.rs:115-215`, `crates/providers/src/apple.rs:215-309`, `crates/test-utils/src/lib.rs:493-562`

## Steps

- [ ] Add required `signing_alg` to `IdentityClaims` and the service canonical type schema, with documentation that it is the resolved JWK algorithm rather than the untrusted JWT header.
- [ ] Add `IdentityProvider::client_id()` and implement it in standard OIDC, Apple, and mock providers from the already-configured client identity.
- [ ] Populate `IdentityClaims.signing_alg` in both validators from the JWK algorithm actually selected or inferred during successful verification; do not alter their signature/issuer/audience validation policy.
- [ ] Update mock fixtures and all constructors/callers for the expanded domain and port contracts.
- [ ] Add focused standard-OIDC and Apple validation tests that demonstrate a returned claim reports the resolved verification algorithm, plus mock/provider trait coverage for `client_id()`.

## Definition of done

- [ ] Every successful generic OIDC and Apple validation result has `signing_alg` equal to the JWK algorithm used for verification, not merely the JWT header value.
- [ ] Every `IdentityProvider` implementation, including test utility mocks, supplies the configured client ID through the port without reaching into provider configuration from core.
- [ ] `IdentityClaims` callers, canonical schema, and scoped canonical prose remain synchronized; no untrusted algorithm selection is introduced.
- [ ] Meets the repo definition of done (provider positive/negative tests, `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and applicable `cargo nextest` tests; record but do not fix the known unrelated three config-test baseline failures).
- [ ] Reviewable: a reviewer can inspect generic and Apple validator tests and see core-facing claims originate from resolved verification data.
