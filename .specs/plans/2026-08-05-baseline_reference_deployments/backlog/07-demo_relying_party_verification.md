# Task 07 — Demo relying party verification

**Plan:** [plan.md](../plan.md) · **Certificate:** omitted by requester

**Implements:** [change spec §Implementation notes — A.7 demo relying party](../../../changes/2026-08-05-baseline_reference_deployments.md#implementation-notes), [change spec §Regression tests](../../../changes/2026-08-05-baseline_reference_deployments.md#regression-tests)
**Depends on:** —
**Produces:** a reusable dependency-free verifier used by the AWS demo’s authenticated route and suitable for Node.js framework samples, with tests that reject forged and invalid claims.
**Pointers:** `examples/aws-web/demo-app/src/routes/authenticated/+page.server.ts:5-35`; `examples/aws-web/demo-app/src/routes/api/login/+server.ts:3-41`; `examples/aws-web/infra/lib/stack.ts:160-166`; `examples/nodejs/*`; `examples/aws-web/demo-app/package.json`

## Steps

- [ ] Extract a typed reusable verifier module that fetches and bounded-caches JWKS from `${AUTH_ENDPOINT}/keys`, selects `kid`, and re-fetches once on an unknown key ID.
- [ ] Parse token segments/header defensively, pin the expected algorithm, verify the signature with `webcrypto.subtle`, and validate `exp`, `iss`, and `aud` against injected deployment values.
- [ ] Replace decode-only authorization in the authenticated page with the verifier and retain safe redirect/error handling for absent or invalid cookies.
- [ ] Document a maintained-library recommendation while keeping the dependency-free sample, and define the import surface for Node.js framework examples without broadening their unrelated hardening scope.
- [ ] Add deterministic token/JWKS tests for valid verification and garbage-signature, `alg: none`, expired, issuer-mismatch, audience-mismatch, and unknown-`kid` refresh cases.

## Definition of done

- [ ] The authenticated route accepts only a signature-verified token with the configured algorithm, unexpired timestamp, issuer, and audience.
- [ ] Every listed invalid-token class is rejected in deterministic negative-space tests, including `alg: none` and stale/unknown key handling.
- [ ] JWKS cache and refetch behavior are bounded and use explicit named limits where introduced.
- [ ] The module has strict TypeScript types and no unvalidated network-data casts.
- [ ] Meets the repo definition of done (TypeScript format, lint, typecheck, tests; negative-space tests; named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the verifier fixtures and observe a valid token render the authenticated page while each forged/invalid token is rejected.
