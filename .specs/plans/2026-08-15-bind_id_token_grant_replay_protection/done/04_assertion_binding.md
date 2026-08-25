# Task 04 — Assertion binding

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted at user request

**Implements:** [`.specs/changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md` §Proposed changes → `03-service-flows.md`](../../../changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md), [§Implementation notes 5–6](../../../changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md); [03-service-flows.md §Token exchange and §Audit emission and blocking](../../../service/specs/03-service-flows.md), [01-domain-model.md §Token types](../../../service/specs/01-domain-model.md)
**Depends on:** 01 (build), 02 (data), 03 (contract)
**Produces:** The core binds each verified ID token once: it enforces lifetime, `azp`, applicable `at_hash`, direct-grant nonce consumption, and assertion replay prevention before user lookup on both exchange paths.
**Pointers:** `crates/core/src/service/exchange.rs:13-94`, `crates/core/src/service/mod.rs`, new `crates/core/src/service/assertion.rs`, `crates/core/src/domain/token.rs:72-84`, `crates/core/tests/exchange.rs`, `crates/test-utils/src/lib.rs:493-562`

## Steps

- [x] Add a focused assertion service module with minting and binding functions that use configured durations, cryptographic random bytes, SHA-256 digest keys, and the `SessionRepository` single-use API.
  - `service::assertion` owns `AppService::mint_nonce` (32 random bytes → base64url-no-pad, only `nonce:<sha256hex>` stored, collision surfaced as `StoreError`) and the free `assertion::bind(session_repo, claims, ctx)` running the five controls in spec order. `AssertionContext` carries provider id/client id/access token/compact JWT/`require_nonce`/the pre-parsed lifetime ceiling; outcomes are typed as `AssertionBindError::{Rejected, Store}` so rejections and infrastructure failures cannot be conflated.
- [x] Parse and validate `exp`, `aud`, `azp`, `at_hash`, `nonce`, and `jti` from verified `raw_claims`; select the `at_hash` digest from trusted `signing_alg`, reject unverifiable EdDSA `at_hash`, and use the compact-JWT digest fallback when `jti` is absent.
  - Digest selection is suffix-driven (`*256`→SHA-256, `*384`→SHA-384, `*512`→SHA-512); EdDSA rejects any `at_hash` outright (before the access-token-presence skip, per the task-03 wave-B contract); unknown families fail closed when a token would otherwise bind. Empty-string `jti` falls through to the `d:`-discriminated compact-JWT digest. `at_hash` comparison is constant-time over the derived digests.
- [x] Enforce the specified failure order: remaining-lifetime ceiling, `azp`, applicable `at_hash`, direct-grant nonce consumption, then assertion marker claim; return `InvalidGrant` and emit `ValidationFailed`/`Warning` audit details identifying the failed control.
  - Nonce burns before the marker claim (anti-pin order). Each rejection audits `ValidationFailed`/`Warning`/`Failure` with `detail.check` ∈ {`lifetime_ceiling`, `azp`, `at_hash`, `nonce`, `single_use`} and a generic reason naming only the control; store failures propagate untouched (`Error::StoreError`) with no rejection audit.
- [x] Extend `ExchangeRequest` with optional `provider_access_token`; carry provider access tokens from the authorization-code path into the same binding context, and call binding after either validation branch but before user lookup.
  - The code path feeds `ProviderTokens.access_token` into the same slot; `exchange()` binds between step 2 (validated claims) and step 4 (user lookup) via `enforce_assertion_binding`, which maps rejections to `InvalidGrant` after auditing. The server's `TokenForm` gained the matching field and passes it through on both exchange grant types.
