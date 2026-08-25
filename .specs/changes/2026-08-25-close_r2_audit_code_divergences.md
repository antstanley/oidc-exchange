# Change: Close the R2 audit's code-side divergences

**Status:** Proposed · **Date:** 2026-08-25 · **Owner:** Ant Stanley · **Target:** crates/core, crates/server, schemas (service)

Fix the seven divergences from the 2026-08-25 R2 conformance review whose remedy is code, not
spec prose: make `adapter = "apple"` resolvable so the shipped Apple provider is reachable
(S1); stop the defaults merge from silently reverting explicit `false`/`0`/`""` overrides
(S2); route the refresh flow's security outcomes through the mandatory audit channel (S3);
thread the middleware-resolved `ClientAddr` into the core flows so their audit events record
true provenance (S7); put `/nonce` under the public per-IP throttle (S11); catch
`schemas/datamodel.schema.json` up with the shipped audit enums and the `operator` field
(S6-code); and give the config-valid `prometheus` telemetry exporter an accurate init arm
instead of the "unknown exporter" warning (S16-code). In five of the seven the canonical spec
already describes the target behaviour — the code catches up to the page — and the canonical
edits are confined to twelve short passages across six pages plus the schema sidecar, none
beyond text these code changes alter or republish.

---

## Motivation

The R2 review (2026-08-25, session-local; not committed to the repo) traced every normative
claim in `.specs/service/specs/00–08` against the branch and found that its three
highest-value findings are executable-code defects the spec correctly describes:

- **S1** — `ProviderAdapter::parse_field` (`crates/core/src/config.rs:2025-2040`) has no
  `"apple"` arm, so `Config::resolve` rejects every config with an Apple provider block and
  the registry arm at `crates/server/src/bootstrap.rs:1607` is dead code. Four canonical
  pages and the docs all claim `adapter = "apple"` works. Empirically verified:
  `providers.adapter: invalid provider adapter "apple"`.
- **S2** — `remove_empty_values` inside `merge_raw_defaults`
  (`crates/server/src/bootstrap.rs:94-111`) strips every `false`, `0`, and `""` from the
  deployment overlay before merging onto `config/default.toml`, on every entry point.
  Empirically verified: `token.refresh_rotation = false` resolves back to `true`;
  `rate_limit.per_subject = 0` resolves back to `10`. Three documented switches — the
  rotation off-switch, "zero disables a scope", and `rate_limit.enabled = false` — are inert.
