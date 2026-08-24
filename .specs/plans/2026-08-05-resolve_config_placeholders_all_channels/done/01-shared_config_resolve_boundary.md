# Task 01: Shared config resolve boundary

**Status:** Done
**Plan:** [plan.md](../plan.md)
**Implements:** [source spec](../../../changes/merged/2026-08-05-resolve_config_placeholders_all_channels.md) → Proposed changes / Implementation notes 1–3; [06-configuration.md](../../../service/specs/06-configuration.md) → future Loading order / Configuration entry points
**Depends on:** —
**Produces:** one server-owned builder-to-resolved-`AppConfig` tail used by both file-backed configuration and FFI TOML, with environment overrides layered before placeholder resolution and validation.
**Pointers:** `crates/server/src/bootstrap.rs:90-128`; `crates/ffi/src/lib.rs:49-81`; `config` 0.15 `File::from_str` already available through `crates/server/Cargo.toml`.

## Steps

- [x] Extract the post-source-assembly tail from `load_config_from_dir`: build `config::Config`, resolve the merged tree, deserialize the raw shape, validate, and return the runtime config. Make this the only production path containing `try_deserialize` and `validate`.
- [x] Retain file-backed source layering in `load_config_from_dir` (default file, optional selected overlay, structural environment overrides), then delegate to the shared tail.
- [x] Rewrite `parse_config` to build from `File::from_str(toml_str, FileFormat::Toml)` plus the same `OIDC_EXCHANGE__…` `Environment` source, then delegate to the shared tail. Remove the direct `toml::from_str` path.
- [x] Keep `OidcExchange::from_file` as file read → `new`; do not introduce a third configuration pipeline.
- [x] Add focused regression tests proving FFI parsing resolves `internal_api.shared_secret = "${INTERNAL_API_SECRET}"` to the environment value and never returns the literal, and applies `OIDC_EXCHANGE__REGISTRATION__MODE=existing_users_only` to inline TOML.
- [x] Preserve the current file-backed happy path and validation tests; run targeted server/FFI tests plus Rust format/clippy checks.

## Definition of done

- [x] File-backed and inline FFI TOML configuration both traverse one resolve/deserialize/validate implementation; repository search shows no other production `try_deserialize`/`validate` bypass in configuration entry points.
- [x] An FFI caller with set `INTERNAL_API_SECRET` gets the resolved secret, not `${INTERNAL_API_SECRET}`; FFI inline TOML receives the documented structural environment override.
- [x] `OidcExchange::from_file` still delegates through `new`, so Node, Python, and the TypeScript Lambda wrapper inherit the same path without channel-specific patches.
- [x] Positive and negative regression tests are added or preserved; no secret value is asserted via error output.
- [x] `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and targeted tests pass. The final `cargo nextest run --workspace` result was 391 passed, 27 skipped.