- [x] Add deterministic core tests for direct and authorization-code behavior: valid first use, replay, missing/unissued/reused nonce, multi-audience and sibling-client `azp`, correct/mismatched/omitted-access-token `at_hash`, lifetime ceiling, no-`jti` fallback, and audit failure detail.
  - `crates/core/tests/assertion.rs` (13 tests): once-only flow proving both rejection controls (burned-nonce replay vs fresh-nonce marker replay), exact marker keys/expiry via `get_single_use_record`, mint-shape/digest-only storage, missing/unissued/reused nonce negative space, lifetime boundary (2h refused / 50m accepted under a 1h ceiling), three `azp` cases plus foreign-`azp` single-aud, six `at_hash` branches including EdDSA with and without an access token and an unknown alg family, no-`jti` fallback key + discriminator non-collision, code-path binding without any nonce touch, and typed store-failure propagation for armed take/put failures.

## Definition of done

- [x] A direct assertion can only succeed with a minted nonce and then only once; missing, unknown, expired, or already-used nonce produces `InvalidGrant` without admitting replay.
  - `take_single_use` makes absent/burned/expired indistinguishable; rejected bindings pin no marker (asserted in the negative-space tests).
- [x] Both exchange paths run shared lifetime, `azp`, applicable `at_hash`, and assertion single-use controls exactly once before registration/user changes; code exchange does not require a nonce.
  - Single call site after either validation branch; the code-path test proves an unrelated live nonce survives two code exchanges untouched.
- [x] Replay keys are provider-namespaced SHA-256 digests of `jti` or compact JWT with the `d:` discriminator, expire at assertion `exp`, and do not persist bearer inputs.
  - Keys asserted verbatim (`assertion:<provider>:<sha256hex(jti)>`, `…:d:<sha256hex(jwt)>`); raw jti/JWT/nonce values never appear in stored keys; marker expiry equals the assertion's own `exp`.
- [x] Every binding rejection emits the specified validation audit classification/detail while store/audit failures remain propagated as typed failures.
  - Audit assertions check event type, severity, outcome, and `detail.check`; the decorator tests prove `StoreError` propagation with an empty rejection trail and preserved/consumed nonce per the burn order.
- [x] New functions use named bounds, meaningful assertions, bounded/simple control flow, and stay within the 70-line review gate by factoring helpers as needed.
  - Named constants (`NONCE_BYTES`, `NONCE_B64URL_LEN`, key prefixes, `d:` discriminator, `EdDSA`, check names); every helper ≤70 lines; assertions in production functions are postcondition/invariant checks, never on adversarial input.
- [x] Meets the repo definition of done (core positive/negative/deterministic tests, `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and applicable `cargo nextest` tests; record but do not fix the known unrelated three config-test baseline failures).
  - Baseline correction carried from tasks 01–03: the "three failing config tests" note is stale (fixed by merged PR #36). At this commit the workspace ran green: **445 passed / 44 skipped**, fmt + clippy (`--workspace --all-targets -D warnings`) clean. Baseline before this task: 426 passed / 44 skipped.
- [x] Reviewable: a reviewer can execute core tests proving a valid assertion succeeds once and every rejected binding condition fails before token issuance.
  - `cargo nextest run -p oidc-exchange-core --test assertion`.

## Notes

- Mock default claims changed shape to keep existing suites honest: `MockIdentityProvider::validate_id_token` now builds fresh defaults per call (`MOCK_DEFAULT_ASSERTION_TTL_SECS = 600` expiry, unique per-call `jti`, echoed `sub`) because binding runs unconditionally — frozen empty `raw_claims` fixtures would have failed the lifetime control, and a static `jti` would have made every second legitimate exchange a false replay. Tests that pin explicit claims are unaffected.
- `ExchangeRequest` gained `#[derive(Clone)]` so tests can replay identical requests; fields remain bearer inputs with no `Debug` exposure.
- Direct-grant replays of byte-identical requests die at the *nonce* control first (it was burned on first use); the *single-use* marker control fires when a fresh nonce wraps an already-spent assertion. Both orders are asserted explicitly in `direct_grant_succeeds_once_then_rejects_replay`.
- Wave-C contract for task 05: mount `POST /nonce` only when `grants.id_token` is true; gate any request carrying an `id_token` field up front in the handler when disabled; make discovery's `grant_types_supported` conditional. Core binding itself is switch-independent by design (the switch gates exposure, not correctness).