- **S3** — the refresh flow emits `TokenRefresh` success, `UserSuspended` (from both the
  rotation path and the rotation-disabled path inside `refresh_without_rotation`), and
  `RefreshTokenReuse` through best-effort `emit_audit` — four of the flow's five emission
  sites (`crates/core/src/service/refresh.rs:202-230, 346-359, 454-467, 500-532`; the
  fifth, the `ValidationFailed` refusal at 161-174, is spec'd best-effort and stays) — so
  a raised `emit_threshold` can drop a reuse alarm — exactly what the mandatory channel
  exists to prevent, and a contradiction of "shipped flows use the mandatory channel" in
  `00-overview`, `03-service-flows`, and `07-telemetry-and-audit`.
  `SecurityEvent::AuthenticationSucceeded { kind: Refresh }` is mapped
  (`domain/audit.rs:213-215`) but never constructed.

The remaining four are smaller but the same shape — code that lags the intended contract:
core flows discard the middleware's `Peer`/`Forwarded` resolution and re-wrap every address
as `Asserted` (S7); `/nonce` is unauthenticated, writes single-use state, and sits outside
the per-IP throttle (S11); `schemas/datamodel.schema.json` — named cross-adapter source of
truth by `08-persistence.md` — lags the code's audit enums by four values and the `operator`
field (S6); and `telemetry.exporter = "prometheus"` passes the closed config domain but
falls into the "unknown telemetry exporter" warning at init (`telemetry.rs:55-64`), which is
misleading for a value the resolver accepted (S16).

Per the R2 rule, a spec claim with no code behind it is fixed in code, not softened in the
page. This change is the code side only. The audit's documentation debt — folding the merged
admin plane into the canonical bodies (S4), the role default (S5), the parity-appendix
merges (S15), and the `canonical-types.schema.json` backlog (which includes the sidecar leg
of `08-persistence.md`'s mirror sentence — the claim that the sidecar mirrors
`datamodel.schema.json` stays false until that backlog lands) — is deferred to a separate
doc-only pass and deliberately not touched here.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md) | Modify: `ExchangeRequest` sketch carries `client_addr: ClientAddr` and picks up the real `provider_access_token` field its republication would otherwise re-omit; refresh-token reuse joins the `SecurityEvent` Warning list (the republished sentence also picks up operator-authentication failure) |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | Modify: the per-IP throttle paragraph names the exact throttled set (`/token`, `/revoke`, `/nonce`) |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | Modify: loading-order step 5 reorders its clauses to S2's merge order (raw-value merge onto the committed defaults, then `RawConfig`, then resolve); the `providers.<name>.adapter` closed-domain row's Type cell becomes `IdentityProviderAdapter` (S1 retypes the field); its `oidc \| apple` Domain cell, the rotation switch, and "zero disables a scope" become true as written |
| [`.specs/service/specs/07-telemetry-and-audit.md`](../service/specs/07-telemetry-and-audit.md) | Add: a `prometheus` row in the exporter-behaviour list. Modify: the Audit section's channel sentence names the refresh flow's Debug-refusal exception |
| [`.specs/service/specs/canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | Modify: `ExchangeRequest.ip_address` → required `client_addr` (+ a `ClientAddr` `$def`) and optional `provider_access_token` joins the properties; `RefreshRequest` and `RevokeRequest` gain `$defs` — only entities this change alters |
| [`.specs/service/specs/00-overview.md`](../service/specs/00-overview.md) | None — the Apple scope row, the rotation-switch Decision, and the mandatory-channel goal become true as written; the goal's "security outcomes" names the closed `SecurityEvent` set, wholly mandatory after S3, and the same goal sentence assigns operational events — where the refresh flow's retained Debug-level `ValidationFailed` refusals live — to the best-effort channel (see the *Retained-text accuracy* Decision) |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | None — the `IdentityProvider | Apple` inventory row becomes reachable as written |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | Modify: the token-exchange client-context sentence names `client_addr` (S7 renames the `ExchangeRequest` field it cites); the reuse step's blocking-failure sentence and the reuse-alarm Decision's rationale move to durability-channel vocabulary (S3 takes the event off the thresholded channel they cite); the Audit-emission closing paragraph qualifies its channel claims — security outcomes go mandatory, and the refresh flow's Debug-level `ValidationFailed` refusals are named as the deliberate best-effort exception; the refresh flow's `ValidationFailed`-at-Debug text already matches the retained behaviour |
| [`.specs/service/specs/05-provider-system.md`](../service/specs/05-provider-system.md) | Modify: the registry sketch's string keys become the `IdentityProviderAdapter` variants and its `other → error` line moves to config load (S1 retypes the field the sketch dispatches on); the Tier-2 `adapter = "apple"` config block boots as written |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) | None — S6 makes the code-side leg of the mirror sentence true again (the service's typed entities match `datamodel.schema.json`); the sentence's other leg — that [`canonical-types.schema.json`](../service/specs/canonical-types.schema.json) also mirrors it — stays inexact and remains with the deferred doc pass (Merge plan step 3 excludes it) |

`schemas/datamodel.schema.json` is a code-side artifact, not a canonical page; its delta is
in [The delta → S6](#s6-code--catch-schemasdatamodelschemajson-up-with-the-code) below.

---

## The delta

### S1 — Make the Apple provider reachable from configuration

`providers.<name>.adapter` will get its own closed domain instead of borrowing
`ProviderAdapter`. `ProviderAdapter` (`crates/core/src/config.rs:1998-2041`) is shared by
five fields — `key_manager.adapter`, `repository.adapter`, `session_repository.adapter`,
`user_sync.adapter`, and `providers.<name>.adapter` — so adding an `Apple` value there would
also make `repository.adapter = "apple"` parse. Instead:

- Add `enum IdentityProviderAdapter { Oidc, Apple }` beside `ProviderAdapter` in
  `crates/core/src/config.rs`, with `as_str()` and a `parse_field` accepting exactly
  `"oidc"` and `"apple"` and rejecting everything else with the existing
  `providers.adapter: invalid provider adapter …` wording.
- `ProviderConfig.adapter` (`config.rs:1728`) becomes `IdentityProviderAdapter`;
  `ProviderConfig::resolve` (`config.rs:1737-1775`) parses through the new domain. The
  issuer requirement at `config.rs:1756-1764` applies to `Oidc` only — the Apple adapter
  pins its issuer to `https://appleid.apple.com` internally and takes its settings from the
  `extra` map.
- `build_single_provider` (`crates/server/src/bootstrap.rs:1596-1616`) matches the enum
  variants directly (`Oidc` → the generic adapter via `provider_config_to_oidc`, `Apple` →
  `AppleProvider::from_config(&config.extra)`); the string match and its `other` arm
  disappear. On main that arm is reachable: the shared enum's eight storage/key values pass
  `Config::resolve` — the issuer requirement (`config.rs:1756-1764`) is Oidc-only — so
  `[providers.x] adapter = "postgres"` errors only here, at registry build
  (`bootstrap.rs:1612-1615`), and under `role = "admin"`, which builds no registry, not
  at all. The dead code is the `"apple"` arm at `bootstrap.rs:1607`,
  which becomes live; the storage/key values move to an earlier config-load rejection (see
  Compatibility).
- `ProviderAdapter` keeps all nine of its values — the eight storage/key values and `Oidc` —
  and its four remaining fields, unchanged. `Oidc` is not removed from the shared enum; only
  `ProviderConfig.adapter` stops parsing through it.

Tests:

- A resolve-level test (through `resolve_config_toml`, `crates/server/src/bootstrap.rs:202`)
  booting a config with an Apple provider block —
  `[providers.apple] adapter = "apple"` plus its `extra` settings — asserts resolution
  succeeds and the resolved provider's adapter is `Apple`, with no `issuer` required.
- Negative tests asserting a `ConfigError` naming `providers.adapter` at resolution for both
  an unknown value (`adapter = "atproto"`) and a storage value the shared enum used to admit
  (`adapter = "postgres"`), pinning the failure point S1 moves to config load.
- A new test pinning the Oidc-only issuer requirement — none exists today: the
  `providers.<id>.issuer: missing required HTTPS URL` rejection (`config.rs:1756-1764`)
  appears in no test, and no fixture omits `issuer` under `adapter = "oidc"`. It asserts
  an `[providers.x] adapter = "oidc"` block without `issuer` fails resolution with that
  error; paired with the Apple boot test above, it pins that the requirement neither
  leaks onto `Apple` nor is lost from `Oidc`.

### S2 — Stop reverting explicit falsy config overrides

The defaults merge will move to raw `toml::Value` trees, and `remove_empty_values` will be
deleted. Root cause: `RawConfig` is `#[serde(default)]` throughout, so both entry points
deserialize the deployment overlay into `RawConfig` first — materializing every unset field
as its Rust default (`""`, `0`, `false`, or a real value) — and then re-serialize it for
`merge_raw_defaults` (`crates/server/src/bootstrap.rs:67-92`). At that point "unset" and
"explicitly set to a falsy value" are indistinguishable, and `remove_empty_values`
(`bootstrap.rs:94-111`) strips both so that serde-default artifacts (e.g. an empty
`access_token_ttl`) cannot clobber `config/default.toml`. That is the stripping's only
genuine need, and it disappears when nothing round-trips:

- `resolve_builder` (`bootstrap.rs:188-196`) deserializes the built source tree into a raw
  `toml::Value` (not `RawConfig`), merges it onto `config/default.toml`'s parsed
  `toml::Value` with the existing recursive table-merge (tables merge, scalars/arrays
  replace), and only then deserializes the merged tree into `RawConfig` for
  `AppConfig::resolve`. Keys the operator never set are genuinely absent from the overlay;
  keys set to `false`/`0`/`""` are explicit values that survive the merge.
- `resolve_config_toml` (`bootstrap.rs:202-215`) parses the input TOML straight to
  `toml::Value` and follows the same path.
- `merge_raw_defaults` takes two `toml::Value`s; `remove_empty_values` is deleted.

An explicitly-set empty string now reaches the domain resolvers and fails loudly (e.g.
`access_token_ttl = ""` is an invalid duration) instead of silently reverting to the
default — the fail-closed direction this codebase already prefers.

Regression tests, all through `resolve_config_toml` (each empirically confirmed broken
today):

- `[token] refresh_rotation = false` resolves with `refresh_rotation == false`.
- `[rate_limit] per_subject = 0` resolves with `per_subject == 0` (zero disables the scope).
- `[rate_limit] enabled = false` resolves with `enabled == false`.
- Preservation: a config omitting those keys still inherits `config/default.toml` (`true`,
  `10`, `true`), and an explicitly empty `[token] access_token_ttl = ""` now fails
  resolution with the duration parser's own error.
- An environment-override path test (`OIDC_EXCHANGE__TOKEN__REFRESH_ROTATION=false` through
  `parse_config`) confirming the structural env channel survives the same way.

### S3 — Put the refresh flow on the mandatory security-audit channel

The refresh flow will emit its security outcomes exactly as `exchange.rs` and `revoke.rs`
already do — through `emit_security_event`/`emit_security_event_with_detail`
(`crates/core/src/service/mod.rs:280-318`), which bypasses `emit_threshold` and applies the
`audit.durability` contract:

- Add `SecurityEvent::RefreshTokenReuse` to the closed set
  (`crates/core/src/domain/audit.rs:156-176`): `severity()` → `Warning`, `event_type()` →
  `AuditEventType::RefreshTokenReuse`. The rendered event is byte-compatible with today's
  (`refresh_token_reuse`, warning, outcome `success`, detail `{family_id,
  sessions_revoked}`); only the channel changes.
- `revoke_family_for_reuse` (`refresh.rs:185-242`): replace `create_audit_event` +
  `emit_audit` with `emit_security_event_with_detail(SecurityEvent::RefreshTokenReuse,
  AuditOutcome::Success, Some(user_id), None, client_addr, user_agent, detail)`. The
  revoke-before-emit order is preserved: the family is dead before the emission can fail.
- Both suspension gates — the rotation path (`refresh.rs:345-360`) and the
  rotation-disabled path inside `refresh_without_rotation` (`refresh.rs:453-468`, live
  whenever `token.refresh_rotation = false`, the switch S2 makes functional): each becomes
  `emit_security_event(SecurityEvent::PrincipalSuspended,
  AuditOutcome::Failure(AuditFailure::PrincipalSuspended), Some(user.id), None,
  client_addr, user_agent)` — the same shape exchange's terminal mapping produces.
- `audit_successful_refresh` (`refresh.rs:500-533`): `emit_security_event_with_detail(
  SecurityEvent::AuthenticationSucceeded { kind: AuthenticationKind::Refresh },
  AuditOutcome::Success, Some(user_id), None, client_addr, user_agent,
  detail {family_id, generation, grace})`. The mapping at `domain/audit.rs:213-215` is
  finally constructed.
- The refresh flow's Debug-level `ValidationFailed` refusals
  (`refuse_with_validation_failed`, `refresh.rs:151-178` — the shared refusal path for
  unknown, expired, missing-user, CAS-raced, and rotation-disabled presentations, serving
  refusal sites before and after `resolve_refresh_token` classifies alike) **stay** on
  best-effort `emit_audit` — `03-service-flows.md` explicitly specifies `ValidationFailed`
  at Debug below the default `emit_threshold`.

Tests (a refresh companion to `crates/core/tests/exchange_mandatory_outcomes.rs`):

- Refresh success, suspension — on both the rotation path and, with
  `token.refresh_rotation = false`, the rotation-disabled path — and reuse events are
  emitted even with `emit_threshold` raised above their severities (e.g. to `error`).
- `ValidationFailed` refusals remain filtered by the default `emit_threshold`.
- With `audit.durability = "enforce"` and a failing sink: the reuse path has already revoked
  the family when the emission error propagates; success and suspension fail the request per
  the durability contract. With `"observe"`, degradation is recorded and the flow's own
  outcome stands.

### S7 — Thread real client provenance into the core flows

`ExchangeRequest` (`crates/core/src/service/exchange.rs:33-54`), `RefreshRequest`
(`refresh.rs:40-51`), and `RevokeRequest` (`revoke.rs:13-25`) will replace
`ip_address: Option<String>` with `client_addr: ClientAddr`, carrying the audit-context
middleware's resolution (`crates/server/src/middleware/audit_context.rs:62-79`:
`Peer` from the observed/hyper/Lambda-platform peer, `Forwarded` behind
`server.trusted_proxies`, `Unknown` otherwise) instead of flattening it to a string:

