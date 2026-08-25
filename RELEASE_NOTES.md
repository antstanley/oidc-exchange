# Release notes

## 0.4.0

0.4.0 closes the code-side divergences found by the post-0.3.0 conformance audit of the
canonical specs against the implementation. The fixes are small but several change
observable behaviour — read **Behaviour changes** before upgrading.

### Behaviour changes

- **Explicit `false`/`0`/`""` configuration values are now honoured.** A defaults-merge bug
  silently discarded falsy overrides before they reached the resolver, so
  `token.refresh_rotation = false`, a zero rate-limit budget (which disables that scope),
  and `rate_limit.enabled = false` were quietly reverted to their defaults. The merge now
  operates on raw values before deserialization, so an explicitly-set falsy value survives.
  Deployments that set any of these and relied on the (broken) default behaviour will now
  see the value they actually wrote.
- **The Apple provider is reachable from configuration.** `providers.<id>.adapter = "apple"`
  now resolves and boots; previously the value was rejected at config load and the adapter
  was unreachable. Adapter selection is a closed domain (`oidc`, `apple`); an unknown value
  still fails closed at startup.
- **Refresh-flow security outcomes use the mandatory audit channel.** Refresh success,
  session suspension, and refresh-token reuse now emit through the mandatory channel governed
  by `audit.durability`, matching the token-exchange and revocation flows — a raised
  `audit.emit_threshold` can no longer drop a token-theft (`RefreshTokenReuse`) alarm. The
  Debug-level pre-emption `ValidationFailed` refusals remain on the best-effort channel by
  design. Under the committed defaults (`durability = "observe"`), a reuse-alarm sink failure
  now records degradation feeding `/health` rather than failing the request.
- **`/nonce` is rate-limited.** The nonce endpoint — unauthenticated and state-writing — now
  sits behind the same per-IP throttle as `/token` and `/revoke`.
- **Core-flow audit events record true client-address provenance.** The token, refresh, and
  revoke flows now carry the middleware-resolved `ClientAddr` (peer/forwarded/unknown)
  instead of recording every address as `asserted`.

### Other changes

- The published `schemas/datamodel.schema.json` was brought back in step with the code
  (the full audit `event_type` and failure-reason vocabularies, and the optional `operator`
  attribution), guarded by a mirror test.
- The `prometheus` telemetry exporter — accepted by configuration but not yet wired — now
  emits an accurate "accepted, not yet exported" warning with defined fallback instead of the
  misleading "unknown exporter" message.
- Release-pipeline reliability fixes carried since 0.3.0: corepack shims enabled in the npm
  validation/build/publish jobs, the last invalid pnpm-11 `exec --offline` removed, and
  `arethetypeswrong` bumped to 0.18.5 to fix a tarball-parse crash in package validation.

Deferred: the canonical specification pages and `canonical-types.schema.json` are updated by
the merge plan of the change spec that drove this release; that documentation pass, together
with the admin-plane and parity-appendix reconciliations the audit also identified, lands
separately.

## 0.3.0

0.3.0 is a security-hardening release: fourteen change specs landed since 0.2.0, tightening
the token endpoint, session lifecycle, configuration, admin plane, outbound HTTP, telemetry,
and the release pipeline itself. Several defaults are now fail-closed — read **Breaking
changes** before upgrading.

### Breaking changes

- **`server.role` defaults to `exchange`.** Earlier releases defaulted to `all`, which
  served the internal admin API on the same process as the public `/token` endpoint the
  moment `internal_api.enabled = true` was set. Deployments that relied on the implicit
  `all` must now set it explicitly; prefer running a separate `role = "admin"` process
  reachable only from an operator network. See `docs/guides/configuration.md` → Upgrading.
- **`role = "all"` binds two listeners.** The admin plane now serves on its own socket
  (`internal_api.host`/`port`, default `127.0.0.1:8081`) and is never merged into the
  public router. Single-plane runtimes (Lambda, embedded FFI) serve the public plane under
  `all` and log the unmounted internal routes.
- **Internal API authentication is named and bounded.** `internal_api.auth_methods` must
  list the enabled mechanisms (`shared_secret`, `operator_token`, `mtls`; the singular
  `auth_method` key still reads as a one-element list). An enabled shared secret must be at
  least 32 bytes. Failed operator authentications are audited and throttled per peer
  address with a lockout window.
