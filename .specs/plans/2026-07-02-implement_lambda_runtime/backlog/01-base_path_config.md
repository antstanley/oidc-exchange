# Task 01 — base_path config field

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-base_path_config-certificate.md](01-base_path_config-certificate.md)

**Implements:** [service/specs/06-configuration.md](../../../service/specs/06-configuration.md) §Sections → `[server]` (the optional `base_path` key); change spec §Type changes (`ServerConfig` gains `base_path: Option<String>`)
**Depends on:** —
**Produces:** the `server.base_path` TOML key deserializes into `ServerConfig` as `Option<String>`, defaulting to `None` when the key is absent — the data the strip layer (Task 02) consumes
**Pointers:** `crates/core/src/config.rs:23-41` (`ServerConfig` struct and its `Default` impl); the existing config round-trip tests at `crates/core/src/config.rs:257-429`

## Steps

- [ ] Add `pub base_path: Option<String>` to `ServerConfig` (`crates/core/src/config.rs:23-30`), keeping the struct `#[serde(default)]` so an omitted key deserializes to `None`.
- [ ] Set `base_path: None` in the `Default` impl (`crates/core/src/config.rs:32-41`); add a short `// why` comment noting it exists for API Gateway stages / mount prefixes.
- [ ] Extend `deserialize_default_toml` to assert `config.server.base_path.is_none()` (the absent-key path).
- [ ] Add a positive test: deserialize a `[server]` block with `base_path = "/prod"` and assert `config.server.base_path.as_deref() == Some("/prod")`.

## Definition of done

- [ ] `server.base_path = "/prod"` deserializes to `Some("/prod")`, and a config omitting the key yields `None` (paired positive/negative cases in `crates/core/src/config.rs` tests).
- [ ] The `Default` impl returns `base_path: None`, verified by the default-TOML test's negative-space assertion.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run `cargo nextest run -p oidc-exchange-core config` and observe both the `base_path = "/prod"` present case and the absent-key `None` case pass.
