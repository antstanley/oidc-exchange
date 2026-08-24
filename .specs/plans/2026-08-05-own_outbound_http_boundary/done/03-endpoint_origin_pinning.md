# Task 03 — Pin discovery endpoint origins and wire config

**Plan:** [plan.md](../plan.md) · **Status:** Done · **Depends on:** [01](01-prerequisite_contracts_and_guard_lint.md), [02](02-provider_transport.md)

**Implements:** source `05-provider-system.md`/`06-configuration.md` deltas; type fragment `OidcProviderConfig`; implementation note 8; origin-pinning tests.

**Scope:** Add `endpoint_origins` to the domain/config/bootstrap/provider construction path, calculate each provider's allowed origin set from issuer, configured endpoints, and config extras, and validate discovery endpoint origins under the externally confirmed `HttpsUrl` contract. Ship warning mode first, with a separately gated enforcement flip. Update every shipped Google configuration/documentation occurrence, not only the source note's partial file list.

## Steps

- [x] Extend `OidcProviderConfig`, parser fixtures, and `provider_config_to_oidc`; define config validation for HTTPS origin-only values and preserve redacted debug behavior.
- [x] Pass permitted origins into discovery for OIDC and the Apple construction path where applicable; reject/warn with endpoint, observed origin, and permitted set as specified.
- [x] Implement warning-mode telemetry and an explicit enforcement configuration/release switch or documented follow-up boundary; do not enforce without the one-release warning decision.
- [x] Update all in-repo Google stanzas surfaced by repository search: examples, README files, deployment/guides, and config tests; preserve placeholders and no-secret rules.
- [x] Add tests for undeclared rejection/enforcement, declared acceptance, issuer/configured endpoint inclusion, invalid origin syntax/scheme, and Google's multiple cross-origin discovery shape.

## Definition of done

- [x] Discovery cannot introduce an unpinned origin once enforcement is enabled; issuer/configured/declared origins behave exactly as documented.
- [x] Warning mode produces structured actionable output without rejecting the same deployment, and the enforcement transition has release-owner approval.
- [x] Every shipped Google sample names both required Google API origins and remains parseable.
- [x] Canonical type/prose updates are deferred to 07 unless this change is approved for merge; no done certificate is produced.

## Notes (completion record)

**Shape.** New `crates/adapters/src/shared/origins.rs` owns everything origin:
`parse_https_origin` (strict: https scheme, bare `scheme://host[:port]`, no
path/query/fragment — used only for operator-declared entries), `origin_of`
(lenient normalized-origin extraction used for the issuer, configured
overrides, and observed endpoints), the `EndpointOrigins` set built once from
issuer + configured endpoints + declared extras, `OriginCheckMode::{Warn,
Enforce}`, the shipped `ENDPOINT_ORIGIN_CHECK_MODE = Warn` release constant,
and `check_pinned_origin`, which warns or rejects naming endpoint kind,
observed origin, and the full permitted set. Named bounds: `MAX_ENDPOINT_
ORIGINS = 16`, `MAX_ENDPOINT_ORIGIN_LEN_BYTES = 256` (over-length entries are
rejected before any parse so hostile config text never reaches an error).

**Wiring.** `OidcProviderConfig.endpoint_origins: Vec<String>` (Debug keeps
client_secret redacted; origins are configuration-grade facts). Bootstrap
`provider_config_to_oidc` lifts the TOML array with strict per-entry
validation and indexed, non-echoing errors; absent means empty. Both providers
re-validate defensively at construction (paired checks across the boundary).
`discovery::discover(issuer, permitted)` now runs every supplied endpoint —
token_endpoint, jwks_uri, revocation_endpoint when present — through the same
mode decision after the RFC 8414 issuer check. Apple performs no runtime
discovery; its overrides are syntax-validated and their origins join the set,
with the admission invariant pinned by debug assertions.

**Warning/enforcement boundary.** Shipped mode is Warn: a structured
`tracing::warn!` fires and the deployment is served unchanged. Enforce is
fully implemented and tested through the mode-parameterised unit tests;
flipping `ENDPOINT_ORIGIN_CHECK_MODE` to `Enforce` is the explicit
release-owner act and must be its own reviewed commit after the one-release
warning window. The two stages are not collapsed anywhere.

**Google stanzas updated** (all repo-search hits): 7 example TOMLs (container,
linux-postgres ×2, aws-web, ecs-fargate, linux-sqlite ×2), `README.md`,
`README.docker.md` (no scopes line there — inserted after client_secret),
7 docs pages (quick-start, guides/providers, guides/configuration,
deployment/container, deployment/linux-postgres ×2 blocks, linux-server,
aws-lambda, linux-sqlite ×2 blocks), and the `crates/core/src/config.rs`
fixture with new parse assertions. Placeholders (`${GOOGLE_CLIENT_ID}` etc.)
and no-secret rules preserved. Canonical `.specs/` prose/schema untouched —
that is task 07.

**Tests added (22).** origins unit: strict accept + normalization (default-port
elision, host casing); strict rejects (path/query/fragment/http/ftp/garbage/
hostless); lenient extraction incl. loopback http; set membership incl.
lookalike-host negative space; unparseable candidates never admitted; dedup +
cap; warn accepts undeclared; enforce rejects naming all three facts; enforce
rejects unparseable; declared passes enforcement. Discovery integration:
warning-mode serving of an undeclared document; declared cross-origin
acceptance. OIDC integration: warning-mode end-to-end over two distinct
loopback origins; configured-endpoint-origin inclusion (discovery admitted via
an explicitly configured jwks origin, then actually used); Google's real
two-origin shape; adapter-boundary rejection of four invalid entry classes.
Bootstrap: absent→empty; normalization on lift; six invalid-entry classes;
non-array value; above-cap rejection; at-cap acceptance (validity boundary).

**Deviations recorded.**
1. Declared entries are strictly https-only, so plain-http loopback test
   origins cannot ride through `endpoint_origins`; tests instead exercise
   cross-origin admission through configured-endpoint origins (which join the
   set leniently) plus real-https Google-shape strings that are parsed but
   never fetched. Production semantics unaffected; wiremock suites stay green.
2. The https scheme constraint on issuer/configured/discovered *endpoints*
   themselves stays unadopted here — it belongs to the sibling `fail_closed`
   change, and enforcing it now would break every loopback test. Only the
   declared-origin entries (this task's new surface) are strict.
3. Enforcement ships as a compile-time constant rather than an operator knob:
   an operator-facing toggle would itself be the release decision task 03 is
   told not to make silently.

No done certificate produced.