- Route handlers pass `audit_ctx.client_addr.clone()` (`crates/server/src/routes/token.rs:245,
  263, 275`; `crates/server/src/routes/revoke.rs:50-55`) instead of `audit_ctx.ip_address()`.
- The `ClientAddr::asserted(request.ip_address)` rebuilds are deleted:
  `exchange.rs:121-125`, the five sites in `refresh.rs` (per S3's channel split, the
  `ValidationFailed` refusal at 161-174 stays on best-effort `emit_audit` and only swaps
  its `ClientAddr` argument; the reuse, both suspension gates, and success sites at
  202-230, 346-359, 454-467, and 500-532 fold into S3's mandatory emission calls), and
  `revoke.rs:44-48` use `request.client_addr` directly, so every core-flow audit event
  records the true `ip_address_source` (`peer`/`forwarded`/`unknown`) rather than
  `asserted`.
- `Session.ip_address` stays `Option<String>`, populated via
  `request.client_addr.audit_address()` (`exchange.rs:491`) — the stored value is unchanged.
- `ClientAddr` gains `impl Default` = `Unknown` (fail-closed) so
  `RefreshRequest`/`RevokeRequest` keep their `#[derive(Default)]`.
- Lambda and FFI need no entry-point changes: both run the same router, `Peer` comes from
  the Lambda platform request context where one exists, and the FFI layer resolves `Unknown`
  when no transport peer exists. `ClientAddr::Asserted` remains in the domain for
  embedder-supplied hints; the core flows simply stop manufacturing it.

Tests: a server e2e asserting a `/token` terminal audit event records
`ip_address_source == "peer"` (and `"forwarded"` behind a trusted proxy), and core-test
updates across `crates/core/tests/{exchange,exchange_mandatory_outcomes,assertion,refresh,
revoke,service_leak_corpus,user_admin}.rs` constructing requests with `ClientAddr` values.

### S11 — Throttle `/nonce`

`public_throttle_layer` (`crates/server/src/middleware/public_throttle.rs:61`) adds
`"/nonce"` to its path set: `matches!(path, "/token" | "/revoke" | "/nonce")`. The route is
unauthenticated and writes single-use state (`mint_nonce`), so it gets the same
server-established per-IP budget before any handler work. The layer is router-wide with a
path early-return, so no mounting changes; `/nonce` is still mounted only when
`grants.id_token` is enabled. The failed-attempt budget (`per_ip_failures`) is untouched —
`/nonce` never renders an authentication failure, so only the normal per-IP budget applies.

Tests (alongside the existing throttle e2e at `crates/server/tests/e2e.rs:730`): exhausting
the per-IP budget against `/nonce` returns `429` with `error == "slow_down"` and
`Retry-After >= 1` and emits the mandatory `ThrottleExceeded` event; the budget is shared
with `/token` (same `RateLimitKey::ClientAddr`); requests without a server-established
address are not throttled.

### S6-code — Catch `schemas/datamodel.schema.json` up with the code

Three edits to the `AuditEvent` definition (`schemas/datamodel.schema.json:62-85`), mirroring
`crates/core/src/domain/audit.rs`:

- `event_type` enum (line 69) gains `refresh_token_reuse`, `missing_credential`,
  `invalid_credential`, `not_configured` — the 18 variants of `AuditEventType`
  (`audit.rs:56-81`).
- `outcome.reason` enum (line 81) gains the same four values — the 9 variants of
  `AuditFailure` (`audit.rs:344-360`), plus `null`.
- `AuditEvent.properties` gains optional `operator`, mirroring the published shape at
  `schemas/internal-api.schema.json:114-126`: an `OperatorPrincipal` definition
  (`{id: non-empty string, mechanism}`, required both) and an `OperatorAuthMechanism`
  definition (`enum: ["mtls", "operator_token", "shared_secret"]`), added under this file's
  `definitions` key. `required` is unchanged — `operator` is `None` on the exchange plane.

A mirror test (no new dependencies) reads the schema file and asserts its `event_type` and
`outcome.reason` enum arrays equal the serde-rendered variant lists of `AuditEventType` and
`AuditFailure`, so the next enum addition fails a test instead of drifting silently.

### S16-code — An explicit `prometheus` arm in telemetry init

`init_telemetry` (`crates/server/src/telemetry.rs:16-68`) will match the closed
`TelemetryExporter` enum (`crates/core/src/config.rs:1936-1966`) instead of
`config.exporter.as_str()` with a catch-all: `None`/`Stdout` → the JSON formatter,
`Otlp`/`Xray` → their existing fallback warnings, and `Prometheus` → the JSON formatter plus
an accurate warning naming prometheus as accepted but not yet implemented — no metrics are
exported and no metrics endpoint is exposed. The unreachable "unknown telemetry exporter"
arm (`telemetry.rs:55-64`) disappears with the string match: the config domain is closed, so
a config-valid value can never again be reported as unknown. **Decision recorded below:**
warn-only, not a real Prometheus pipeline — exporter wiring belongs with the pending
[`2026-06-24-complete_telemetry_exporters.md`](2026-06-24-complete_telemetry_exporters.md)
change.

Coordination: that pending change also rewrites the same exporter-behaviour list in
`07-telemetry-and-audit.md` — its Modify block replaces the `otlp`/`xray` bullets this
change's Add block sits beside, and carries no `prometheus` row. Whichever spec merges
second must re-verify that list against `init_telemetry` and re-seat the `prometheus`
bullet rather than apply its block mechanically (see Merge plan step 1).

---

## Proposed changes

### `.specs/service/specs/01-domain-model.md` → Exchange request types (`service/exchange.rs`) (Modify)

In the `ExchangeRequest` sketch, the line

> ```rust
>     ip_address: Option<String>,
> ```

becomes

> ```rust
>     provider_access_token: Option<String>,
>     client_addr: ClientAddr,
> ```

(`provider_access_token` is not new code — the field has shipped since the grant-binding
merge (`exchange.rs:44`) and the sketch this change republishes omits it; see the
*Republished-sketch completeness* Decision) and the final sentence of the section — "The
three trailing fields are client context captured by the audit-context middleware, not
grant parameters." — is replaced by:

> `provider_access_token` carries the provider access token co-issued with a
> directly-presented ID token so the `at_hash` binding control can verify it (on the
> authorization-code path it is ignored — the access token from `ProviderTokens` takes the
> same slot); a bearer credential, never logged, never persisted, dropped once the
> assertion is bound. The three trailing fields are client context captured by the
> audit-context middleware, not grant parameters. `client_addr` preserves the middleware's
> resolved provenance (`Peer`/`Forwarded`/`Unknown`), so the flow's audit events record the
> true `ip_address_source`; `RefreshRequest` and `RevokeRequest` carry the same field.

(The replacement embeds the replaced sentence verbatim between the new
`provider_access_token` and `client_addr` prose. The section's earlier closing sentences —
`ExchangeCredential` as the typed form of the declared `grant_type`, `RefreshRequest` as
the refresh grant's own input type, and `ExchangeRequest` deriving no `Default` — remain
true and stand unchanged.)

### `.specs/service/specs/01-domain-model.md` → SecurityEvent and ClientAddr (`domain/audit.rs`) (Modify)

The severity-mapping sentence adds refresh-token reuse to the Warning list. Because the
rewrite re-asserts that the mappings are exhaustive, it also picks up operator-authentication
failure (`OperatorAuthenticationFailed` → `Warning`, `crates/core/src/domain/audit.rs:200`),
which the current sentence predates — a sentence this change republishes must be true on
merge, so this one omission does not wait for the deferred S4 pass:

> Its `severity()` and `event_type()` mappings are exhaustive: exchange/refresh success and
> session revocation are `Info`; authentication failure, registration denial, principal
> suspension, provider rejection, refresh-token reuse, operator-authentication failure, and
> `ThrottleExceeded` are `Warning`; principal creation, all-session revocation, and admin
> mutations are `Notice`.

### `.specs/service/specs/03-service-flows.md` → Token exchange (`exchange.rs`) (Modify)

The client-context sentence closing the exchange flow (03-service-flows.md:103-105) names
the `ExchangeRequest` field S7 renames. It becomes:

> `ExchangeRequest` carries the client context (`client_addr`, `user_agent`, `device_id`)
> extracted by the server's audit-context middleware; the stored session records all three
> (the address via `client_addr.audit_address()`), and every audit event in the flow records
> the `ip_address` — with the middleware's resolved `ip_address_source` — and `user_agent`
> (the `AuditEvent` shape carries no `device_id`).

(The rest of the paragraph — the per-event classification list from "A suspended user
audits…" — is unchanged. The page's later "carries the same client context; audit events in
the flow record its `ip_address` and `user_agent`" sentences (03-service-flows.md:181-182,
187-188, 229-230) also stand: `ip_address` there names an `AuditEvent` field, which keeps
its name; only this sentence names the renamed request field.)

