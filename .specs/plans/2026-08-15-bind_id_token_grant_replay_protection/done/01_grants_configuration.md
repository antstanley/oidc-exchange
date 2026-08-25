# Task 01 — Grants configuration

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted at user request

**Implements:** [`.specs/changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md` §Proposed changes → `06-configuration.md`](../../../changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md), [§Implementation notes 1](../../../changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md); [06-configuration.md §Sections and §Defaults summary](../../../service/specs/06-configuration.md)
**Depends on:** —
**Produces:** Startup accepts an opt-in `grants.id_token` switch and validated nonce/assertion-lifetime durations with compiled defaults that keep the direct grant disabled.
**Pointers:** `crates/core/src/config.rs:9-23`, `crates/core/src/config.rs:45-93`, `crates/core/src/service/mod.rs` (`parse_duration_secs`), `config/default.toml`

## Steps

- [x] Add a serde-defaulted `GrantsConfig` to `AppConfig` with `id_token`, `nonce_ttl`, and `max_assertion_lifetime`, using named compiled defaults of `false`, `10m`, and `1h`.
- [x] Validate both grant durations in `AppConfig::validate` through the existing duration parser and field-prefixing helper so invalid configuration fails at startup with the precise field name.
- [x] Add focused configuration tests for omitted defaults and separately invalid nonce-TTL and maximum-assertion-lifetime values; preserve the existing default TOML unless an explicit disabled section is useful documentation.
  - Kept `config/default.toml` unchanged, per implementation note 1: the compiled defaults already express the disabled-by-default intent.
- [x] Update only the scoped canonical configuration prose/schema material when this implementation PR folds the proposed change; do not merge the change spec or edit unrelated proposed specs.
  - Folded the `[grants]` Sections block and Defaults-summary rows into `.specs/service/specs/06-configuration.md` (page Date bumped). The source change spec was not moved or status-flipped.

## Definition of done

- [x] Missing `[grants]` uses the disabled direct-grant default and the specified duration defaults.
- [x] Invalid `grants.nonce_ttl` and `grants.max_assertion_lifetime` fail validation with their field names; valid values pass.
- [x] New constants have explicit units or documented duration semantics; touched Rust functions meet the assertion and 70-line review gates.
  - Named constants: `DEFAULT_GRANTS_ID_TOKEN` (`false`), `DEFAULT_NONCE_TTL` (`"10m"`), `DEFAULT_MAX_ASSERTION_LIFETIME` (`"1h"`), each documented as a humantime duration string parsed by `service::parse_duration_secs`.
- [x] Meets the repo definition of done (focused tests, negative-space tests, `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and applicable `cargo nextest` tests).
  - Baseline correction: the plan's "three failing config tests" note is stale — merged PR #36 fixed them. Baseline on this branch ran green (387 passed / 27 skipped), and stayed green after this task.
- [x] Reviewable: a reviewer can construct default and malformed configs and confirm direct ID-token service remains opt-in and fails closed on invalid durations.

## Notes

- Tests added to `crates/core/src/config.rs`: `grants_section_deserializes_explicit_values`, `omitted_grants_section_uses_disabled_direct_grant_defaults` (positive + empty-document negative space), `validate_rejects_unparseable_nonce_ttl`, `validate_rejects_unparseable_max_assertion_lifetime`, and `validate_accepts_valid_grant_durations_and_enabled_switch` (including the at-boundary `"0s"` case). `deserialize_default_toml` now also asserts the grants defaults.
- Wave-B contract: task 04 reads `config.grants.max_assertion_lifetime` via `parse_duration_secs`; task 05 mounts `POST /nonce` only when `config.grants.id_token` is true and sizes nonce records with `nonce_ttl`.
