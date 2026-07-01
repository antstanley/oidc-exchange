# Task 01 — run_migrations config

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-run_migrations_config-certificate.md](01-run_migrations_config-certificate.md)

**Implements:** [06-configuration.md](../../../service/specs/06-configuration.md) §`[repository]` (users + sessions) — add `run_migrations?` to the `[repository.postgres]` keys
**Depends on:** —
**Produces:** `[repository.postgres] run_migrations` deserializes into `PostgresConfig.run_migrations: Option<bool>` — present maps to `Some(true|false)`, absent to `None` (later resolved as `true`); 06-configuration.md documents the key and its default
**Pointers:** `crates/core/src/config.rs:148-152` (`PostgresConfig`); spec page `.specs/service/specs/06-configuration.md:67-71` (`[repository]` section) and its Assumptions/Decisions block

## Steps

- [ ] Add `run_migrations: Option<bool>` to `PostgresConfig` in `crates/core/src/config.rs`, immediately after `max_connections`.
- [ ] Confirm the field deserializes from TOML without breaking the existing `[repository.postgres] { url, max_connections? }` parse (the struct is not `#[serde(default)]`, so keep `url` required and the new field optional).
- [ ] Add a config-deserialization unit test in `crates/core` covering three cases: `run_migrations = false` → `Some(false)`, `run_migrations = true` → `Some(true)`, and the key absent → `None`.
- [ ] Update `06-configuration.md` §`[repository]` to list `run_migrations?` in the `[repository.postgres]` keys and note `run_migrations` defaults to `true` (set `false` for locked-down databases where DDL is applied out-of-band); bump the page's `**Date:**`.

## Definition of done

- [ ] `PostgresConfig` carries `run_migrations: Option<bool>` and a TOML `[repository.postgres]` block with the key deserializes to the matching `Some(_)`; without the key it deserializes to `None`.
- [ ] Negative-space test: a `[repository.postgres]` block that omits `run_migrations` still deserializes (field is `None`, not a parse error), asserting the "absent → default" contract at the config layer.
- [ ] `06-configuration.md` documents `run_migrations?` on `[repository.postgres]` and its `true` default; the page `**Date:**` is bumped.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer loads a TOML with `[repository.postgres] run_migrations = false` and one without the key, and observes the deserialized `PostgresConfig` carrying `Some(false)` and `None` respectively via the new test.