### `.specs/service/specs/03-service-flows.md` → Token refresh (`refresh.rs`) (Modify)

The closing sentence of the reuse step (03-service-flows.md:141-142) rationalizes
revoke-before-emit through a "blocking audit failure" — the `blocking_threshold` mechanics
S3 takes this event off. Step 3 becomes:

> 3. **Reuse.** `revoke_family(family_id)`, then emit `RefreshTokenReuse` at
>    `AuditSeverity::Warning` with `detail { family_id, sessions_revoked }`, then return
>    `InvalidToken` carrying the same reason string as the unknown-token branch — the
>    response does not tell the presenter that an alarm fired. Revocation runs before the
>    emission so a durability-enforced emission failure cannot leave the family alive.

(Only the final sentence changes; the rest of the step is republished verbatim for a
mechanical merge.)

### `.specs/service/specs/03-service-flows.md` → Audit emission (`service/mod.rs`) (Modify)

The section's closing paragraph claims "Every shipped flow uses the mandatory channel" and
that `emit_audit` "remains available for operational events supplied by embedders" — both
unqualified, while S3's own channel split deliberately keeps the refresh flow's Debug-level
`ValidationFailed` refusals on `emit_audit`. The paragraph becomes:

> Severity follows RFC 5424 (emergency 0 … debug 7); lower is more severe. Every shipped
> flow emits its security outcomes on the mandatory channel; the one deliberate exception
> is the refresh flow's Debug-level `ValidationFailed` refusals, which stay on
> best-effort `emit_audit` below the default `emit_threshold`. The HTTP public
> per-IP throttle also emits `ThrottleExceeded` through this same API before returning its
> terminal `429`; the middleware logs an enforce-mode emission error but preserves the
> `429`, so audit-sink behavior cannot make the denial unsafe. `emit_audit` otherwise
> remains available for operational events supplied by embedders, and only that
> best-effort channel is governed by `emit_threshold` and `blocking_threshold`.

