# Task 05 — Verify the discovered issuer matches the configured one

**Plan:** [plan.md](../plan.md) · **Certificate:** [05-discovery_issuer_check-certificate.md](05-discovery_issuer_check-certificate.md)

**Implements:** [`02-ports-and-adapters.md` §Shared OIDC utilities](../../../service/specs/02-ports-and-adapters.md) (`discovery::discover(issuer)` — errors if the document's `issuer` does not equal the configured issuer, RFC 8414 §3.3)
**Depends on:** 01
**Produces:** `discovery::discover` rejects a discovery document whose `issuer` field does not equal the configured issuer (trailing slash normalised, as the fetch already does), per RFC 8414 §3.3
**Pointers:** `crates/adapters/src/shared/discovery.rs:24-31` — post-parse handling; the existing trailing-slash normalisation at `discovery.rs:16-19` (`trim_end_matches('/')`)

## Steps

- [x] After parsing the `DiscoveryDocument`, compare `doc.issuer` to `issuer_url`, normalising the trailing slash on both sides the same way the fetch URL is built (`trim_end_matches('/')`).
- [x] On mismatch, return `Error::ProviderError` (naming the configured and discovered issuers) before returning the document.
- [x] Add ≥2 assertions to `discover` (e.g. assert `issuer_url` is non-empty; assert the returned document's normalised issuer equals the configured one on the success path).
- [x] Add a `wiremock` test: a discovery document whose `issuer` differs from the requested URL is rejected; confirm the existing matching-issuer tests still pass (including `discover_strips_trailing_slash_from_issuer_url`).

## Definition of done

- [x] A discovery document whose `issuer` differs from the configured issuer returns an error; a matching issuer (including a trailing-slash-only difference) still succeeds.
- [x] The comparison normalises the trailing slash consistently with the fetch-URL construction.
- [x] Negative-space test: an issuer-mismatch document is rejected with a `ProviderError` naming both issuers.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the issuer-mismatch test and confirms the mismatch is rejected while the existing discovery tests stay green.
