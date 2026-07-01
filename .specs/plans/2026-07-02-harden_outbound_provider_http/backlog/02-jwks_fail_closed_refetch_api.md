# Task 02 — JWKS fail-closed caching and rate-limited refetch API

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-jwks_fail_closed_refetch_api-certificate.md](02-jwks_fail_closed_refetch_api-certificate.md)

**Implements:** [`02-ports-and-adapters.md` §Shared OIDC utilities](../../../service/specs/02-ports-and-adapters.md) (`jwks::JwksCache` — non-2xx is a `ProviderError` and never cached; a forced refetch rate-limited by a 30s minimum refresh interval)
**Depends on:** 01
**Produces:** a non-2xx JWKS response is a `ProviderError` and is never written to the cache; `JwksCache` exposes a forced-refetch path (e.g. `get_key(kid)` / `refresh()`) guarded by a 30s minimum refresh interval so a caller can force at most one refetch per interval
**Pointers:** `crates/adapters/src/shared/jwks.rs` — `fetch_keys` (`jwks.rs:71-83`), `get_keys` (`jwks.rs:42-69`), `CachedJwks` struct (`jwks.rs:17-20`), `DEFAULT_TTL` (`jwks.rs:8`)

## Steps

- [ ] In `fetch_keys`, check `response.status()` before `.json()`; on a non-2xx status return `Error::ProviderError` (naming the status) and return before any cache write, so an error body is never stored.
- [ ] Add a `MIN_REFRESH_INTERVAL` named constant (30s, units in the identifier) and track the last forced-refetch `Instant` on the cache (extend `CachedJwks` or add a field) to enforce it.
- [ ] Add a forced-refetch API — `get_key(kid)` that returns the matching JWK, refetching once when the `kid` is absent and the interval has elapsed, or a `refresh()` that callers invoke on a `kid` miss — bounded to at most one network fetch per `MIN_REFRESH_INTERVAL`.
- [ ] Add ≥2 assertions to each touched function (e.g. assert the refetch interval is non-zero; assert the fetched value has the expected shape before caching).
- [ ] Add `wiremock` tests: a 500 response returns an error and does not populate the cache (a subsequent success then caches); a forced refetch inside the interval does not issue a second network call.

## Definition of done

- [ ] A non-2xx JWKS response returns `Error::ProviderError` and leaves the cache unpopulated (fail-closed) — verified by a `wiremock` 500 test asserting no cached key set.
- [ ] The forced-refetch path issues at most one network fetch per `MIN_REFRESH_INTERVAL` (30s), which is a named constant with units in the identifier.
- [ ] Negative-space test: a forced refetch within the interval makes no second request (rate limit holds); a 500 is not cached.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the JWKS 500-not-cached test and the rate-limit test and confirms both pass.
