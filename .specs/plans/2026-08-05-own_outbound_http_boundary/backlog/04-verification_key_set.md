# Task 04 — Build shared verification key set

**Plan:** [plan.md](../plan.md) · **Status:** Backlog · **Depends on:** [01](01-prerequisite_contracts_and_guard_lint.md)

**Implements:** source `VerificationKeySet` design; implementation note 6; C11/C12 key corpus.

**Scope:** Create `shared::keys::{VerificationKeySet, VerificationKey}` as the sole JWK-to-verification-key conversion path. Replace both private `find_jwk` copies and per-provider algorithm matches without merging provider validators. Keep per-provider admitted algorithm sets and preserve missing-`alg` inference only where the source permits it.

## Steps

- [ ] Translate the baseline corpus outcome into explicit admitted algorithm sets per OIDC and Apple; do not widen Apple to the OIDC union.
- [ ] Implement constructor filtering: reject inappropriate `use`, missing `verify` in declared `key_ops`, unsupported/declared-outside-set algorithms, `alg`/`kty`/`crv` inconsistency, `oct`, `none`, malformed JWKS, and ambiguity failures as required by the source.
- [ ] Handle absent `alg` with narrowed inference: RSA/approved EC and only `OKP` + `Ed25519`; distinguish absent from unknown declared algorithms.
- [ ] Make lookup by `kid` order-independent when duplicate entries include ineligible and eligible keys; return an owned/shared verification value carrying the selected algorithm as data.
- [ ] Migrate `OidcProvider` and `AppleProvider` to consume the key set and delete both selectors, both algorithm matches, and tautological selected-`kid` assertions.
- [ ] Extend the shared corpus through both provider validation paths for every source-listed case and assert equal disposition; include a real `use: sig` success case.

## Definition of done

- [ ] No production code outside `shared::keys` turns raw JWK JSON into a `DecodingKey` or re-derives verification algorithms.
- [ ] OIDC and Apple retain their intentionally different admitted algorithm policies while agreeing on selection eligibility behavior (C12 evidence).
- [ ] Positive, negative, duplicate-order, absent-vs-unknown-algorithm, and key-type compatibility paths are tested.
- [ ] Public adapter visibility and `JwksCache::get_keys` compatibility impact are recorded for 07; no done certificate is produced.
