# Task 01: Shared config resolve boundary

**Plan:** [plan.md](../plan.md)
**Implements:** [source spec](../../../changes/2026-08-05-resolve_config_placeholders_all_channels.md) → Proposed changes / Implementation notes 1–3; [06-configuration.md](../../../service/specs/06-configuration.md) → future Loading order / Configuration entry points
**Depends on:** —
**Produces:** one server-owned builder-to-resolved-`AppConfig` tail used by both file-backed configuration and FFI TOML, with environment overrides layered before placeholder resolution and validation.
**Pointers:** `crates/server/src/bootstrap.rs:90-128`; `crates/ffi/src/lib.rs:49-81`; `config` 0.15 `File::from_str` already available through `crates/server/Cargo.toml`.

## Steps

- [ ] Extract the post-source-assembly tail from `load_config_from_dir`: build `config::Config`, resolve the merged tree, deserialize the raw shape, validate, and return the runtime config. Make this the only production path containing `try_deserialize` and `validate`.
- [ ] Retain file-backed source layering in `load_config_from_dir` (default file, optional selected overlay, structural environment overrides), then delegate to the shared tail.
- [ ] Rewrite `parse_config` to build from `File::from_str(toml_str, FileFormat::Toml)` plus the same `OIDC_EXCHANGE__…` `Environment` source, then delegate to the shared tail. Remove the direct `toml::from_str` path.
- [ ] Keep `OidcExchange::from_file` as file read → `new`; do not introduce a third configuration pipeline.
- [ ] Add focused regression tests proving FFI parsing resolves `internal_api.shared_secret = "${INTERNAL_API_SECRET}"` to the environment value and never returns the literal, and applies `OIDC_EXCHANGE__REGISTRATION__MODE=existing_users_only` to inline TOML.
- [ ] Preserve the current file-backed happy path and validation tests; run targeted server/FFI tests plus Rust format/clippy checks.

## Definition of done

- [ ] File-backed and inline FFI TOML configuration both traverse one resolve/deserialize/validate implementation; repository search shows no other production `try_deserialize`/`validate` bypass in configuration entry points.
- [ ] An FFI caller with set `INTERNAL_API_SECRET` gets the resolved secret, not `${INTERNAL_API_SECRET}`; FFI inline TOML receives the documented structural environment override.
- [ ] `OidcExchange::from_file` still delegates through `new`, so Node, Python, and the TypeScript Lambda wrapper inherit the same path without channel-specific patches.
- [ ] Positive and negative regression tests are added or preserved; no secret value is asserted via error output.
- [ ] `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and targeted tests pass. If `cargo test --workspace` is run, record the known three missing `providers.*.adapter` test failures without changing them.
