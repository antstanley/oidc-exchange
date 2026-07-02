# Done Certificate — Task 03: Refetch JWKS on an unknown kid in both providers

**Task:** [03-refetch_on_unknown_kid.md](03-refetch_on_unknown_kid.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** An unknown `kid` triggers exactly one rate-limited JWKS refetch in both `OidcProvider` and `AppleProvider` before the token is rejected, so a rotated signing key validates on the next login without a TTL wait.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the rest of `validate_id_token` in either provider — header decode, JWK→decoding-key build, algorithm-from-JWK selection, issuer/audience validation, and `sub` extraction (`oidc/mod.rs:82-167`, `apple.rs:210-290`) all still run once the `kid` resolves.

## Obligations

- **O1 — Unknown `kid` triggers one refetch then validates, in `OidcProvider`.**
  - *Claim:* at `oidc/mod.rs:103-108` the terminal `InvalidGrant` is replaced by a forced refetch (task 02 API) and a re-search; a token whose `kid` was absent then present validates.
  - *Evidence to collect:* read `oidc/mod.rs` around the `kid` lookup; confirm the miss path calls the refetch API and re-searches before rejecting. Run the new `wiremock` rotation test for `OidcProvider` — expect PASS (first key set omits the `kid`, second contains it, validation succeeds without a TTL sleep).
  - *Checks:* resolve the refetch call — confirm it is task 02's rate-limited API on `JwksCache`, not a fresh unbounded fetch.
  - *Status:* ☑ SATISFIED — `oidc/mod.rs:139-154`: the `kid` miss path (`find_jwk` → `None`) calls `self.jwks_cache.refresh().await?` then re-searches via `find_jwk` on `get_keys()`, rejecting with `InvalidGrant` only after the re-search misses. The call resolves to `JwksCache::refresh` at `crates/adapters/src/shared/jwks.rs:123` (field `jwks_cache: JwksCache` at `oidc/mod.rs:20`; `JwksCache::refresh` is guarded by `MIN_REFRESH_INTERVAL`, jwks.rs:132-143) — no shadowing, not an unbounded fetch. Test `oidc::tests::validate_id_token_refetches_jwks_on_unknown_kid_then_validates` → PASS (stale set omits the `kid`, refetched set contains it, validation succeeds with no TTL sleep; wiremock `expect(1)` on each mock proves exactly one initial fetch + one forced refetch).

- **O2 — Unknown `kid` triggers one refetch then validates, in `AppleProvider`.**
  - *Claim:* the same change is applied at `apple.rs:231-236`.
  - *Evidence to collect:* read `apple.rs` around the `kid` lookup; run the `AppleProvider` rotation `wiremock` test — expect PASS.
  - *Checks:* resolve the refetch call at `apple.rs` to the same `JwksCache` rate-limited API.
  - *Status:* ☑ SATISFIED — `apple.rs:241-256`: identical miss path — `find_jwk` → `None` → `self.jwks_cache.refresh().await?` → re-search → `InvalidGrant` only on a second miss. `jwks_cache` is `JwksCache` from `oidc_exchange_adapters::shared::jwks` (`apple.rs:9,35`), so `refresh()` resolves to the same rate-limited API at jwks.rs:123. Test `apple::tests::validate_id_token_refetches_jwks_on_unknown_kid_then_validates` → PASS (same rotation shape, ES256, no TTL sleep, `expect(1)` per mock).

- **O3 — Refetch is rate-limited; a still-missing `kid` is rejected without a loop.**
  - *Claim:* repeated unknown `kid`s do not each cause a network fetch (task 02's `MIN_REFRESH_INTERVAL` guard holds), and a `kid` still absent after the refetch yields `InvalidGrant` with no infinite loop.
  - *Evidence to collect:* run the negative-space test — a `kid` absent from both the cached and refetched set returns `InvalidGrant`; assert the JWKS endpoint received at most the rate-limit-permitted number of requests (`wiremock` `expect`).
  - *Status:* ☑ SATISFIED — `oidc::tests::validate_id_token_rejects_kid_still_missing_after_refetch` and `apple::tests::validate_id_token_rejects_kid_still_missing_after_refetch` both PASS. Each mounts the JWKS mock with `expect(2)` (initial fetch + exactly one forced refetch) and calls `validate_id_token` twice with the same unknown `kid`: both calls return `Err(InvalidGrant)` (no hang, no loop), and a third GET would have panicked the mock on drop — proving the second call's refetch was suppressed by `MIN_REFRESH_INTERVAL` (30s, jwks.rs:15) with no per-miss network fetch.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, ≥2 assertions per touched function.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-adapters -p oidc-exchange-providers` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` clean; `cargo clippy --workspace -- -D warnings` finished with no warnings; `cargo nextest run -p oidc-exchange-adapters -p oidc-exchange-providers` → 113 tests run, 113 passed (10 skipped). Both touched `validate_id_token` functions carry ≥2 assertions: `assert!(refreshed["keys"].is_array(), …)` and `assert_eq!(jwk["kid"].as_str(), Some(kid), …)` (`oidc/mod.rs:149-162`, `apple.rs:251-263`), matching the task's named examples. The rate limit uses the named constant `MIN_REFRESH_INTERVAL`.

- **O5 — Reviewable: a rotated key validates on the next call in both providers without waiting out the TTL.**
  - *Claim:* a reviewer can run the two rotation tests and see a rotated `kid` validate without a TTL sleep.
  - *Evidence to collect:* run the `OidcProvider` and `AppleProvider` `kid`-rotation tests — expect both green and neither sleeping for the TTL.
  - *Status:* ☑ SATISFIED — exercised: `cargo nextest run -p oidc-exchange-adapters -E 'test(refetches_jwks_on_unknown_kid) or test(rejects_kid_still_missing)'` → 2/2 PASS in ~1.0s each, and the Apple pair passed in ~0.9s each in the full run — no TTL sleep anywhere (test bodies contain no sleep; runtimes are far below any TTL). A rotated `kid` validates on the very next `validate_id_token` call in both providers.

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `oidc/mod.rs::validate_id_token` for a token whose `kid` is already in the cached set → expect it still validates in the first pass with no extra refetch : ☑ PRESERVED — trace: `find_jwk` returns `Some(jwk)` on the first pass, so the `None` arm (refresh + re-search) never runs; header decode, JWK→decoding-key build, alg-from-JWK selection, issuer/audience validation, and `sub` extraction are untouched downstream. All pre-existing happy-path tests (e.g. the single-mock `validate_id_token` tests in `oidc::tests`) pass, and their single-response wiremock setups would surface any extra fetch.
- `apple.rs::validate_id_token` for an already-present `kid` → expect unchanged single-fetch validation : ☑ PRESERVED — same trace; `apple::tests::exchange_and_validate_flow` and the coercion/rejection tests all pass unchanged in the 113/113 run.

## Residue

- The fail-closed status check and the refetch API itself belong to Task 02; this task only consumes them at the two call sites.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence in hand — both providers' `kid`-miss paths resolve to task 02's rate-limited `JwksCache::refresh` (jwks.rs:123), all four new rotation/negative-space wiremock tests pass with request budgets enforced by `expect(N)`, fmt/clippy/nextest are clean (113/113), and both named regression callers are PRESERVED.