- **Configuration is typed and fail-closed.** Every security-relevant field resolves into a
  closed domain at load; an invalid value aborts startup instead of being absorbed. KMS
  `key_manager.kms.algorithm` takes JWS names (`ES256`, `RS256`, …), not AWS
  `SigningAlgorithmSpec` names. `${VAR}` placeholders resolve (and fail closed on unset
  variables) on every entry point — server, Lambda, and FFI — and the new
  `oidc-exchange config check` subcommand validates a deployment file without side effects.
- **`grant_type` strictly binds `/token` parameters.** Each grant accepts a closed
  parameter set; parameters from another flow are rejected rather than ignored.
- **The direct ID-token grant ships disabled.** It now sits behind `[grants] id_token` and,
  when enabled, requires a server-issued nonce, is single-use, and enforces `azp`/`at_hash`
  binding.
- **Refresh tokens rotate on redemption.** Every refresh issues a replacement token and
  retires the presented one (60s default grace). Reuse of a retired token outside its grace
  window is treated as credential theft: the whole session family is revoked and a
  `RefreshTokenReuse` security event is recorded. The access-token `sid` claim now carries
  the session family id, and `/revoke` with an access token revokes exactly that family
  after full claim validation.
- **Node:** `handleRequest` now returns a `Promise`; callers must `await` it. Requests use
  separate `rawPath` and `query` fields and ordered header entries. `handleRequestSync`
  remains deprecated for one major cycle.
- **Python:** direct callers now pass `raw_path` and `query` separately and use ordered
  `(name, value)` header pairs. ASGI/WSGI applications migrate automatically and enforce
  the published 2 MiB body cap before buffering.
- **Lambda:** base-path handling moved from event adapters into the Rust normaliser. API
  Gateway and ALB adapters preserve the rawest event representation available, and sibling
  paths such as `/authorize` are no longer stripped as `/auth` children.

### Security hardening

- **Mandatory security-audit channel.** Authentication outcomes, registration denials,
  throttle lockouts, session revocations, and admin mutations emit through a channel that
  bypasses the emit threshold, with a closed failure-reason vocabulary and client-address
  provenance (`peer`/`forwarded`/`asserted`/`unknown`). `audit.durability = "enforce"`
  fails the request when the mandatory record cannot be written; `X-Forwarded-For` is
  honoured only from `server.trusted_proxies`.
- **Public-route rate limiting.** Per-address, per-failed-address, per-subject, and
  per-provider fixed-window budgets, a shared concurrency bound, and an access log guard
  every public route.
- **Secrets are unprintable by type.** Refresh-token hashes, client secrets, shared
  secrets, and upstream response bodies move through a `Secret<T>` newtype that cannot be
  formatted; spans and errors carry redacted, bounded excerpts only, backed by
  structural-leak and runtime-leak regression corpora.
- **One outbound HTTP boundary.** Every provider request (discovery, JWKS, token exchange,
  revocation) goes through a shared transport: status inspected before any body is read,
  success bodies fail at a 64 KiB ceiling, failure bodies truncate through a redacting
  diagnostic path. JWKS keys resolve through a purpose-filtered `VerificationKeySet` (key
  `use`/`key_ops`/`alg` eligibility decided in one place), and discovery endpoints are
  pinned to a declared per-provider `endpoint_origins` set (warning mode this release).
- **Attributed admin plane.** Every `/internal/*` mutation records the authenticated
  operator principal (`operator_token` and `mtls` name the operator; the shared-secret
  compatibility mechanism records the reserved `unattributed` identity). Admin reads are
  cursor-paginated and bounded.
- **Admin console session integrity.** The admin UI verifies its session JWT against the
  service JWKS via discovery (issuer, audience, expiry, signature) and fails closed,
  instead of trusting a decoded payload.
- **Hardened release pipeline.** Release jobs run least-privilege with pinned actions and
  frozen lockfiles; binaries and images carry GitHub build provenance attestations; npm and
  PyPI publish via OIDC trusted publishing (no stored tokens); a three-graph
  (Cargo/pnpm/Python) advisory gate and a signing-path dependency policy block releases;
  `install.sh` verifies checksums fail-closed and authenticates provenance with
  `gh attestation` when available.

### Other changes

- The canonical specs under `.specs/` were brought up to date with all shipped behavior,
  indexed, and a reference-deployment security baseline plan was filed for the next cycle.
- pnpm dependencies are exact-pinned with a minimum release age; the Python binding moved
  to pyo3 0.29.

Deprecated synchronous entry points are intentionally retained in 0.3 and scheduled for
removal in the following major release cycle; removal is deferred from this change.