(Only the second and the closing sentences change; the two-channel algorithm block above
the paragraph is unchanged.)

### `.specs/service/specs/03-service-flows.md` → Assumptions and open questions (Modify)

The Decision *The reuse alarm is emitted at `Warning`.* (03-service-flows.md:401-405)
derives the severity from `emit_threshold` survival and `blocking_threshold` failure — the
best-effort channel mechanics S3 removes for this event. It becomes:

> - *The reuse alarm is emitted at `Warning`.* **`RefreshTokenReuse` carries
>   `AuditSeverity::Warning` on the mandatory security channel.** The severity classifies;
>   it does not route. The mandatory channel bypasses `emit_threshold`, so no configured
>   threshold can drop the alarm, and a sink failure follows `audit.durability` — `observe`
>   (the default) records degradation while the refusal stands, `enforce` fails the request.
>   `Warning` keeps the event beside the other attack signals (authentication failure,
>   principal suspension, `ThrottleExceeded`) rather than among the `Info` successes, which
>   is the boundary downstream alerting keys on.

(The surrounding Decisions are unchanged.)

### `.specs/service/specs/04-http-api.md` → Middleware stack (Modify)

The per-IP throttle sentence names the exact throttled set:

> Public routes additionally use, in route-layer execution order, the per-IP throttle, access
> log, and concurrency guard. The throttle covers `/token`, `/revoke`, and `/nonce` — the
> unauthenticated routes that handle credentials or write single-use state — and runs before
> handler/provider work; the remaining public routes (`/keys`, `/health`, and the discovery
> route) pass through the access log and concurrency guard but are not throttled. Only `Peer`
> and `Forwarded` values become rate-limit keys.

(The rest of the paragraph — `429 slow_down`, `Retry-After`, the mandatory
`ThrottleExceeded` emission, the durability note — is unchanged. The page's second,
contradictory stack description under its `## Runtime parity update` appendix
(04-http-api.md:312-341) is deliberately untouched: the throttle sentence exists only under
`## Middleware stack`, and merging that appendix into the body is the deferred S15 pass,
not this change.)

### `.specs/service/specs/05-provider-system.md` → Provider registry (Modify)

The section's framing ("every `[providers.*]` block whose `adapter` is recognised") and its
sketch's `other → error (unknown adapter)` line (05-provider-system.md:118-130) describe the
shared-enum dispatch S1 deletes: after S1, `adapter` is the two-value
`IdentityProviderAdapter`, parsed during `Config::resolve`, so no unrecognized value reaches
the bootstrap. The opening and sketch become:

> The bootstrap builds the registry from every `[providers.*]` block. `adapter` is the
> closed two-value `IdentityProviderAdapter` domain, parsed at config load, so an unknown
> value is rejected during `Config::resolve` and every block that reaches the registry
> names its constructor:
>
> ```
> Oidc  → OidcProvider::from_config
> Apple → AppleProvider::from_config
> ```

(The closing paragraph — no registry for the `admin` role, request-time `UnknownProvider` →
HTTP 400 `invalid_request` — is unchanged.)

### `.specs/service/specs/06-configuration.md` → Loading order (Modify)

