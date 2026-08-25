# Task 01 — Defaults merge keeps explicit falsy overrides

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-defaults_merge_keeps_falsy_overrides-certificate.md](01-defaults_merge_keeps_falsy_overrides-certificate.md)

**Implements:** [06-configuration.md](../../../service/specs/06-configuration.md) §Loading order, §"zero disables a scope" / `refresh_rotation` / `enabled`; change spec §The delta → S2
**Depends on:** —
**Produces:** Explicit `false`/`0`/`""` config overrides survive resolution instead of silently reverting to `config/default.toml`; an explicitly empty string now fails loudly in its domain resolver.
**Pointers:** `crates/server/src/bootstrap.rs:67-111` (`merge_raw_defaults`, `remove_empty_values`), `:188-196` (`resolve_builder`), `:202-215` (`resolve_config_toml`)

## Steps

- [ ] Rework `merge_raw_defaults` (`bootstrap.rs:67`) to take two `toml::Value` trees and merge with the existing recursive table-merge (tables merge, scalars and arrays replace); delete `remove_empty_values` (`bootstrap.rs:94`).
- [ ] In `resolve_builder` (`:188`), deserialize the built source tree into a raw `toml::Value` (not `RawConfig`), merge it onto `config/default.toml`'s parsed `toml::Value`, then deserialize the merged tree into `RawConfig` for `AppConfig::resolve`.
- [ ] In `resolve_config_toml` (`:202`), parse the input TOML straight to `toml::Value` and follow the same merge-then-deserialize path.
- [ ] Confirm the env-override channel (`parse_config` / `OIDC_EXCHANGE__…`) flows through the same value-level merge.

## Definition of done

- [ ] Regression tests through `resolve_config_toml`: `[token] refresh_rotation = false` resolves `false`; `[rate_limit] per_subject = 0` resolves `0`; `[rate_limit] enabled = false` resolves `false` (each confirmed broken before the fix).
- [ ] Negative-space + preservation tests: an explicit `[token] access_token_ttl = ""` fails resolution with the duration parser's error; a config omitting these keys still inherits the committed defaults (`true`, `10`, `true`); an env path `OIDC_EXCHANGE__TOKEN__REFRESH_ROTATION=false` through `parse_config` resolves `false`.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace`; named-constant limits where applicable — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the new bootstrap config tests and sees a `false`/`0`/`""` override reach the resolved `AppConfig` unchanged (and an empty duration rejected) where it previously reverted.
