# Change: Verify the admin console's session JWT against the service JWKS

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** apps/admin-ui

Make the admin console verify the JWS signature of every token it treats as a credential,
against the JWKS the exchange service already publishes, before any claim of that token
influences a decision. Today both of the console's enforcement points — the per-request session
gate and the public `POST /login` action — decide who is an operator by *decoding* a JWT, never
by *verifying* it. This change collapses both onto one verifying helper, binds `exp`/`iss`/`aud`,
replaces the coercing admin-claim check with an exact type-checked comparison, and sets the
session cookie `Secure`, `HttpOnly`, and `SameSite`.

---

## Motivation

The console's authorization decision is currently derived entirely from attacker-authored bytes.
`src/hooks.server.ts:12-37` reads the client-supplied `access_token` cookie, hands it to
`decodeJwtPayload` (`src/lib/auth.ts:34-39` — split on `.`, base64url-decode segment `[1]`,
`JSON.parse`), and decides expiry and the admin role from that object. A hand-built payload
`{"sub":"x","role":"admin","exp":<future>}` with any header and any signature bytes is accepted.
`src/routes/login/+page.server.ts:20-57` — the public `POST /login` action, which the gate
explicitly exempts from authentication at `hooks.server.ts:8` — applies the identical decode and
then *mints* the session cookie from the caller's own string, so one unauthenticated POST is
sufficient; no cookie planting and no operator interaction. Every request that clears either
point reaches `src/lib/api.ts:6-15`, which attaches `Authorization: Bearer ${INTERNAL_API_SECRET}`
to `/internal/*` on the attacker's behalf — including the claims writes that the service flattens
into every access token it subsequently signs. The console already ships the missing half:
`getJwks` (`src/lib/auth.ts:21-31`) fetches and caches the service's JWKS and has zero call sites
anywhere in the repository. The verification step was written and never wired in. This violates
the repository's own invariants I1 ("signature before trust", which names `apps/admin-ui`'s
session check) and I13, and it is the console-side twin of the service-side fix already merged as
[2026-07-01-require_iss_aud_in_token_validation.md](merged/2026-07-01-require_iss_aud_in_token_validation.md).

