# Task 03 — Overlay and env overrides

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-overlay_and_env_overrides-certificate.md](03-overlay_and_env_overrides-certificate.md)

**Implements:** [06-configuration.md](../../../service/specs/06-configuration.md) → Loading order steps 1–3 (deep-merge overlay of `config/{OIDC_EXCHANGE_ENV}.toml` over `config/default.toml`; `OIDC_EXCHANGE__{section}__{key}` env overrides reaching every path, including map-valued sections)
**Depends on:** —
**Produces:** `load_config` that deep-merges the env-specific TOML over the default (tables merge recursively, scalars/arrays replace) and then applies `OIDC_EXCHANGE__…` environment overrides down to nested and map-valued keys.
**Pointers:** `crates/server/src/bootstrap.rs:26-47` (`load_config` — currently *replaces* rather than overlays, and has no env-override step); the already-declared `config` crate at `crates/server/Cargo.toml:20`

## Steps

- [x] Rewrite `load_config` to build a layered config: `config/default.toml` as the base, then `config/{OIDC_EXCHANGE_ENV}.toml` overlaid on top when `OIDC_EXCHANGE_ENV` is set, with tables merged recursively and scalars/arrays replaced (via the `config` crate's layered builder, or a `toml::Value` deep-merge if the crate cannot express base-plus-overlay cleanly).
- [x] Preserve the current fallbacks: a missing default file and a missing/empty env file are not errors; with no files present, compiled-in `AppConfig::default()` still applies.
- [x] Add an `Environment` source with prefix `OIDC_EXCHANGE` and separator `__`, so `OIDC_EXCHANGE__PROVIDERS__GOOGLE__CLIENT_ID` lands on `providers.google.client_id`; segments are lowercased and single underscores stay inside a segment (so `my_idp` is addressable).
- [x] Keep `load_config` returning the deserialized `AppConfig` (the placeholder-resolution and validation steps are added by tasks 04 and 05; do not remove the seam where they will attach).
- [x] Add tests: env TOML overlays (not replaces) default; a nested `OIDC_EXCHANGE__` override reaches a map-valued provider key; a single-underscore provider name (`my_idp`) is addressable.

## Definition of done

- [x] A value present only in `config/default.toml` survives when an env overlay sets a *different* key (overlay merges, does not wholesale-replace); a key set in both takes the env value.
- [x] An `OIDC_EXCHANGE__PROVIDERS__GOOGLE__CLIENT_ID` env var sets `providers.google.client_id`, and an `OIDC_EXCHANGE__SERVER__PORT` override reaches `server.port`.
- [x] Negative-space / edge coverage: missing files fall back to defaults without error; a single-underscore segment (`my_idp`) is addressed, not split.
- [x] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [x] Reviewable: with a default TOML plus an env overlay and an `OIDC_EXCHANGE__…` var exported, load and assert the merged `AppConfig` carries both the un-overridden default value and the overlaid/overridden values.

## Open questions

- Whether the `config` crate `0.15` layered builder deep-merges tables the way the spec requires, or whether a `toml::Value` recursive merge is needed as the base step; resolve during implementation and record the choice in the task's review notes.
