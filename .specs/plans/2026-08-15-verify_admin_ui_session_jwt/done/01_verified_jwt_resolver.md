# Task 01 — Verified JWT resolver

**Plan:** [plan.md](../plan.md) · **Certificate:** Intentionally omitted at user direction; do not create a certificate file.

**Implements:** [change spec §Proposed changes — Authentication model](../../../changes/merged/2026-08-05-verify_admin_ui_session_jwt.md#specsadmin-uispecs00-overviewmd--authentication-model-modify), [§Proposed changes — Environment](../../../changes/merged/2026-08-05-verify_admin_ui_session_jwt.md#specsadmin-uispecs00-overviewmd--environment-modify), [§Type changes](../../../changes/merged/2026-08-05-verify_admin_ui_session_jwt.md#type-changes), and [§Implementation notes 1–3](../../../changes/merged/2026-08-05-verify_admin_ui_session_jwt.md#implementation-notes)
**Depends on:** —
**Produces:** `verifyAccessToken(token)` returns only a signature-verified, issuer- and audience-bound payload using discovery-resolved JWKS; no helper decodes claims without verification.
**Pointers:** `apps/admin-ui/package.json:6-37`; `pnpm-lock.yaml:15-68`; `apps/admin-ui/src/lib/auth.ts:1-55`; `apps/admin-ui/src/app.d.ts:1-11`; `.specs/service/specs/04-http-api.md:49-54`

## Steps

- [x] Add `jose` as the runtime JWT verification dependency and update the pnpm lockfile; add the named environment-variable constants and make missing issuer or audience configuration fail closed during module initialization.
- [x] Replace direct `/keys` fetching and `decodeJwtPayload` with a five-minute, negative-safe discovery and JWKS resolver that validates the configured issuer against discovery before constructing the local JWK set.
- [x] Implement `verifyAccessToken` with `jwtVerify`, constrain algorithms to the discovered allowlist, use JWKS `kid` selection, require `exp`, `iss`, `aud`, and `sub`, and bind issuer and audience to configured values; remove the separate expiry and unverified-decode paths.
- [x] Make `hasAdminClaim` accept only a string exactly equal to the configured required value, and use the verified payload type in `App.Locals` rather than retaining a raw token claim source.
- [x] Keep cache duration and any new validation bounds as named constants, validate network JSON as untrusted input, and ensure errors propagate to callers for fail-closed handling.

## Definition of done

- [x] A correctly signed token for the configured issuer, audience, algorithm, and JWKS `kid` produces a verified payload containing required claims.
- [x] Missing configuration, malformed discovery/JWKS, failed network fetch, unknown `kid`, unsupported or `none` algorithm, invalid signature, missing required claim, expired token, foreign issuer, and wrong audience reject without returning claims.
- [x] `hasAdminClaim` rejects absent, array, number, and object values even when coercion would have produced the required string, and no `decodeJwtPayload` or equivalent unverified payload reader remains under `apps/admin-ui`.
- [x] Meets the repo definition of done (named bounds, assertions, TypeScript tests, `pnpm format:check`, `pnpm lint`, `pnpm typecheck`, and `pnpm test`; see plan.md baseline).
- [x] Reviewable: inspect one exported verifier path and demonstrate that only a token signed by the discovery-selected service key can yield claims.

## Evidence

- `apps/admin-ui/src/lib/auth.ts` is the sole token-to-claims path and uses `jose` verification with configured issuer/audience and discovery algorithms.
- Discovery and JWKS fetches use named timeout, response-size, key-count, algorithm-count, and five-minute cache bounds; failed loads are not cached and concurrent loads are coalesced.
- Discovery and JWKS URLs require HTTPS and same origin; malformed data, redirects, unsupported types/algorithms, missing claims, invalid time claims, and failed fetches reject without exposing response bodies.
- `apps/admin-ui/src/app.d.ts` retains verified claims instead of a raw token.
- Repository `pnpm format:check` is intentionally deferred/skipped under jj per standing instruction; direct package formatting is used in final gates.
- Round-1 remediation fixes the trust algorithm set at EdDSA, RS256/384/512, PS256/384/512, and ES256/384/512 and requires discovery to be a unique nonempty subset. JWKs require unique 128-character-safe kids, mandatory matching alg, signature use/verification-only operations, Ed25519 or exact NIST curves, or RSA >=2048 bits.
- JSON is streamed under a one-MiB cap with JSON media-type, final-origin, redirect, Content-Length, five-second whole-response timeout, overflow cancellation, and no unbounded arrayBuffer path. Rotation is one retry under a 30-second global cooldown plus deterministic 128-entry negative-kid cache; non-rotation failures never refetch.
- Tokens are <=16 KiB with 30-second tolerance, one-hour maximum age and lifetime, exp > iat, bounded JOSE/issuer/subject/audience/configured claim strings, and at most eight audiences.
