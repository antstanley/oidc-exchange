# Task 03 — Provider verification contract

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted at user request

**Implements:** [`.specs/changes/2026-08-05-bind_id_token_grant_replay_protection.md` §Proposed changes → `01-domain-model.md`, `02-ports-and-adapters.md`, and `05-provider-system.md`](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md), [§Type changes](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md), [§Implementation notes 3–4](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md); [01-domain-model.md §Token types](../../../service/specs/01-domain-model.md), [02-ports-and-adapters.md §IdentityProvider and §Mock adapters](../../../service/specs/02-ports-and-adapters.md), [05-provider-system.md §OidcProvider behaviour](../../../service/specs/05-provider-system.md), [canonical-types.schema.json `IdentityClaims`](../../../service/specs/canonical-types.schema.json)
**Depends on:** —
**Produces:** Core receives the configured provider client ID and the actual JWK algorithm used to verify every returned ID token.
**Pointers:** `crates/core/src/ports/identity_provider.rs:7-19`, `crates/core/src/domain/token.rs:72-84`, `crates/adapters/src/oidc/mod.rs:115-215`, `crates/providers/src/apple.rs:215-309`, `crates/test-utils/src/lib.rs:493-562`

## Steps

- [x] Add required `signing_alg` to `IdentityClaims` and the service canonical type schema, with documentation that it is the resolved JWK algorithm rather than the untrusted JWT header.
  - `crates/core/src/domain/token.rs`: required `signing_alg: String`, doc-commented as the resolved-JWK algorithm (never the header's) that selects the `at_hash` digest. Canonical schema folded: `.specs/service/specs/canonical-types.schema.json` `IdentityClaims` gains the `signing_alg` property (`$defs/NonEmptyString`) and it joins `required`.
- [x] Add `IdentityProvider::client_id()` and implement it in standard OIDC, Apple, and mock providers from the already-configured client identity.
  - Port method documented as "the audience this provider pins"; implemented from each adapter's existing configured field (OIDC `self.client_id`, Apple's Services ID, mock's new field with a builder override). No implementation reaches into provider config from core.
- [x] Populate `IdentityClaims.signing_alg` in both validators from the JWK algorithm actually selected or inferred during successful verification; do not alter their signature/issuer/audience validation policy.
  - Both validators report `shared::jwks::jws_alg_name(jwk_alg)` — the same resolved value their `Validation::new(jwk_alg)` verified with, whether the JWK declared `alg` explicitly or it was inferred from key material. Neither `Validation` block changed; a new shared exhaustive `Algorithm → JWS name` mapping lives in `adapters/src/shared/jwks.rs` because jsonwebtoken 10 has no `Display` for `Algorithm`.
- [x] Update mock fixtures and all constructors/callers for the expanded domain and port contracts.
  - Every `IdentityClaims` literal updated (mock defaults ×2, core exchange tests ×7, both validators); no shims. `MockIdentityProvider` gains a `client_id` field (default `"test-client-id"`, exported as `MOCK_CLIENT_ID`) plus a consuming `with_client_id` builder — a plain setter cannot work because the port hands out `&str` borrowed from the field.
- [x] Add focused standard-OIDC and Apple validation tests that demonstrate a returned claim reports the resolved verification algorithm, plus mock/provider trait coverage for `client_id()`.
  - OIDC: happy path asserts `RS256`; both alg-less inference tests assert the reported name comes from inference (`RS256` / `ES256`); a header-alg-vs-JWK mismatch is rejected (`InvalidGrant`); `client_id_returns_configured_audience`. Apple: flow test asserts `ES256`; `validate_id_token_reports_jwk_signing_algorithm`; the same header-mismatch negative against an RS256-pinned Apple JWKS; `client_id_returns_configured_services_id`. Mock: `mock_identity_provider_reports_configured_client_id` covers default, override, provider id, and default-claims `signing_alg`. Shared helper tests round-trip `jws_alg_name` against `Algorithm::from_str`.

## Definition of done

- [x] Every successful generic OIDC and Apple validation result has `signing_alg` equal to the JWK algorithm used for verification, not merely the JWT header value.
  - Positive assertions on explicit-alg and inferred-alg paths in both providers; negative space proves the header never wins: an ES256-header token naming an RS256-pinned JWK's kid is rejected by both validators.
- [x] Every `IdentityProvider` implementor, including test utility mocks, supplies the configured client ID through the port without reaching into provider configuration from core.
  - All three implementors return their pre-configured audience; core still holds no `[providers.*]` dependency.
- [x] `IdentityClaims` callers, canonical schema, and scoped canonical prose remain synchronized; no untrusted algorithm selection is introduced.
  - Prose folded into 01-domain-model (IdentityClaims bullet), 02-ports-and-adapters (port listing + `client_id` contract), and 05-provider-system (`validate_id_token` bullet + the "replay binding is above the provider boundary" decision).
- [x] Meets the repo definition of done (provider positive/negative tests, `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and applicable `cargo nextest` tests; record but do not fix the known unrelated three config-test baseline failures).
  - Baseline correction carried over from task 01: the plan's "three failing config tests" note is stale — merged PR #36 fixed them. This task ran green: 426 passed / 44 skipped, fmt + clippy (`--workspace --all-targets -D warnings`) clean.
- [x] Reviewable: a reviewer can inspect generic and Apple validator tests and see core-facing claims originate from resolved verification data.
  - The inference tests are the sharpest evidence: the JWK carries no `alg` at all, so a correct `signing_alg` can only have come from trusted key material.

## Notes

- Wave-B contract for task 04: read `claims.signing_alg` to pick the `at_hash` digest (SHA-256 for `*256`, SHA-384 for `*384`, SHA-512 for `*512`; EdDSA defines no digest → reject any `at_hash` on an EdDSA assertion), and compare `azp` against `provider.client_id()`. Both values now arrive as data on verified claims / the port.
- `jws_alg_name` is exhaustive over jsonwebtoken's `Algorithm` including HMAC variants the validators never select; its tests pin the mapping via `from_str` round-trip so a future enum variant cannot silently drift.
- Apple's validator accepts RS256 or ES256 JWKs by policy (Apple pins ES256 today); `signing_alg` reports whichever the JWK pinned, so task 04 needs no Apple-specific digest logic.
