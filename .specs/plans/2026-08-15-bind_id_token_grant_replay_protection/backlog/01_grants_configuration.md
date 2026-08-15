# Task 01 — Grants configuration

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted at user request

**Implements:** [`.specs/changes/2026-08-05-bind_id_token_grant_replay_protection.md` §Proposed changes → `06-configuration.md`](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md), [§Implementation notes 1](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md); [06-configuration.md §Sections and §Defaults summary](../../../service/specs/06-configuration.md)
**Depends on:** —
**Produces:** Startup accepts an opt-in `grants.id_token` switch and validated nonce/assertion-lifetime durations with compiled defaults that keep the direct grant disabled.
**Pointers:** `crates/core/src/config.rs:9-23`, `crates/core/src/config.rs:45-93`, `crates/core/src/service/mod.rs` (`parse_duration_secs`), `config/default.toml`

## Steps

- [ ] Add a serde-defaulted `GrantsConfig` to `AppConfig` with `id_token`, `nonce_ttl`, and `max_assertion_lifetime`, using named compiled defaults of `false`, `10m`, and `1h`.
- [ ] Validate both grant durations in `AppConfig::validate` through the existing duration parser and field-prefixing helper so invalid configuration fails at startup with the precise field name.
- [ ] Add focused configuration tests for omitted defaults and separately invalid nonce-TTL and maximum-assertion-lifetime values; preserve the existing default TOML unless an explicit disabled section is useful documentation.
- [ ] Update only the scoped canonical configuration prose/schema material when this implementation PR folds the proposed change; do not merge the change spec or edit unrelated proposed specs.

## Definition of done

- [ ] Missing `[grants]` uses the disabled direct-grant default and the specified duration defaults.
- [ ] Invalid `grants.nonce_ttl` and `grants.max_assertion_lifetime` fail validation with their field names; valid values pass.
- [ ] New constants have explicit units or documented duration semantics; touched Rust functions meet the assertion and 70-line review gates.
- [ ] Meets the repo definition of done (focused tests, negative-space tests, `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and applicable `cargo nextest` tests; record but do not fix the known unrelated three config-test baseline failures).
- [ ] Reviewable: a reviewer can construct default and malformed configs and confirm direct ID-token service remains opt-in and fails closed on invalid durations.
