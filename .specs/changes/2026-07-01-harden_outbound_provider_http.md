# Change: Harden outbound provider HTTP (timeouts, status checks, key rotation)

**Status:** Proposed · **Date:** 2026-07-01 · **Owner:** Ant Stanley · **Target:** crates/adapters (shared), crates/providers

Give every outbound provider call (JWKS, discovery, token endpoint, revocation) a shared
`reqwest::Client` with constant connect/total timeouts and redirects disabled; check
HTTP status before parsing bodies; refetch the JWKS on an unknown `kid` (rate-limited) so
upstream key rotation does not cause up to an hour of rejected logins; surface OAuth error
bodies from the token endpoint instead of a misleading "Invalid JWT header"; and verify the
discovered `issuer` matches the configured one (RFC 8414).

---

## Motivation

Every outbound call builds a fresh client with reqwest defaults — no timeouts, 10-redirect
policy (`crates/adapters/src/shared/jwks.rs:72`, `shared/discovery.rs:20`,
`shared/token_endpoint.rs:12`, `oidc/mod.rs:175`, `providers/src/apple.rs:300`). A hung
provider stalls `/token` indefinitely. `JwksCache::fetch_keys` never checks the status, so a
4xx/5xx JSON body is cached as the key set for the full 1h TTL; and an unknown `kid` never
triggers a refetch, so key rotation rejects logins until the TTL expires — the canonical
spec's assumption that the TTL alone "picks up upstream key rotation" is fragile.

`shared/token_endpoint.rs:23-42` likewise never checks the status and defaults a missing
`id_token` to the empty string, so a `400 {"error":"invalid_grant"}` surfaces downstream as
"Invalid JWT header" from `validate_id_token`. And `shared/discovery.rs:15-32` never checks
the discovered `issuer` against the configured one, which RFC 8414 §3.3 requires.

---

## Affected spec pages

| Canonical page                                                                               | Nature of change                                                                                                                                                                 |
| -------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | Rewrite the `Shared OIDC utilities` section: shared HTTP client with timeouts, JWKS status check + refetch-on-unknown-kid, discovery issuer check, token-endpoint error handling |
| [`.specs/service/specs/05-provider-system.md`](../service/specs/05-provider-system.md)       | Replace the key-rotation Assumption with the refetch-on-unknown-kid behaviour; note the shared client                                                                            |

Companion change spec touching the same validation paths (spec page 05):
[2026-07-01-require_iss_aud_in_token_validation.md](2026-07-01-require_iss_aud_in_token_validation.md).

---

## Proposed changes

### `.specs/service/specs/02-ports-and-adapters.md` → Shared OIDC utilities (Modify)

> Reused by the OIDC and Apple providers. All outbound HTTP goes through a single shared
> `reqwest::Client` per process with a 5s connect timeout, a 10s total request timeout
> (compile-time constants, not configuration), and redirects disabled; a hung or slow
> provider fails the request rather than stalling `/token`. Non-2xx responses are errors at
> this layer — bodies are only parsed on success.
>
> - `jwks::JwksCache` — fetches and caches a remote JWKS behind a read/write lock with a TTL
>   (default 1h); `with_ttl` overrides. A non-2xx JWKS response is a `ProviderError` and is
>   never cached. When a token's `kid` is not in the cached set, the cache refetches once
>   (rate-limited by a 30s minimum refresh interval) before the provider rejects the token, so
>   upstream key rotation takes effect immediately instead of at TTL expiry.
> - `discovery::discover(issuer)` — fetches and parses `.well-known/openid-configuration`
>   into `DiscoveryDocument { issuer, token_endpoint, jwks_uri, revocation_endpoint }` and
>   errors if the document's `issuer` does not equal the configured issuer (RFC 8414 §3.3).
> - `token_endpoint::exchange_code(endpoint, client_id, client_secret, code, redirect_uri)`
>   — the standard form-encoded `grant_type=authorization_code` POST. A non-2xx response is
>   parsed as an OAuth error body (`error`, `error_description`) and propagated as a
>   `ProviderError` naming both; a 2xx response without an `id_token` is an error, not an
>   empty string.

