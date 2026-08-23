# Task 03 — Verifier security tests

**Plan:** [plan.md](../plan.md) · **Certificate:** Intentionally omitted at user direction; do not create a certificate file.

**Implements:** [change spec §Implementation notes 8–9](../../../changes/merged/2026-08-05-verify_admin_ui_session_jwt.md#implementation-notes), [§Proposed changes — Authentication model](../../../changes/merged/2026-08-05-verify_admin_ui_session_jwt.md#specsadmin-uispecs00-overviewmd--authentication-model-modify), and [§Proposed changes — Environment](../../../changes/merged/2026-08-05-verify_admin_ui_session_jwt.md#specsadmin-uispecs00-overviewmd--environment-modify)
**Depends on:** 01, 02
**Produces:** deterministic Vitest coverage generates a signing key/JWKS fixture and proves verifier, hook, and login behavior across valid and adversarial JWT inputs.
**Pointers:** `apps/admin-ui/package.json:6-37`; `apps/admin-ui/vite.config.ts:1-10`; `apps/admin-ui/src/lib/auth.ts:1-55`; `apps/admin-ui/src/hooks.server.ts:1-40`; `apps/admin-ui/src/routes/login/+page.server.ts:1-58`; `bindings/lambda/__tests__/adapters.test.ts:1-20`

## Steps

- [x] Add Vitest and an `apps/admin-ui` `test` script using the repository's existing Vitest version; establish test helpers that generate a real signing key, serve a matching discovery/JWKS response, sign tokens, and construct SvelteKit cookie/request/redirect test doubles.
- [x] Test the resolver's successful signed token path plus garbage signature, `alg: none`, a key outside the JWKS, unknown `kid`, missing `exp`/`iss`/`aud`/`sub`, foreign issuer, wrong audience, and discovery/JWKS failure cases.
- [x] Test exact admin-claim handling so `role: ["admin"]` and other non-string values fail while a configured exact string succeeds.
- [x] Exercise the hook and login load/action using the same fixture: valid admin tokens pass both enforcement points, verification failures create no session or clear one, and verified non-admin tokens follow the denied flow.
- [x] Assert login cookie name and attributes (`Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`, no Domain) and expiry-derived `maxAge`; add a final repository grep assertion or review step that no unverified decode path remains.

## Definition of done

- [x] The test suite uses a real generated key pair and matching JWKS fixture rather than fabricated decoded payloads for positive signature acceptance.
- [x] Every validation rule introduced by Task 01 has positive and negative-space coverage, including all specified malformed, signature, algorithm, key, required-claim, issuer, audience, and fetch-failure paths.
- [x] Gate and login tests prove both enforcement points use verification, do not issue sessions on rejection, and retain the verified non-admin denied behavior.
- [x] Cookie assertions prove the exact host-prefixed name, required attributes, absent Domain, and verified-expiry lifetime; source checks find no remaining unverified JWT decode helper or caller in `apps/admin-ui`.
- [x] Meets the repo definition of done (named bounds, assertions, TypeScript tests, `pnpm format:check`, `pnpm lint`, `pnpm typecheck`, and `pnpm test`; see plan.md baseline).
- [x] Reviewable: run the focused admin-UI Vitest suite and inspect named tests showing a valid signed admin token succeeds while each attack-shaped token has no session outcome.

## Evidence

- `corepack pnpm@11.9.0 --dir apps/admin-ui test`: 2 files, 40 tests passed. Generated RSA keys/tokens remain in memory and are never printed or snapshotted.
- Resolver cases cover valid and malformed tokens, signature/key/kid/rotation/cache behavior, algorithm/type, required/time/issuer/audience/admin claims, discovery/JWKS network failures, URL and collection bounds.
- Hook/login/logout tests cover protected enforcement, locals, denial, invalid clearing/no-persistence, exact cookie attributes/lifetime, and matching logout deletion.
- `corepack pnpm@11.9.0 --dir apps/admin-ui lint`: 0 warnings/errors. `typecheck`: 0 errors and one pre-existing Svelte warning in `routes/(app)/users/[id]/+page.svelte`.
