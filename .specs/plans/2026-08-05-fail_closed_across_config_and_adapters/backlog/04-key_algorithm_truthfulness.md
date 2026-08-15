# Task 04 — Key algorithm truthfulness

**Plan:** [plan.md](../plan.md)  
**Status:** Backlog  
**Implements:** [source spec](../../../changes/2026-08-05-fail_closed_across_config_and_adapters.md) → Validation at load, Ports and adapters KeyManager, Implementation notes 3–4, Compatibility, and Decisions; [ports and adapters canonical page](../../../service/specs/02-ports-and-adapters.md) → KeyManager  
**Depends on:** 01  
**Produces:** local/KMS algorithm declarations validated as JWS names and cross-checked against adapter capability/key material; `KeyManager::algorithm()` and published JWK/discovery metadata report the actual algorithm.  
**Pointers:** `crates/adapters/src/local_keys/mod.rs`; `crates/adapters/src/kms/mod.rs`; `crates/server/src/bootstrap.rs`; `crates/core/src/config.rs`; `examples/aws-web/config/oidc-exchange.toml`; `examples/ecs-fargate/config/fargate.toml`; `docs/architecture/adapters.md`; `docs/guides/configuration.md`; `docs/deployment/aws-lambda.md`.

## Steps

- [ ] Make `SigningAlgorithm` a closed JWS vocabulary with adapter-specific acceptance: local
  accepts only `EdDSA`; KMS accepts only the documented RS/PS/ES JWS names. Reject AWS
  `SigningAlgorithmSpec` vocabulary during resolution with a field-named `ConfigError`.
- [ ] Change local-key construction so Ed25519 parsing derives `EdDSA`, rejects a mismatching
  declared value, and never stores/publishes an operator label as the algorithm.
- [ ] Cross-check KMS declaration against what can be truthfully established by the adapter and
  ensure `algorithm()`, JWT header, JWK, and discovery metadata share the derived value. Resolve
  the source-spec `GetPublicKey` startup question explicitly: implement the check or document a
  reviewed, tested exception; never represent a gap as validation.
- [ ] Update bootstrap/use sites to consume typed algorithms without re-parsing strings or
  deferring invalid KMS configuration until the first signing request.
- [ ] Add unit/integration tests for local Ed25519 declared as `ES256` rejection, valid EdDSA,
  invalid KMS JWS/AWS vocabulary rejection at config load, KMS mapping coverage, and metadata
  consistency across `algorithm()`, JWK, and discovery.
- [ ] Update KMS reference examples and the three source-spec-named documentation pages from
  `ECDSA_SHA_256`/`ECDSA_SHA256` to `ES256`; Task 08 reconciles the broader canonical/doc set.

## Definition of done

- [ ] No local key can be initialized with a label different from the Ed25519-derived `EdDSA`.
- [ ] KMS config accepts exactly its JWS domain and fails before request handling for any AWS
  vocabulary or unknown value.
- [ ] Published signing metadata is derived/truthful, not a copied config string, and tests cover
  both rejected mismatch and valid metadata.
- [ ] The startup-KMS-key-material decision is explicit and evidence-backed; no silent exception
  remains.
- [ ] Updated examples/docs and focused adapters/server tests are reported with Rust format/lint
  checks.

## Sibling boundaries

- Do not alter KMS signature encoding/JWK conversion work already covered by the merged KMS
  change; this task only controls config truthfulness and metadata derivation.