Whether any operator runs this console on a reachable network is **not resolved by this
repository, and this change spec does not assume it away**. `apps/admin-ui/package.json` is
`private: true` at `0.0.1`; `.github/workflows/release.yml` has no admin-ui job and the root
`Dockerfile` builds only the Rust binary; `.github/workflows/ci.yml:139-147` runs `pnpm lint`,
`format:check` and `typecheck` for the console and never `pnpm build` or a test. Against that,
[admin-ui/specs/00-overview.md](../admin-ui/specs/00-overview.md) is marked `Implemented` and
describes a separate deployed service holding `INTERNAL_API_SECRET`, and `svelte.config.js`
targets `@sveltejs/adapter-node`. The exposure question is recorded in Open questions and is
tracked as blocking nothing here: the source defect is unconditional, the fix is small, and a
console that is not deployed today is a console that must be safe the day it is. Related defects
folded into this one change: the session cookie's hard-coded `secure: false`
(`login/+page.server.ts:47`, finding `g4-admin-console-session-cookie-not-secure`), the absence
of any issuer or audience binding (finding `g4-admin-console-missing-issuer-audience-binding`),
and the `String()` coercion in `hasAdminClaim` (`src/lib/auth.ts:43`).

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/admin-ui/specs/00-overview.md`](../admin-ui/specs/00-overview.md) | Replace the `Authentication model` section with the verifying gate and the single `verifyAccessToken` helper; extend `Environment` with the two new required variables; extend Assumptions, Decisions, and Open questions |
| [`.specs/development-guidelines.md`](../development-guidelines.md) | Extend the *Three toolchains, one CI* Decision: the `web-apps` job also runs `pnpm test` for `apps/admin-ui` |

No service-side spec changes. The console consumes `GET /.well-known/openid-configuration` and
`GET /keys` exactly as [service/specs/04-http-api.md](../service/specs/04-http-api.md) already
documents them; this change reuses that surface rather than inventing one. `apps/admin-ui` has no
`canonical-types.schema.json` sidecar and this change does not add one.

---

## Proposed changes

### `.specs/admin-ui/specs/00-overview.md` → Authentication model (Modify)

Replaces the section in full.

> `src/hooks.server.ts` is the gate. Public paths are `/login` and `/denied`;
> every other route requires a `__Host-admin_session` cookie carrying an access token whose JWS
> signature verifies against the service's published JWKS. The gate reads no claim from a token
> it has not verified.
>
> `src/lib/auth.ts` exposes exactly one function that turns a token string into claims:
> `verifyAccessToken(token)`. It resolves the service's key material through
> `GET ${OIDC_EXCHANGE_URL}/.well-known/openid-configuration` and verifies against the JWKS at
> that document's `jwks_uri` ([service/specs/04-http-api.md](../../service/specs/04-http-api.md)),
> caching both for five minutes. It:
>
> - constrains the signature algorithm to the discovery document's
>   `id_token_signing_alg_values_supported`, which the service populates from the live key
>   (`KeyManager::algorithm()`); `none` is never a member of that set and is never accepted, and
>   the token header never selects the algorithm;
> - selects the verification key by the token header's `kid` and rejects a `kid` absent from the
>   JWKS;
> - requires `exp`, `iss`, `aud`, and `sub` to be **present**, not merely correct when present;
> - rejects an expired token;
> - requires `iss` to equal `OIDC_EXCHANGE_ISSUER`, and requires the discovery document's own
>   `issuer` field to equal it too, so the console never learns the issuer solely from the
>   document it fetched;
> - requires `aud` to equal `ADMIN_UI_AUDIENCE`.
>
> Only a payload returned by `verifyAccessToken` reaches `hasAdminClaim`, which compares
> `payload[REQUIRED_CLAIM]` to `REQUIRED_VALUE` by exact equality after a `typeof === "string"`
> check (default `role === "admin"`, overridable via `REQUIRED_CLAIM`/`REQUIRED_VALUE`). An
> array, number, object, or absent claim is not admin; no claim value is coerced with `String()`.
>
> The failure mode is fail-closed and uniform. An absent cookie, a malformed token, a bad
> signature, an unknown `kid`, a missing or wrong `exp`/`iss`/`aud`, and an unreachable or
> unparseable JWKS all produce the same outcome: no session. The gate deletes the cookie and
> redirects to `/login`. A token that verifies but carries no admin claim redirects to `/denied`.
> The console never falls back to unverified claims when key material cannot be fetched.
>
> The login page accepts a pasted access token. `login/+page.server.ts` runs every token it
> handles through `verifyAccessToken` before anything else — the session cookie in its `load`
> (the redirect-if-already-signed-in check), the pasted string in its form action — and mints a
> session only from a token that verified; a
> token that fails verification returns `fail(401)` and sets no cookie. The session cookie is
> `__Host-admin_session`, set `secure: true`, `httpOnly: true`, `sameSite: "strict"`, `path: "/"`,
> with no `Domain`, and `maxAge` derived from the verified `exp`. `/logout` deletes the same
> cookie.
>
> There is no non-verifying decode helper. `decodeJwtPayload` does not exist.

### `.specs/admin-ui/specs/00-overview.md` → Environment (Modify)

> `INTERNAL_API_URL`, `INTERNAL_API_SECRET`, `OIDC_EXCHANGE_URL` (base for discovery and JWKS),
> `OIDC_EXCHANGE_ISSUER` (the exact `iss` the console accepts), `ADMIN_UI_AUDIENCE` (the exact
> `aud` the console accepts), `REQUIRED_CLAIM` (default `role`), `REQUIRED_VALUE` (default
> `admin`).
>
> `OIDC_EXCHANGE_ISSUER` and `ADMIN_UI_AUDIENCE` have no defaults. When either is unset the
> console serves no authenticated route: there is no configuration in which it accepts a token
> without binding both claims. `ADMIN_UI_AUDIENCE` must match the service's `[token] audience`
> ([service/specs/06-configuration.md](../../service/specs/06-configuration.md)). That key is
> required service-side — [2026-08-05-fail_closed_across_config_and_adapters.md](2026-08-05-fail_closed_across_config_and_adapters.md)
> merges first and makes an unset `token.audience` a startup error rather than an empty `aud` —
> so the pairing is well defined: a service that boots has an audience, and this console must
> be configured with the same value.

### `.specs/admin-ui/specs/00-overview.md` → Assumptions (Add)

> - `OIDC_EXCHANGE_URL` points at an instance serving the public role (`exchange` or `all`), so
>   discovery and `/keys` are mounted; a console pointed at an `admin`-role instance has no JWKS
>   to verify against and, by the fail-closed rule, admits no one.
> - The service publishes `id_token_signing_alg_values_supported` truthfully from the live
>   signing key, and signs access tokens with that same key manager. `GET /keys` returns a single
>   JWK, so a key rotation ends every console session that was signed under the previous key.
> - The deployment sets `[token] audience` on the service and `ADMIN_UI_AUDIENCE` on the console
>   to the same value.

### `.specs/admin-ui/specs/00-overview.md` → Decisions (Add)

> - *Verify, then read.* **One helper, `verifyAccessToken`, is the console's only path from a
>   token string to claims, and both the gate and the login action call it.** The previous split
>   — a shared non-verifying decoder behind two enforcement points — let each point look correct
>   in isolation while neither checked authenticity; a single verifying entry point cannot be
>   fixed in one place and regressed in the other.
> - *Discovery, not a hard-coded path.* **Key material is resolved through
>   `/.well-known/openid-configuration`, not by appending `/keys` to a base URL.** The document
>   supplies `jwks_uri` and the signing-algorithm allowlist together, so the console's algorithm
>   pinning tracks the service's live key instead of a constant that drifts at the next rotation.
> - *Issuer and audience are configured, not discovered.* **`OIDC_EXCHANGE_ISSUER` and
>   `ADMIN_UI_AUDIENCE` come from the environment; the discovery document's `issuer` is checked
>   against the configured value rather than trusted as the source of it.** A document fetched
>   from a substituted `OIDC_EXCHANGE_URL` would otherwise define the very issuer it is meant to
>   be checked against.
> - *Required, not correct-when-present.* **`exp`, `iss`, `aud`, and `sub` must be present.**
>   Matches the rule the service already applies to inbound ID tokens — `exp`/`iss`/`aud` as
>   required spec claims ([05-provider-system.md](../../service/specs/05-provider-system.md)),
>   and a missing or empty `sub` rejected by the provider adapters — and closes the
>   cross-token-type confusion class.
> - *Exact claim comparison.* **`hasAdminClaim` requires a string claim equal to
>   `REQUIRED_VALUE`.** `String(["admin"])` is `"admin"`, so the previous coercion admitted an
>   array-valued claim; the check must not depend on what a JSON value happens to stringify to.
> - *No dev escape hatch on the cookie.* **`secure: true` is unconditional — not a flag, an env
>   var, or a build-mode branch.** A configurable flag is precisely what shipped as
>   `secure: false // set to true in production`; browsers treat `http://localhost` as a secure
>   context, so unconditional `Secure` costs local development nothing.
> - *`__Host-` prefixed cookie, `SameSite=Strict`.* **The session cookie is
>   `__Host-admin_session`.** The prefix makes `Secure`, `Path=/`, and the absence of `Domain`
>   browser-enforced rather than reviewer-enforced, which closes the sibling-subdomain
>   cookie-write vector; `Strict` is available because the console has no cross-site entry flow.

