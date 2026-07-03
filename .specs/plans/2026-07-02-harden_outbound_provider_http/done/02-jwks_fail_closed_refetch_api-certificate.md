# Done Certificate — Task 02: JWKS fail-closed caching and rate-limited refetch API

**Task:** [02-jwks_fail_closed_refetch_api.md](02-jwks_fail_closed_refetch_api.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — `fetch_keys` (`jwks.rs:179-185`) checks `response.status()` and returns `Error::ProviderError` (naming the status) before `.json()`; in `get_keys` the only cache write is `*guard = Some(CachedJwks { … })` at `jwks.rs:76`, after `self.fetch_keys().await?` at `jwks.rs:75`, so the error propagates before any write (line shifted from the authored `:64` by the new constant/field). `refresh()` likewise writes the cache (`jwks.rs:161-165`) only after `fetch_keys().await?` succeeds. Test `non_2xx_response_is_error_and_leaves_cache_unpopulated` — PASS: 500 → `Error::ProviderError` whose message contains "500", direct assertion `cache.cache.read().await.is_none()`, then a 200 caches (`is_some()`).

- **O2 — Forced refetch is rate-limited to one fetch per `MIN_REFRESH_INTERVAL` (30s).**
  - *Claim:* the forced-refetch path (`get_key`/`refresh`) issues at most one network fetch per 30s, guarded by a named constant with units in the identifier.
  - *Evidence to collect:* read `jwks.rs`; confirm a `MIN_REFRESH_INTERVAL` (30s) constant and a stored last-refetch `Instant` consulted before fetching. Run the rate-limit test — a second forced refetch within the interval makes no second request (a `wiremock` `expect(1)`).
  - *Checks:* confirm the interval guard compares against the last forced-refetch time, not the TTL `fetched_at`, so it is a distinct bound from `DEFAULT_TTL`.
  - *Status:* ☑ SATISFIED — `MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(30)` at `jwks.rs:15`; a dedicated `last_forced_refetch: Arc<RwLock<Option<Instant>>>` field (`jwks.rs:24`) is consulted in `refresh()` before fetching (read-lock check at `jwks.rs:129-136` plus a double-check under the write lock at `jwks.rs:141-145`), comparing `last.elapsed() < MIN_REFRESH_INTERVAL` against the forced-refetch Instant — not `fetched_at` — so it is a distinct bound from `DEFAULT_TTL`. The timestamp is recorded before the network call, so even failed fetches are rate-limited. Tests `forced_refetch_within_interval_makes_no_second_request` and `get_key_forces_one_refetch_on_unknown_kid_then_rate_limits` (wiremock `expect(1)`/`expect(2)`) — PASS. Note: the identifier carries no literal unit suffix (task step itself prescribed the name `MIN_REFRESH_INTERVAL`); units are unambiguous via the typed `Duration`, `from_secs(30)`, and the doc comment naming 30 seconds, matching the file's existing `DEFAULT_TTL` convention.

- **O3 — Negative-space test: rate limit holds and a 500 is not cached.**
  - *Claim:* a forced refetch within the interval makes no second request, and a 500 response never populates the cache.
  - *Evidence to collect:* run the two `wiremock` tests (500-not-cached; refetch-within-interval-no-second-call) — expect both PASS.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange-adapters`: `non_2xx_response_is_error_and_leaves_cache_unpopulated` PASS and `forced_refetch_within_interval_makes_no_second_request` PASS; a third negative-space test `failing_upstream_still_rate_limits_forced_refetch` (500 upstream is still hit at most once per interval and never cached) also PASS.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, the new bound named.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-adapters` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` clean (exit 0); `cargo clippy --workspace -- -D warnings` finished with no warnings; `cargo nextest run -p oidc-exchange-adapters` 95 passed / 0 failed (10 skipped); full `cargo nextest run --workspace` 244 passed / 0 failed. The new 30s bound is the named constant `MIN_REFRESH_INTERVAL`.

- **O5 — Reviewable: the 500-not-cached and rate-limit tests pass.**
  - *Claim:* a reviewer can run the JWKS fail-closed and rate-limit tests and see both pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters shared::jwks` (or the equivalent filter) — expect the new tests green alongside the existing three.
  - *Status:* ☑ SATISFIED — exercised: the adapters run shows all 8 `shared::jwks::tests` green — the existing three (`first_call_fetches_from_url`, `second_call_uses_cache`, `stale_cache_triggers_refresh`) plus the five new ones (`non_2xx_response_is_error_and_leaves_cache_unpopulated`, `forced_refetch_within_interval_makes_no_second_request`, `failing_upstream_still_rate_limits_forced_refetch`, `get_key_returns_matching_key_without_refetch_when_cached`, `get_key_forces_one_refetch_on_unknown_kid_then_rate_limits`).

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `OidcProvider::validate_id_token` (`oidc/mod.rs:89`) and `AppleProvider::validate_id_token` (`apple.rs:217`) call `jwks_cache.get_keys()` → after the change, a successful 200 fetch still returns the key set and caches it : ☑ PRESERVED — both callers resolved: `crates/adapters/src/oidc/mod.rs:112` and `crates/providers/src/apple.rs:218` call `self.jwks_cache.get_keys().await?`; `get_keys`'s fast-path/slow-path TTL logic (`jwks.rs:54-81`) is unchanged, and `fetch_keys` on a 200 JSON-object response behaves identically (the new status and shape checks only reject non-2xx / non-object responses), so a successful fetch still returns and caches the key set. Full-workspace suite (244 tests, including the providers crate) all pass.
- Existing tests `first_call_fetches_from_url`, `second_call_uses_cache`, `stale_cache_triggers_refresh` → expect still passing : ☑ PRESERVED — all three PASS in `cargo nextest run -p oidc-exchange-adapters`.

## Residue

- Wiring the forced-refetch API into the providers' `kid`-miss branches is Task 03, not this task. Task 02 only adds the API and the fail-closed check.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with collected evidence — the status check precedes `.json()` and every cache write follows a successful fetch (fail-closed, proven by the 500 test's direct cache assertion), the forced-refetch path (`get_key`/`refresh`) is bounded by the dedicated `last_forced_refetch` Instant against `MIN_REFRESH_INTERVAL` (30s) distinct from the TTL, all five new wiremock tests and the three pre-existing ones pass, fmt/clippy/nextest are clean workspace-wide, and both downstream `get_keys()` callers (`oidc/mod.rs:112`, `providers/src/apple.rs:218`) are PRESERVED. Minor note: the constant name carries no unit suffix (the task step itself prescribed `MIN_REFRESH_INTERVAL`); the typed `Duration` and doc comment make the 30s bound unambiguous.
