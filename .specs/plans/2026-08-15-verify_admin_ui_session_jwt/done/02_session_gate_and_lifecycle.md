# Task 02 — Session gate and lifecycle

**Plan:** [plan.md](../plan.md) · **Certificate:** Intentionally omitted at user direction; do not create a certificate file.

**Implements:** [change spec §Proposed changes — Authentication model](../../../changes/merged/2026-08-05-verify_admin_ui_session_jwt.md#specsadmin-uispecs00-overviewmd--authentication-model-modify), [§Type changes](../../../changes/merged/2026-08-05-verify_admin_ui_session_jwt.md#type-changes), and [§Implementation notes 4–7](../../../changes/merged/2026-08-05-verify_admin_ui_session_jwt.md#implementation-notes)
**Depends on:** 01
**Produces:** every protected route and login decision uses a verified payload, and verified admin login mints only the hardened `__Host-admin_session` cookie.
**Pointers:** `apps/admin-ui/src/hooks.server.ts:1-40`; `apps/admin-ui/src/routes/login/+page.server.ts:1-58`; `apps/admin-ui/src/routes/logout/+page.server.ts:1-8`; `apps/admin-ui/src/app.d.ts:4-7`; `apps/admin-ui/src/lib/auth.ts:1-55`

## Steps

- [x] Replace the gate's raw-cookie decode/expiry checks with `await verifyAccessToken`, retain redirect rethrows, put only `userId` and verified `claims` into locals, and remove the stale public `/login/callback` allowlist entry.
- [x] On all gate verification failures, delete `__Host-admin_session` at `path: "/"` and redirect to `/login`; direct verified non-admin payloads to `/denied` without treating them as a failed signature.
- [x] Apply the verifier in both login `load` and the pasted-token form action before checking claims or calculating cookie expiry; return `fail(401)` and set no cookie whenever verification fails.
- [x] Mint sessions only for verified admin payloads with `__Host-admin_session`, `secure: true`, `httpOnly: true`, `sameSite: "strict"`, `path: "/"`, no Domain option, and `maxAge` derived from the verified `exp`; preserve the `/denied` redirect for verified non-admin input.
- [x] Rename logout deletion and every remaining admin UI cookie reference to the host-prefixed cookie, preserving the `__Host-` prefix constraints.

## Definition of done

- [x] Missing, malformed, expired, tampered, unresolvable-key, or invalid-claim session cookies never reach a protected route, are deleted, and redirect to `/login`.
- [x] A valid signed admin session reaches protected routes with verified claims in locals; a valid signed non-admin token redirects to `/denied` and does not gain protected access.
- [x] Login load and form action verify before using any claim; invalid submissions return 401 with no cookie, while valid admin submissions set exactly the required hardened session attributes and a verified-expiry-derived lifetime.
- [x] Logout deletes `__Host-admin_session`, no `access_token` cookie reads or writes remain, and no handler reaches `hasAdminClaim` with an unverified payload.
- [x] Meets the repo definition of done (named bounds, assertions, TypeScript tests, `pnpm format:check`, `pnpm lint`, `pnpm typecheck`, and `pnpm test`; see plan.md baseline).
- [x] Reviewable: paste a verified admin token to receive the host-prefixed cookie, follow it to a protected page, then show an altered token redirects to login and clears the cookie.

## Evidence

- Hook and login load/action call `verifyAccessToken` before claims are read; protected locals contain only verified claims and subject.
- The only session name is `__Host-admin_session`; set/delete options are Secure, HttpOnly, SameSite=Strict, Path=/, and omit Domain. Login lifetime is derived from verified `exp`.
- Invalid gate/login-load cookies are cleared; invalid submissions return 401 without persistence; verified non-admin tokens retain the denied flow.
- Public paths are limited to `/login` and `/denied`; redirects are rethrown to avoid catch-loop behavior and no caller-controlled return URL is used.