### `.specs/admin-ui/specs/00-overview.md` → Open questions (Modify)

Keeps the existing entry and adds two.

> - A `/sessions` list view and an in-UI audit-log viewer are not implemented (the service has no
>   list-all-sessions endpoint); session management is per-user via revoke. The provider-redirect
>   flow is not built: there is no `login/callback` route, and its stale entry has been removed
>   from the gate's public-path list rather than left standing — an unauthenticated allowlist
>   entry for a route that does not exist is a gap waiting for a future route to fill it. The
>   entry returns when the flow does.
> - Whether this console is deployed anywhere is unresolved. `apps/admin-ui` is `private: true`,
>   the release workflow has no admin-ui job, the `Dockerfile` builds only the Rust binary, and
>   CI lints, format-checks, type-checks, and tests the console but does not build it. Whether to
>   add a build and a release artifact — or to state that the console is a reference
>   implementation operators deploy themselves — is open.
> - `sameSite: "strict"` is correct while login is a same-site form POST. If the
>   provider-redirect flow behind `login/callback` is built, the top-level cross-site GET return
>   needs `lax` on the cookie or a separate same-site landing hop; which of those is open.

### `.specs/development-guidelines.md` → Decisions, *Three toolchains, one CI* (Modify)

Extends the trailing sentence of the existing Decision.

