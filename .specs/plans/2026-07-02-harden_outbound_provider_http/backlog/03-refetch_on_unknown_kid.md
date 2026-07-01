# Task 03 — Refetch JWKS on an unknown kid in both providers

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-refetch_on_unknown_kid-certificate.md](03-refetch_on_unknown_kid-certificate.md)

**Implements:** [`05-provider-system.md` §OidcProvider behaviour](../../../service/specs/05-provider-system.md) (a JWKS `kid` miss triggers one rate-limited refetch before the token is rejected); [`05-provider-system.md` §Assumptions](../../../service/specs/05-provider-system.md) (upstream key rotation is picked up by the refetch-on-unknown-kid path immediately; the TTL only bounds how long a removed key stays trusted)
**Depends on:** 02
**Produces:** an unknown `kid` in a validated token triggers exactly one rate-limited JWKS refetch (via task 02's API) before the token is rejected, in both `OidcProvider` and `AppleProvider`, so a rotated signing key is picked up on the next login without waiting out the 1h TTL
**Pointers:** `crates/adapters/src/oidc/mod.rs:103-108` (terminal `InvalidGrant` on `kid` miss in `validate_id_token`); `crates/providers/src/apple.rs:231-236` (same terminal `InvalidGrant`); consumes the refetch API from `crates/adapters/src/shared/jwks.rs`

## Steps

- [ ] In `oidc/mod.rs::validate_id_token`, replace the terminal `InvalidGrant` at the `kid`-lookup failure (`:103-108`) with a call into task 02's forced-refetch path, re-searching the refetched key set for the `kid`; reject with `InvalidGrant` only after the refetch still misses.
- [ ] Apply the same change at `apple.rs:231-236`.
- [ ] Keep the rate limit intact — the refetch must go through task 02's `MIN_REFRESH_INTERVAL` guard, so a burst of unknown `kid`s cannot hammer the JWKS endpoint.
- [ ] Add ≥2 assertions per touched function (e.g. assert the refetched key set is a `keys` array; assert the resolved JWK's `kid` equals the header `kid`).
- [ ] Add `wiremock` tests for both providers: the JWKS server first serves a key set missing the token's `kid`, then (after rotation) one containing it; a validation that first misses then succeeds on refetch, without a TTL wait.

## Definition of done

- [ ] A token whose `kid` is absent from the cached set triggers one refetch and then validates when the refetched set contains the `kid`, in both `OidcProvider` and `AppleProvider` — verified by a `wiremock` rotation test per provider.
- [ ] The refetch is rate-limited by task 02's `MIN_REFRESH_INTERVAL`; repeated unknown `kid`s do not each cause a network fetch.
- [ ] Negative-space test: a `kid` still absent after the refetch is rejected with `InvalidGrant` (not a hang, not an infinite refetch loop).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the per-provider `kid`-rotation tests and confirms a rotated key validates on the next call without waiting out the TTL.
