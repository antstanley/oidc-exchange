# Task 04 — Assertion binding

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted at user request

**Implements:** [`.specs/changes/2026-08-05-bind_id_token_grant_replay_protection.md` §Proposed changes → `03-service-flows.md`](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md), [§Implementation notes 5–6](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md); [03-service-flows.md §Token exchange and §Audit emission and blocking](../../../service/specs/03-service-flows.md), [01-domain-model.md §Token types](../../../service/specs/01-domain-model.md)
**Depends on:** 01 (build), 02 (data), 03 (contract)
**Produces:** The core binds each verified ID token once: it enforces lifetime, `azp`, applicable `at_hash`, direct-grant nonce consumption, and assertion replay prevention before user lookup on both exchange paths.
**Pointers:** `crates/core/src/service/exchange.rs:13-94`, `crates/core/src/service/mod.rs`, new `crates/core/src/service/assertion.rs`, `crates/core/src/domain/token.rs:72-84`, `crates/core/tests/exchange.rs`, `crates/test-utils/src/lib.rs:493-562`

## Steps

- [ ] Add a focused assertion service module with minting and binding functions that use configured durations, cryptographic random bytes, SHA-256 digest keys, and the `SessionRepository` single-use API.
- [ ] Parse and validate `exp`, `aud`, `azp`, `at_hash`, `nonce`, and `jti` from verified `raw_claims`; select the `at_hash` digest from trusted `signing_alg`, reject unverifiable EdDSA `at_hash`, and use the compact-JWT digest fallback when `jti` is absent.
- [ ] Enforce the specified failure order: remaining-lifetime ceiling, `azp`, applicable `at_hash`, direct-grant nonce consumption, then assertion marker claim; return `InvalidGrant` and emit `ValidationFailed`/`Warning` audit details identifying the failed control.
- [ ] Extend `ExchangeRequest` with optional `provider_access_token`; carry provider access tokens from the authorization-code path into the same binding context, and call binding after either validation branch but before user lookup.
- [ ] Add deterministic core tests for direct and authorization-code behavior: valid first use, replay, missing/unissued/reused nonce, multi-audience and sibling-client `azp`, correct/mismatched/omitted-access-token `at_hash`, lifetime ceiling, no-`jti` fallback, and audit failure detail.

## Definition of done

- [ ] A direct assertion can only succeed with a minted nonce and then only once; missing, unknown, expired, or already-used nonce produces `InvalidGrant` without admitting replay.
- [ ] Both exchange paths run shared lifetime, `azp`, applicable `at_hash`, and assertion single-use controls exactly once before registration/user changes; code exchange does not require a nonce.
- [ ] Replay keys are provider-namespaced SHA-256 digests of `jti` or compact JWT with the `d:` discriminator, expire at assertion `exp`, and do not persist bearer inputs.
- [ ] Every binding rejection emits the specified validation audit classification/detail while store/audit failures remain propagated as typed failures.
- [ ] New functions use named bounds, meaningful assertions, bounded/simple control flow, and stay within the 70-line review gate by factoring helpers as needed.
- [ ] Meets the repo definition of done (core positive/negative/deterministic tests, `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and applicable `cargo nextest` tests; record but do not fix the known unrelated three config-test baseline failures).
- [ ] Reviewable: a reviewer can execute core tests proving a valid assertion succeeds once and every rejected binding condition fails before token issuance.