> The Astro/SvelteKit apps add a `web-apps` job (oxlint/oxfmt + `astro check`/`svelte-check`),
> and that job runs `pnpm test` for `apps/admin-ui`, whose session-verification logic is
> security-critical and must be exercised rather than only linted.

---

## Type changes

`apps/admin-ui` has no `canonical-types.schema.json`, and this change adds no domain entity. The
one typed surface that changes is SvelteKit's `App.Locals` (`src/app.d.ts:4-7`), which stops
carrying the raw token string alongside the user id and carries the verified payload instead, so
no downstream consumer can re-derive claims from an unverified string:

```typescript
interface Locals {
  userId: string;
  claims: Readonly<Record<string, unknown>>; // verified payload from verifyAccessToken
}
```

---

## Implementation notes

1. Add `jose` to `apps/admin-ui/package.json` dependencies. Hand-rolled JWS verification is not
   acceptable here; `jose` supplies `createLocalJWKSet`/`jwtVerify` with `algorithms`,
   `issuer`, `audience`, and `requiredClaims`.
2. Rewrite `src/lib/auth.ts`. Delete `decodeJwtPayload` (`:33-39`). Fold `getJwks` (`:21-31`)
   into a discovery-aware resolver that fetches
   `${OIDC_EXCHANGE_URL}/.well-known/openid-configuration`, validates that its `issuer` equals
   `OIDC_EXCHANGE_ISSUER`, and fetches `jwks_uri`; keep the existing five-minute cache and make
   the cache negative-safe (a failed fetch must not be cached as an empty key set). Export
   `verifyAccessToken(token)` returning the verified payload or throwing. Rewrite `hasAdminClaim`
   (`:42-44`) to `typeof v === "string" && v === REQUIRED_VALUE`. `isExpired` (`:47-51`) is
   subsumed by `jwtVerify`'s `exp` handling plus `requiredClaims`; remove it rather than leaving
   a second expiry path.
3. Add named constants for the two new env vars beside `REQUIRED_CLAIM`/`REQUIRED_VALUE`
   (`src/lib/auth.ts:3-5`) and fail at module load when either is unset.
4. `src/hooks.server.ts:18-37` — replace `decodeJwtPayload` + `isExpired` + `hasAdminClaim` with
   `await verifyAccessToken(token)` followed by `hasAdminClaim`. Read the cookie
   (`:13`) under the new name. Keep the existing `"status" in err` re-throw so redirects are not
   swallowed by the catch-all, and keep the catch-all deleting the cookie and redirecting to
   `/login`.
5. `src/routes/login/+page.server.ts` — apply `verifyAccessToken` in both the `load` (`:5-18`)
   and the action (`:29-52`). Replace the `cookies.set` block (`:44-50`), including
   `secure: false` at `:47`, with the `__Host-admin_session` cookie described above. Return
   `fail(401)` on a verification failure and set no cookie.
6. `src/routes/logout/+page.server.ts:5` — delete the renamed cookie.
7. `src/app.d.ts:4-7` — replace `token: string` with `claims`. Update any consumer that reads
   `locals.token`.
8. Add `vitest` and a `test` script to `apps/admin-ui/package.json`, and an `Admin UI — test`
   step to the `web-apps` job (`.github/workflows/ci.yml:139-147`). Tests, generating a real key
   pair and a matching JWKS fixture: a correctly signed admin token is accepted by both the gate
   and the action; garbage signature bytes are rejected; `alg: none` is rejected; a token signed
   by a key outside the JWKS is rejected; an unknown `kid` is rejected; a token missing `exp`,
   `iss`, `aud`, or `sub` is rejected; a foreign `iss` and a wrong `aud` are each rejected; a
   `role` of `["admin"]` is rejected; a JWKS fetch failure yields no session rather than a
   fallback; the login action sets no cookie on any rejection; and the `Set-Cookie` carries
   `Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`, and no `Domain`.
9. Grep for any remaining unverified decode in `apps/admin-ui` before finishing; the invariant is
   that no path reads a claim from a string that `verifyAccessToken` did not return.

