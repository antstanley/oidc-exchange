# Release notes

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