### `.specs/service/specs/05-provider-system.md` → OidcProvider behaviour (Modify)

> - All outbound calls (discovery, JWKS, token endpoint, revocation) use the shared
>   timed-out HTTP client from `adapters/shared`; a JWKS `kid` miss triggers one
>   rate-limited refetch before the token is rejected.

### `.specs/service/specs/05-provider-system.md` → Assumptions (Modify)

> - Upstream key rotation is picked up by the refetch-on-unknown-kid path immediately; the
>   JWKS cache TTL (default 1h) only bounds how long a _removed_ key remains trusted.

---

## Type changes

None. `DiscoveryDocument` and `ProviderTokens` keep their shapes; `id_token` was already a
required `String` — it just stops being silently defaulted.

---

## Implementation notes

1. Add `crates/adapters/src/shared/http.rs`: a `OnceLock<reqwest::Client>` (or per-provider
   client) built with `connect_timeout` (5s) and `timeout` (10s) as compile-time constants
   and `redirect::Policy::none()`.
   Replace `reqwest::get` at `shared/jwks.rs:72` and `shared/discovery.rs:20`, and
   `reqwest::Client::new()` at `shared/token_endpoint.rs:12`, `oidc/mod.rs:175`, and
   `crates/providers/src/apple.rs:300`.
2. `shared/jwks.rs:71-83` — check `response.status()` before `.json()`; never cache an error
   body. Add a `get_key(kid)`-style API (or `refresh()` with a 30s `min_refresh_interval`
   timestamp guard) so callers can force one refetch on a `kid` miss.
3. Wire the refetch into the kid-lookup failures at `oidc/mod.rs:103-108` and
   `apple.rs:231-236` (currently a terminal `InvalidGrant`).
4. `shared/token_endpoint.rs:23-42` — on non-2xx, parse `{"error", "error_description"}` and
   return `ProviderError` with both; on 2xx, error if `id_token` is absent (drop the
   `unwrap_or_default()` at line 39).
5. `shared/discovery.rs:24-31` — compare `doc.issuer` to `issuer_url` (normalising the
   trailing slash, as the fetch already does) and error on mismatch.
6. Tests (wiremock): JWKS 500 not cached; kid rotation succeeds without waiting out the TTL;
   token endpoint 400 with `invalid_grant` surfaces the OAuth error; 200 without `id_token`
   errors; discovery issuer mismatch errors; a delayed response times out.

---

## Merge plan

1. Apply the 02-ports-and-adapters block and both 05-provider-system blocks to their
   canonical pages; bump each page's `**Date:**`.
2. No schema change.
3. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- Providers serve all of these endpoints without redirects (Apple and Google do today);
  disabling redirects breaks no supported provider.
- One shared client per process is safe across providers (reqwest clients are cheap to
  clone and connection-pool per host).

### Decisions

- _Fail closed on JWKS errors._ **A non-2xx JWKS response is an error, never a cached key
  set.** Serving a cached error body for 1h is strictly worse than failing the request.
- _Refetch is rate-limited._ **A `kid` miss triggers at most one forced refetch per
  minimum-refresh interval.** An attacker spraying random `kid`s must not turn the service
  into a JWKS-endpoint hammer.
- _Timeouts are constants._ **5s connect / 10s total stay compile-time constants, not config
  knobs.** No supported provider needs different values, and constants keep the config
  surface flat until a real deployment demands otherwise.
- _Kid-miss refetch interval is 30s._ **The minimum refresh interval for forced refetches is
  30s, down from the proposed 60s.** Halving it makes key rotation pick up faster while
  still capping refetches at two per minute per process.
- _Redirects disabled._ **The shared client uses `redirect::Policy::none()`.** Apple and
  Google serve these endpoints without redirects today, so a redirect is more likely
  misconfiguration or attack than legitimate behaviour.

### Open questions

- (None at this stage.)