Structural context for the wider admin-plane question — this change is the "verify and encode"
first step, not the plane separation — is in
`.security/oidc-exchange/53cbdec9_20260804T102454Z/hardening/proposals/admin-plane-separation.md`.
Findings: `g4-admin-console-unverified-jwt-session-gate`,
`g4-admin-console-unverified-jwt-login-action`,
`g4-admin-console-missing-issuer-audience-binding`,
`g4-admin-console-session-cookie-not-secure`.

---

## Merge plan

1. Apply the five `Proposed changes` blocks to
   [admin-ui/specs/00-overview.md](../admin-ui/specs/00-overview.md); bump its `**Date:**` to the
   merge date.
2. Apply the *Three toolchains, one CI* block to
   [development-guidelines.md](../development-guidelines.md); bump its `**Date:**`.
3. No schema to fold in; no new canonical page.
4. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, and move it to
   `.specs/changes/merged/`.
5. Update `.specs/README.md`'s Change specs table.

---

## Assumptions and open questions

### Assumptions

- The exchange service in front of the console serves the public role (`exchange` or `all`), so
  `/keys` and `/.well-known/openid-configuration` are mounted. A console pointed at an
  `admin`-role instance has no JWKS to verify against and, by the fail-closed rule, admits no one.
- Operators can set `[token] audience` on the service. Without it the service issues `aud: ""`
  (`crates/core/src/service/mod.rs:73`) and this console rejects every token — an intended
  fail-closed outcome, but one that makes `[token] audience` a deployment prerequisite rather
  than an option wherever the console runs.
- `jose` is acceptable as the console's first runtime dependency outside the charting stack.

### Decisions

- *One change, four defects.* **Signature verification, `iss`/`aud` binding, the claim-coercion
  fix, and the cookie attributes ship together.** They share two files and one helper; splitting
  them would land partial states in which the console looks hardened but is not. The coercion
  fix in particular is only reachable as a bypass once signatures are verified, so shipping it
  separately would be a no-op patch to a still-open door.
- *Access tokens verified with the ID-token algorithm list.* **The console pins on
  `id_token_signing_alg_values_supported`.** The field is named for ID tokens, but the service
  populates it from `KeyManager::algorithm()` and signs access tokens with the same key manager,
  so it truthfully describes the key that will have produced the signature. It is the only
  algorithm metadata the service publishes.
- *Fail-closed on JWKS unavailability.* **A failed discovery or JWKS fetch ends the session
  rather than falling back to a stale-but-usable cache beyond its TTL or to unverified claims.**
  Matches the service's own fail-closed JWKS handling
  ([merged/2026-07-01-harden_outbound_provider_http.md](merged/2026-07-01-harden_outbound_provider_http.md));
  the alternative degrades to exactly the behaviour this change removes.
- *Existing sessions end at deploy.* **Renaming the cookie invalidates every session in flight.**
  Every session that exists under the current code is one that was admitted without a signature
  check, so ending them is the point, not a cost.

### Open questions

- Whether `apps/admin-ui` is deployed anywhere is unresolved, and this change spec does not
  resolve it. The package is `private: true`, no release job or `Dockerfile` stage builds it, and
  CI never runs `pnpm build`. The question is whether to add a shipping path (and raise these
  findings' severity accordingly) or to declare the console a reference implementation. It does
  not gate acceptance of this change: the defect is unconditional in the source either way.
- Should the console verify against the service at all, or should the service issue the console
  its own operator session? Option 2 of the admin-plane-separation proposal replaces this
  paste-a-token model outright. This change is deliberately the smaller one; whether it is the
  last word on console authentication is open.
- The console still holds `INTERNAL_API_SECRET` and spends it on behalf of whoever its gate
  admits. This change locks the front door and leaves the confused-deputy shape intact. Whether
  to scope the console's credential down to the operations it needs is open and belongs to
  [2026-08-05-harden_admin_plane.md](2026-08-05-harden_admin_plane.md). Merge order: that
  sibling names this spec a prerequisite, and its Environment and Decisions blocks for the same
  admin-ui page are written against the text this spec leaves behind — so this spec merges
  strictly first, and any change to those two blocks here means re-checking that sibling's.
