# Task 01 — Closed configuration domain and resolve

**Plan:** [plan.md](../plan.md)  
**Status:** Backlog  
**Implements:** [source spec](../../../changes/2026-08-05-fail_closed_across_config_and_adapters.md) §§Proposed changes → Configuration, Type changes, Implementation notes 1–2, Compatibility, and Decisions; [configuration canonical page](../../../service/specs/06-configuration.md) → Loading order, Validation at load, Sections, Defaults summary; [canonical types](../../../service/specs/canonical-types.schema.json) → `AccessTokenClaims`  
**Depends on:** —  
**Produces:** `RawConfig` as the serde/TOML boundary and typed runtime `Config` constructed only by `Config::resolve`; closed enums/newtypes for the source-spec domains; required non-empty HTTPS issuer and audience; preserved duration/allowlist/internal-secret checks; a controlled two-phase compatibility mode with observable rejections rather than a silent fallback.  
**Pointers:** `crates/core/src/config.rs`; `crates/core/src/service/mod.rs`; `crates/core/src/domain/provider.rs`; `config/default.toml`; `crates/core/tests/`.

## Steps

- [ ] Inventory every current `AppConfig` construction and consumer before changing the public
  shape; define a migration order that leaves no production caller holding unvalidated strings.
- [ ] Introduce `RawConfig` to mirror TOML and typed runtime `Config` (or a clearly named
  equivalent), with `Config::resolve(raw, env)` as the sole construction path for runtime
  configuration. Preserve all current `AppConfig::validate` checks through construction; do not
  delete a check merely because the representation changes.
- [ ] Define constructors/parsers for `ServerRole`, `RegistrationMode`, `SigningAlgorithm`,
  `AuditAdapter`, `TelemetryExporter`, `InternalAuthMethod`, `ProviderAdapter`, `HttpsUrl`,
  `AsciiDomainPattern`, and `NonEmptyString`. Keep construction error messages field-specific and
  return typed `ConfigError` values.
- [ ] Require `server.issuer` and `token.audience` in raw input and construct them as a non-empty
  HTTPS URL and non-empty string. Remove runtime representations that can emit empty `iss`/`aud`.
- [ ] Move duration parsing and allowlist shape/ASCII validation into construction, preserve the
  served-internal-API non-empty-secret cross-field rule, and validate every source-spec enum
  vocabulary instead of relying on downstream `unwrap_or`/default behavior.
- [ ] Apply `HttpsUrl` to server issuer, webhook URL, and the typed provider endpoint surface;
  expose only a `#[cfg(test)]` test constructor/injection seam for HTTP fixtures.
- [ ] Implement the source spec's temporary permissive rollout only as explicit, structured,
  field/class-named warnings and a documented temporary switch; default and final behavior must
  become hard rejection on schedule, with no silent fallback branch. Keep the exact release
  timing/flag decision visible for review.
- [ ] Add table-driven resolve tests for every accepted domain and every rejected value named by
  the source spec, including empty/missing issuer/audience, invalid role/mode/algorithms,
  non-ASCII/malformed allowlist entries, invalid audit/telemetry/internal values, non-HTTPS and
  scheme-less URLs, duration overflow, and served internal API without a secret.

## Definition of done

- [ ] The only production representation consumed after config construction uses closed types;
  malformed security fields return `ConfigError` naming the originating config field.
- [ ] Existing checks for role, three durations, allowlist shape, and served internal secret still
  reject invalid input, with both positive and negative tests.
- [ ] `server.issuer` and `token.audience` cannot be empty in runtime config or emitted access
  claims; URL constructors reject `http`, `file`, relative, and scheme-less values in production.
- [ ] Every enum parser and newtype constructor has representative accepted and rejected tests;
  test-only HTTP fixtures cannot create a production bypass.
- [ ] Any permissive compatibility window is explicit, observable, time-bounded, and covered by
  tests; no malformed value silently chooses a permissive runtime interpretation.
- [ ] Touched functions follow the guidelines' assertion/error/bound rules and focused core tests,
  `cargo fmt`, and relevant clippy checks are reported.

## Sibling boundaries

- This is the prerequisite seam for the placeholder-resolution sibling; retain existing
  placeholder behavior but do not implement that sibling's missing-FFI placeholder work here.
- Do not add `deny_unknown_fields`; the source spec leaves unknown config keys as a separate open
  question.
