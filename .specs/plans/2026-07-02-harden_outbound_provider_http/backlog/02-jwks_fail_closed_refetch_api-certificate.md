# Done Certificate — Task 02: JWKS fail-closed caching and rate-limited refetch API

**Task:** [02-jwks_fail_closed_refetch_api.md](02-jwks_fail_closed_refetch_api.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** A non-2xx JWKS response becomes a `ProviderError` that is never cached, and `JwksCache` gains a forced-refetch path rate-limited to at most one network fetch per 30s.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing `get_keys` fast-path/slow-path TTL cache behaviour (`jwks.rs:42-69`) — a fresh 200 fetch still caches and subsequent calls within the TTL still serve from cache (tests `first_call_fetches_from_url`, `second_call_uses_cache`, `stale_cache_triggers_refresh`).

## Obligations

- **O1 — A non-2xx JWKS response fails closed and is never cached.**
  - *Claim:* `fetch_keys` checks `response.status()` before `.json()` and returns `Error::ProviderError` on a non-2xx status, before any write to the cache.
  - *Evidence to collect:* read `crates/adapters/src/shared/jwks.rs` `fetch_keys`; confirm a status check precedes `.json()` and that `get_keys` writes `*guard = Some(...)` only on the `Ok` path. Run the new `wiremock` 500 test — expect an error and that a following 200 call then caches (proving the error path left the cache empty).
  - *Checks:* trace `get_keys` on the error path — confirm `fetch_keys().await?` propagates the error before the `*guard = Some(CachedJwks { … })` write at `jwks.rs:64`.
  - *Status:* ☐ unverified

- **O2 — Forced refetch is rate-limited to one fetch per `MIN_REFRESH_INTERVAL` (30s).**
  - *Claim:* the forced-refetch path (`get_key`/`refresh`) issues at most one network fetch per 30s, guarded by a named constant with units in the identifier.
  - *Evidence to collect:* read `jwks.rs`; confirm a `MIN_REFRESH_INTERVAL` (30s) constant and a stored last-refetch `Instant` consulted before fetching. Run the rate-limit test — a second forced refetch within the interval makes no second request (a `wiremock` `expect(1)`).
  - *Checks:* confirm the interval guard compares against the last forced-refetch time, not the TTL `fetched_at`, so it is a distinct bound from `DEFAULT_TTL`.
  - *Status:* ☐ unverified

- **O3 — Negative-space test: rate limit holds and a 500 is not cached.**
  - *Claim:* a forced refetch within the interval makes no second request, and a 500 response never populates the cache.
  - *Evidence to collect:* run the two `wiremock` tests (500-not-cached; refetch-within-interval-no-second-call) — expect both PASS.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, the new bound named.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-adapters` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: the 500-not-cached and rate-limit tests pass.**
  - *Claim:* a reviewer can run the JWKS fail-closed and rate-limit tests and see both pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters shared::jwks` (or the equivalent filter) — expect the new tests green alongside the existing three.
  - *Status:* ☐ unverified

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `OidcProvider::validate_id_token` (`oidc/mod.rs:89`) and `AppleProvider::validate_id_token` (`apple.rs:217`) call `jwks_cache.get_keys()` → after the change, a successful 200 fetch still returns the key set and caches it : ☐ (PRESERVED / REGRESSION)
- Existing tests `first_call_fetches_from_url`, `second_call_uses_cache`, `stale_cache_triggers_refresh` → expect still passing : ☐ (PRESERVED / REGRESSION)

## Residue

- Wiring the forced-refetch API into the providers' `kid`-miss branches is Task 03, not this task. Task 02 only adds the API and the fail-closed check.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
