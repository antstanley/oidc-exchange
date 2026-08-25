# Task 04 — Build shared verification key set

**Plan:** [plan.md](../plan.md) · **Status:** Done · **Depends on:** [01](01-prerequisite_contracts_and_guard_lint.md)

**Implements:** source `VerificationKeySet` design; implementation note 6; C11/C12 key corpus.

**Scope:** Create `shared::keys::{VerificationKeySet, VerificationKey}` as the sole JWK-to-verification-key conversion path. Replace both private `find_jwk` copies and per-provider algorithm matches without merging provider validators. Keep per-provider admitted algorithm sets and preserve missing-`alg` inference only where the source permits it.

*(04a — "Keep provider-specific admitted algorithm policies explicit" — is the
`#scope` anchor of this file and is satisfied in the same change; see the 04a
notes below.)*

## Steps

- [x] Translate the baseline corpus outcome into explicit admitted algorithm sets per OIDC and Apple; do not widen Apple to the OIDC union.
- [x] Implement constructor filtering: reject inappropriate `use`, missing `verify` in declared `key_ops`, unsupported/declared-outside-set algorithms, `alg`/`kty`/`crv` inconsistency, `oct`, `none`, malformed JWKS, and ambiguity failures as required by the source.
- [x] Handle absent `alg` with narrowed inference: RSA/approved EC and only `OKP` + `Ed25519`; distinguish absent from unknown declared algorithms.
- [x] Make lookup by `kid` order-independent when duplicate entries include ineligible and eligible keys; return an owned/shared verification value carrying the selected algorithm as data.
- [x] Migrate `OidcProvider` and `AppleProvider` to consume the key set and delete both selectors, both algorithm matches, and tautological selected-`kid` assertions.
- [x] Extend the shared corpus through both provider validation paths for every source-listed case and assert equal disposition; include a real `use: sig` success case.

## Definition of done

- [x] No production code outside `shared::keys` turns raw JWK JSON into a `DecodingKey` or re-derives verification algorithms.
- [x] OIDC and Apple retain their intentionally different admitted algorithm policies while agreeing on selection eligibility behavior (C12 evidence).
- [x] Positive, negative, duplicate-order, absent-vs-unknown-algorithm, and key-type compatibility paths are tested.
- [x] Public adapter visibility and `JwksCache::get_keys` compatibility impact are recorded for 07; no done certificate is produced.

## Notes (completion record)

**Where things live.**
`crates/adapters/src/shared/keys.rs`: `VerificationKeySet::from_jwks(provider,
&jwks, admitted)` + `VerificationKey { kid, algorithm, decoding_key }`
(`Arc`-shared; `set.get(kid)` clones a pointer, never deep key material).
`crates/adapters/src/shared/jwks.rs`: cache values are now
`Arc<VerificationKeySet>` (source implementation note 1's shape), constructed
with an admitted-algorithms parameter, and `JwksCache::get_key(kid)` encapsulates
the resolve → one-rate-limited-forced-refetch → re-resolve → fail-closed path
that previously lived duplicated in both providers.

**(04a) Explicit per-provider policies.** `OIDC_ADMITTED_ALGORITHMS` (nine JWS
signature algorithms — RS256/384/512, ES256/384, PS256/384/512, EdDSA) is a named
const in `crates/adapters/src/oidc/mod.rs`; `APPLE_ADMITTED_ALGORITHMS`
(`{RS256, ES256}`) is a named const in `crates/providers/src/apple.rs`. Neither is
derived from the other, no union exists anywhere, and a dedicated unit test
(`apple_shaped_policy_narrows_the_generic_nine`, plus
`inference_cannot_widen_an_admitted_set`) proves the parameterisation: the same
JWKS through different policies yields different sets, and inference output passes
the same admission check as declared algorithms so inference cannot widen policy.

**Constructor rulebook** (entry-level violations are dropped, document-level are
errors): drop when `use` present ≠ `"sig"`; drop when `key_ops` present without
`"verify"` (absent member stays permissive per RFC 7517 §4.2–4.3); drop `oct`;
drop unknown *declared* algorithms (`"RSA-OAEP"`, `"none"` — never treated as
absent, closing the pre-consolidation OIDC conflation); drop declared algorithms
inconsistent with `kty`/`crv`; narrow absent-`alg` inference to RSA→RS256,
EC P-256→ES256, EC P-384→ES384, OKP+Ed25519→EdDSA (the `(Some("OKP"), _)`
wildcard is narrowed to `Some("Ed25519")`, C11's disposition). Document errors:
missing/non-array `keys`, and two eligible entries sharing a `kid` (ambiguity →
whole-set error naming the contested kid).

**Migration.** Both providers call `self.jwks_cache.get_key(kid)` and configure
`Validation::new(verification_key.algorithm())` — algorithm as data from the key
set, never the token header, never a local match. Deleted: both private
`find_jwk` copies, `infer_alg_from_jwk`, OIDC's nine-arm match, Apple's two-arm
match, and both tautological selected-`kid` `assert_eq!`s (C11's reproduction
site — contradiction disposed of by typed construction, not answered).

**C12 evidence.** `crates/providers/tests/cross_provider_corpus.rs` now asserts
the post-consolidation table: all twelve source-listed cases have **equal**
dispositions on both provider paths (0 disagreements vs the baseline's 6), with
both `use: sig` non-regression cases verifying on both paths (RSA/RS256 on OIDC
and Apple; EC P-256/ES256 on Apple). The flip from the committed pre-consolidation
baseline is itself review evidence of deliberate behavior change.

**Recorded for task 07 (compatibility/docs sweep):**
1. `JwksCache::get_keys` returns `Arc<VerificationKeySet>` instead of
   `serde_json::Value`, and `JwksCache::new`/`with_ttl` gained an admitted-algorithms
   parameter — breaking for embedders using these directly (source §Compatibility
   anticipated exactly this; `JwksCache` remains `pub`). `IdentityProvider`'s trait
   signature is unchanged, so server/FFI surfaces needed no edits.
2. `shared::keys::{VerificationKeySet, VerificationKey}` are new public adapter
   types (internal adapter types per the source spec — no canonical schema entry).
3. Behavior deltas worth a release note: alg-less RSA/EC-P-256/OKP-Ed25519 keys
   now validate on Apple's path too (uniform narrowed inference); unknown declared
   algorithms (`RSA-OAEP`) no longer fall back to inference on the generic path;
   duplicate-kid JWKS documents whose eligible entry appears second now validate.

No done certificate produced.