Step 5 currently describes the pre-merge `RawConfig` round-trip S2 deletes ("deserialized as
`RawConfig`, merged onto the committed defaults, and narrowed"). Its clauses reorder to the
mechanism S2 installs — raw-value merge first, deserialization second:

> 5. The merged tree is merged onto the committed defaults as raw `toml::Value` trees
>    (tables merge, scalars and arrays replace), then deserialized as `RawConfig` and
>    narrowed by `AppConfig::resolve` into the typed configuration the service runs on; any
>    value outside its closed domain is rejected (see
>    [Validation at load](#validation-at-load)) and a failure aborts before any adapter or
>    router is built.

(Steps 1–4 and the "Steps 4 and 5 are one function" paragraph that follows are unchanged;
the latter's "deserializing the merged tree yields only the raw config" reads true under
either order.)

### `.specs/service/specs/06-configuration.md` → Validation at load (Modify)

In the closed-domain table, the `providers.<name>.adapter` row's Type cell names the new
domain S1 introduces — the Domain cell is already correct once S1 lands:

> | `providers.<name>.adapter` | `IdentityProviderAdapter` | `oidc` \| `apple` |

### `.specs/service/specs/07-telemetry-and-audit.md` → Telemetry (`telemetry::init_telemetry`) (Add)

Under that heading, a third bullet joins the "Current behaviour by `[telemetry].exporter`:"
list (the list intro at 07-telemetry-and-audit.md:20 — an intro line, not itself a heading):

> - `prometheus` → accepted, but currently emits a warning naming the exporter as not yet
>   implemented and falls back to the JSON formatter; no metrics are exported and no metrics
>   endpoint is exposed.

### `.specs/service/specs/07-telemetry-and-audit.md` → Audit (Modify)

The paragraph's closing sentence — "Shipped flows use the mandatory channel." — carries the
same unqualified claim as 03's Audit-emission section. It becomes:

> Shipped flows emit their security outcomes on the mandatory channel; the refresh flow's
> Debug-level `ValidationFailed` refusals are the one deliberate exception, retained on the
> best-effort channel ([03-service-flows.md](03-service-flows.md)).

(The rest of the paragraph — the two-channel definitions and the durability contract — is
unchanged.)

---

## Type changes

Fragment for `.specs/service/specs/canonical-types.schema.json`. Deliberately minimal: it
carries only entities this change alters — the retyped provenance field with its new
`ClientAddr` `$def`, and `$defs` for `RefreshRequest` and `RevokeRequest`, both retyped by
S7 and named in the proposed `01-domain-model.md` prose but never modeled by the sidecar.
Because the `ExchangeRequest` `$def` is `additionalProperties: false`, republishing it
without the shipped `provider_access_token` field (`exchange.rs:44`) would keep rejecting
the real shape, so that one field folds in here alongside `client_addr` (see the
*Republished-sketch completeness* Decision). As a modified entity, `ExchangeRequest` is
shown complete in its post-merge shape — retained properties, `type`, `description`, and
`additionalProperties` included — so the fold is a wholesale `$def` replacement, not a
hand-merge; its `$comment` enumerates the diff. The sidecar's other known staleness
(`AuditEvent.operator`, the three operator-auth event types, `UserPage`/`OperatorPrincipal`)
is the deferred doc pass's backlog and is not folded in here.

```json
{
  "$comment": "Fragment for 2026-08-25-close_r2_audit_code_divergences. ClientAddr, RefreshRequest, and RevokeRequest are new $defs; ExchangeRequest is a modified entity shown complete in its post-merge shape (replace the sidecar's $def wholesale) — its own $comment enumerates the diff.",
  "$defs": {
    "ClientAddr": {
      "type": "object",
      "description": "A client address together with how the service learned it (domain/audit.rs). peer/forwarded carry a server-established IP eligible as a rate-limit key; asserted carries a bounded client-authored string; unknown carries no address.",
      "required": ["source"],
      "additionalProperties": false,
      "properties": {
        "source": { "$ref": "#/$defs/ClientAddrSource" },
        "address": { "type": ["string", "null"] }
      }
    },
    "ExchangeRequest": {
      "$comment": "Complete post-merge shape; the fold replaces the sidecar's current ExchangeRequest $def wholesale. Diff vs the current sidecar: ip_address (string|null) is removed; client_addr is added and joins required — the Rust field is non-optional, with ClientAddr::Unknown (not absence) as the no-address case; provider_access_token is added, optional, matching the shipped field (exchange.rs:44) the def's additionalProperties: false previously rejected; the description now counts client_addr among the always-present fields. credential, provider, user_agent, device_id, type, and additionalProperties are retained unchanged.",
      "type": "object",
      "description": "Input to AppService::exchange. No default construction: credential, provider, and client_addr are always present.",
      "required": ["credential", "provider", "client_addr"],
      "additionalProperties": false,
      "properties": {
        "credential": { "$ref": "#/$defs/ExchangeCredential" },
        "provider": { "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString" },
        "provider_access_token": { "type": ["string", "null"] },
        "client_addr": { "$ref": "#/$defs/ClientAddr" },
        "user_agent": { "type": ["string", "null"] },
        "device_id": { "type": ["string", "null"] }
      }
    },
    "RefreshRequest": {
      "type": "object",
      "description": "Input to AppService::refresh. Derives Default: an empty refresh_token is constructible and fails classification downstream; client_addr defaults to unknown.",
      "required": ["refresh_token", "client_addr"],
      "additionalProperties": false,
      "properties": {
        "refresh_token": { "type": "string" },
        "client_addr": { "$ref": "#/$defs/ClientAddr" },
        "user_agent": { "type": ["string", "null"] },
        "device_id": { "type": ["string", "null"] }
      }
    },
    "RevokeRequest": {
      "type": "object",
      "description": "Input to AppService::revoke (RFC 7009). Derives Default; token_type_hint is \"refresh_token\" or \"access_token\" when present.",
      "required": ["token", "client_addr"],
      "additionalProperties": false,
      "properties": {
        "token": { "type": "string" },
        "token_type_hint": { "type": ["string", "null"] },
        "client_addr": { "$ref": "#/$defs/ClientAddr" },
        "user_agent": { "type": ["string", "null"] },
        "device_id": { "type": ["string", "null"] }
      }
    }
  }
}
```

The `schemas/datamodel.schema.json` edits are code-side and fully specified in
[The delta → S6](#s6-code--catch-schemasdatamodelschemajson-up-with-the-code); on merge they
need no canonical-page edit. They make the code-side leg of `08-persistence.md`'s mirror
sentence true (typed entities ↔ `datamodel.schema.json`); the sentence's sidecar leg —
`canonical-types.schema.json` mirroring the same file — stays false, and S6 widens that gap
(`datamodel.schema.json` goes to 18 event types plus `operator` while the sidecar keeps 15
without it), which is exactly the deferred backlog named above.

---

## Implementation notes

Suggested order — S2 first (it gates config-driven tests for everything else), then S1, then
the core-flow pair S7 → S3 (S3's emission calls take the `ClientAddr` S7 threads through),
then the independent S11, S6, S16:

```
1. S2  crates/server/src/bootstrap.rs:67-111, 188-196, 202-215 — value-level merge; delete
       remove_empty_values; regression tests beside the existing bootstrap config tests.
2. S1  crates/core/src/config.rs:1719-1775, 1998-2041 (new IdentityProviderAdapter;
       ProviderConfig.adapter retyped); crates/server/src/bootstrap.rs:1596-1616 (enum match).
3. S7  crates/core/src/service/{exchange.rs:33-54,121-125,491; refresh.rs:40-51;
       revoke.rs:13-25,44-48}; impl Default for ClientAddr (domain/audit.rs);
       crates/server/src/routes/{token.rs:245,263,275; revoke.rs:50-55}; test constructors in
       crates/core/tests/*.rs.
4. S3  crates/core/src/domain/audit.rs:156-238 (RefreshTokenReuse variant + mappings);
       crates/core/src/service/refresh.rs:185-242, 345-360, 453-468, 500-533 (emission
       swaps — 453-468 is the rotation-disabled suspension gate inside
       refresh_without_rotation); new crates/core/tests/refresh_mandatory_outcomes.rs
       modeled on exchange_mandatory_outcomes.rs.
5. S11 crates/server/src/middleware/public_throttle.rs:61; e2e tests beside
       crates/server/tests/e2e.rs:730.
6. S6  schemas/datamodel.schema.json:62-85 (+ definitions for OperatorPrincipal and
       OperatorAuthMechanism); enum-mirror test in crates/core.
7. S16 crates/server/src/telemetry.rs:16-68 (enum match; prometheus arm).
```

References: `emit_security_event`/`emit_security_event_with_detail`/
`emit_mandatory_audit_event` (`crates/core/src/service/mod.rs:255-346`) and exchange's
terminal-emission pattern (`crates/core/src/service/exchange.rs:126-210`) are the models for
S3; `audit_context_from_request`/`resolve_client_addr`
(`crates/server/src/middleware/audit_context.rs:39-79`) is the provenance source for S7;
`schemas/internal-api.schema.json:114-126` is the operator shape for S6.

---

## Compatibility and migration

- **S2** is behaviour-visible for configs that relied on the bug's side effect: an
  explicitly-set empty string (or a `${VAR}` placeholder resolving to one) previously
  reverted silently to the committed default and now fails resolution loudly. Deployments
  setting real falsy values (`refresh_rotation = false`, zero budgets,
  `rate_limit.enabled = false`) get the documented behaviour for the first time — operators
  who set those switches and were silently overridden should re-check their intent before
  upgrading.
- **S3** changes failure routing for the refresh flow's three security event kinds —
  success, suspension (emitted from both the rotation and rotation-disabled paths), and
  reuse, four emission sites in all: `emit_threshold` no longer
  filters them, and sink failures follow `audit.durability` instead of
  `blocking_threshold`. Under the committed defaults (`durability = "observe"`,
  `blocking_threshold = "warning"`) a reuse-alarm sink failure previously failed the request
  and now records degradation (feeding `/health`) while the refusal stands; under
  `durability = "enforce"` the request fails as before. Event wire shapes are unchanged.
- **S7** changes the recorded `ip_address_source` on core-flow audit events from
  `"asserted"`/`"unknown"` to the true `"peer"`/`"forwarded"`/`"unknown"` — SIEM queries
  filtering on the old constant must be updated. Stored session rows and event addresses are
  value-identical. `ExchangeRequest`/`RefreshRequest`/`RevokeRequest` are workspace-internal
  service inputs (constructed only by `crates/server` routes and core tests); no binding or
  FFI surface changes.
- **S11** newly rate-limits `/nonce` clients sharing a NAT egress with heavy `/token`
  traffic (shared per-IP budget, default 60/min). The 429 contract is the throttle's
  existing one.
- **S1** is additive for every working provider configuration but moves one failure
  earlier: a storage/key adapter value on a provider block
  (e.g. `[providers.x] adapter = "postgres"`) previously passed `Config::resolve` — the
  issuer requirement (`config.rs:1756-1764`) is Oidc-only — and failed only at registry
  build, in `build_single_provider`'s `other` arm (`bootstrap.rs:1612-1615`). For the
  roles that build the registry (`exchange`, `all`) both are startup `ConfigError`s, so
  nothing that boots changes behaviour. One residual break: a `role = "admin"` deployment
  never builds the registry (`05-provider-system.md`: for roles that do not serve
  `/token`, the registry is not built; `bootstrap.rs:420-424`), so such a block passes
  resolution and **boots today**; after S1 it is rejected at config load. Fail-closed and
  deliberate — the block could never serve a provider — but a boot-time break for that
  configuration shape (Decision below).
- **S6, S16** are strictly additive.

---

## Merge plan

1. Apply the twelve `Proposed changes` blocks to `01-domain-model.md`,
   `03-service-flows.md`, `04-http-api.md`, `05-provider-system.md`, `06-configuration.md`,
   and `07-telemetry-and-audit.md`; bump each page's `**Date:**` to the merge date. If
   [`2026-06-24-complete_telemetry_exporters.md`](2026-06-24-complete_telemetry_exporters.md)
   has merged first, its Modify block has replaced 07's exporter-behaviour list wholesale —
   re-verify that list against `init_telemetry` before seating the `prometheus` bullet
   (and if this change merges first, that spec's merge owes the same re-verification).
2. Fold the `Type changes` fragment into
   `.specs/service/specs/canonical-types.schema.json`: add the `ClientAddr`,
   `RefreshRequest`, and `RevokeRequest` `$defs`, and replace the existing `ExchangeRequest`
   `$def` wholesale with the fragment's — it is the complete post-merge shape (net effect,
   per its `$comment`: `ip_address` removed; `client_addr` added and required; optional
   `provider_access_token` added). Drop the change-tracking `$comment`s — the fragment
   root's and `ExchangeRequest`'s — on the way in; they document the fold, not the entity.
3. Verify the no-text-change rows of `Affected spec pages` now hold against the code (the
   R2 findings S1/S2/S3 claims read true). For `08-persistence.md`, verify the code-side
   leg only — typed entities against `datamodel.schema.json`; the sidecar leg of its
   mirror sentence is expected to remain false and belongs to the deferred doc pass, not
   to this verification. Clear any related Open-question entries on those pages.
4. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, move it to
   `.specs/changes/merged/`.
5. Update `.specs/README.md`: move this spec's row from pending to merged.

---

## Assumptions and open questions

### Assumptions

- The R2 review's empirical findings hold on this branch (re-verified against
  `main @ 323b049`): `adapter = "apple"` fails resolution; `refresh_rotation = false` and
  `per_subject = 0` revert; all five `refresh.rs` emission sites use `emit_audit` (the R2
  text cites four — the rotation-disabled suspension gate at `refresh.rs:454-467`, inside
  `refresh_without_rotation`, is the fifth, confirmed on this branch).
- `AppleProvider::from_config(&config.extra)` is construction-complete for a config-supplied
  block (the R2 review verified the adapter's internals conform; only reachability is
  broken).
- No deployment path bypasses `Config::resolve` to reach the provider registry (the R2
  review found none).
- The audit document itself is session-local and intentionally not committed; this spec
  carries the anchors it needs.

### Decisions

- *S14 existing-user predicate.* **Deliberately not changed.** The unconditional
  verified-email predicate for existing users (`exchange.rs:330-341`) is intended — code
  comment and design both say a tightened policy must not be bypassed by a prior login. The
  canonical page (`03-service-flows.md`, which conditions the re-check on
  `registration.domain_allowlist` being set) is wrong and will be corrected in the deferred
  doc-only pass, not here.
- *Scope.* **Code fixes only; the audit's doc debt is out of scope.** S4 (admin-plane
  fold-in), S5 (role default), S15 (parity-appendix merges), and the
  `canonical-types.schema.json` backlog go to a separate doc-only pass. Modify blocks here
  touch only canonical text these code changes alter or republish.
- *Retained-text accuracy.* **A canonical sentence one of these code changes falsifies gets
  a Modify block, even on a page otherwise untouched.** `03-service-flows.md:103` names the
  `ExchangeRequest` field S7 renames; `06-configuration.md`'s loading-order step 5 describes
  the pre-merge `RawConfig` round-trip S2 deletes; `03-service-flows.md`'s reuse step
  (03:141-142) and reuse-alarm Decision (03:401-405) rationalize through the
  `blocking_threshold`/`emit_threshold` mechanics S3 removes for that event;
  `03-service-flows.md`'s Audit-emission closing paragraph (03:315-320) and
  `07-telemetry-and-audit.md`'s "Shipped flows use the mandatory channel" (07:62) read as
  unqualified channel claims that S3's own channel split — the refresh flow's Debug-level
  `ValidationFailed` refusals staying on `emit_audit` — keeps inexact, so each names that
  exception; and `05-provider-system.md`'s registry section (05:118-130) keeps a
  "recognised"-adapter framing and an `other → error` line S1 moves to config load. Merging
  any of them unchanged would leave a canonical page contradicting the shipped code.
  `00-overview.md`'s channel goal (00:37-39) is the one channel claim that stands
  unqualified: its "security outcomes on a mandatory channel no configured threshold can
  filter" quantifies over the closed `SecurityEvent` set — every member of which S3 puts on
  the mandatory channel — and the same sentence assigns operational events to the
  best-effort channel, which is where the retained refusals (plain `ValidationFailed`
  audit events, never `SecurityEvent`s) live; 03:315-320 and 07:62 need qualifying blocks
  because they quantify over shipped flows' channel use, not over the security-outcome
  set. Sentences the changes leave true — 03's later "record its `ip_address`" lines,
  which describe `AuditEvent` fields — stay with the deferred doc pass.
- *S1 domain shape.* **A dedicated `IdentityProviderAdapter` domain, not an `Apple` value on
  `ProviderAdapter`.** The shared enum backs four storage/key fields whose domains must not
  widen; a two-value provider domain also matches `06-configuration.md`'s closed-domain row
  (`oidc | apple`) exactly. The row's Type cell updates to the new enum name in
  `Proposed changes`.
- *S1 admin-role residual break.* **Accepted; stated in Compatibility, not worked around.**
  A `role = "admin"` deployment carrying a storage/key value on a provider block boots
  today only because the registry that would reject the block is never built; S1's
  config-load rejection closes that gap for every role. Grandfathering the value for the
  admin role would preserve a config no code path can ever serve and reintroduce the
  role-dependent validation asymmetry S1 exists to remove.
- *S2 fix mechanism.* **Merge raw `toml::Value` trees before any `RawConfig`
  deserialization; delete the stripping.** The stripping's one genuine need — serde-default
  artifacts from the overlay's round-trip through `#[serde(default)]` structs — disappears
  when the overlay never round-trips. Diffing against serde defaults was rejected: it cannot
  distinguish an explicit default-valued setting from an unset one and would keep the bug
  for values that happen to equal a Rust default.
- *S2 empty strings.* **Explicit `""` now fails loudly.** Silently restoring the default hid
  misconfiguration (including `${VAR}` placeholders resolving to empty); failing in the
  domain resolver is the repo's established fail-closed posture.
- *S3 channel split.* **Success, suspension (both gates), and reuse go mandatory; the
  refresh flow's Debug-level `ValidationFailed` refusals stay best-effort.** The canonical
  spec explicitly places those refusals at `Debug`, below the default `emit_threshold`;
  moving them would change a documented contract, not conform to one. The retained refusals are the
  reason 03's Audit-emission paragraph and 07's channel sentence get qualifying Modify
  blocks instead of standing as written.
- *S3 reuse ordering.* **Revoke-before-emit is preserved.** A durability-enforced emission
  failure must never leave the reused family alive; the existing order already guarantees
  it, so only the emission call changes.
- *S3 severity-sentence accuracy.* **The republished mapping sentence also names
  operator-authentication failure; nothing else of S4 folds in.** The sentence re-asserts
  that the mappings are exhaustive, and `OperatorAuthenticationFailed` maps to `Warning`
  (`crates/core/src/domain/audit.rs:200`) — republishing a known omission under an
  "exhaustive" claim is not covered by deferring S4. The rest of the admin-plane fold-in
  stays with the doc-only pass.
- *Republished-sketch completeness.* **The `ExchangeRequest` sketch and `$def` this change
  republishes also pick up `provider_access_token`; nothing else of the sidecar backlog
  folds in.** The same rule as the severity sentence: the field has shipped
  (`exchange.rs:44`), the change has the pen on exactly that sketch and that
  `additionalProperties: false` definition, and re-omitting it would republish a known
  falsehood — a `$def` that still rejects the real shape. The remaining sidecar/01 backlog
  (`AuditEvent.operator`, operator-auth event types, `UserPage`/`OperatorPrincipal`) stays
  with the doc-only pass.
- *S7 representation.* **The request structs carry the domain `ClientAddr`; `Session` keeps
  its string.** Threading the resolved provenance is the point of the fix; the stored
  session value (`audit_address()`) is unchanged, so no adapter or schema migration is
  needed. `Default` for `ClientAddr` is `Unknown` — the fail-closed variant.
- *S7 `Asserted` retained.* **The variant stays in the domain.** Embedders may still assert
  addresses without transport provenance; the change removes only the core flows'
  manufacture of it.
- *S7 schema coverage.* **`RefreshRequest` and `RevokeRequest` get `$defs` in this change's
  fragment, and `client_addr` joins `ExchangeRequest.required`.** S7 retypes all three
  request structs; an entity named in merged prose with no schema definition is an orphan,
  so the pair folds in here rather than waiting for the doc pass. `client_addr` is required
  because the Rust field is non-optional — the sidecar's convention is that always-present
  fields are required, and `ClientAddr::Unknown`, not absence, is the no-address case.
- *S11 scope.* **`/nonce` joins the normal per-IP budget only.** It never renders an
  authentication failure, so the `per_ip_failures` budget is not extended to it; `/keys` and
  `/health` stay un-throttled (read-only, cacheable).
- *S16 depth.* **An accurate warning, not a wired exporter.** No Prometheus dependency
  exists in the workspace, and exporter wiring is already scoped by the pending
  `2026-06-24-complete_telemetry_exporters.md` change; a real pipeline here would smuggle
  that change in through a conformance fix. Matching the closed enum exhaustively also
  deletes the unreachable "unknown exporter" arm, making the misleading warning
  unrepresentable.

### Open questions

- Should the pending `2026-06-24-complete_telemetry_exporters.md` change spec be extended to
  cover a real `prometheus` metrics pipeline (it currently scopes OTLP and X-Ray only)?
