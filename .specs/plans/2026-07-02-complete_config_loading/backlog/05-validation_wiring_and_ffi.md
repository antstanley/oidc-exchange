# Task 05 — Validation wiring and FFI

**Plan:** [plan.md](../plan.md) · **Certificate:** [05-validation_wiring_and_ffi-certificate.md](05-validation_wiring_and_ffi-certificate.md)

**Implements:** [06-configuration.md](../../../service/specs/06-configuration.md) → Validation at load (the wiring: `load_config` validates after merge+resolution, and "the same validation runs for config supplied as a string through the FFI bindings"); [04-http-api.md](../../../service/specs/04-http-api.md) → Bootstrap step 2 (validate after resolve); [01-ffi-core.md](../../../bindings/specs/01-ffi-core.md) → Responsibilities (config via `new`/`from_file` passes the same load-time validation, rejected as `FfiError` at construction)
**Depends on:** 02, 04
**Produces:** `AppConfig::validate()` called at the tail of `load_config` (after merge, override, and placeholder resolution) and inside `parse_config`, so a server startup and an FFI `OidcExchange::new`/`from_file` both reject invalid config before building anything.
**Pointers:** `crates/server/src/bootstrap.rs:26-47` (`load_config` — add the `validate()` call at the end) and `:50-53` (`parse_config` — add the `validate()` call before returning); `crates/ffi/src/lib.rs:51-72` (`OidcExchange::new` routes through `parse_config`; `from_file` reads then calls `new`), where the `ConfigError` surfaces as `FfiError { code: "CONFIG_ERROR" }`

## Steps

- [ ] Call `config.validate()?` at the end of `load_config`, after the task-04 placeholder resolution, so the server validates the fully-merged, fully-resolved config.
- [ ] Call `config.validate()?` in `parse_config` before returning, so the FFI path validates identically to the server path.
- [ ] Confirm the `ConfigError` propagates through `parse_config` into `OidcExchange::new` as an `FfiError` (`code: "CONFIG_ERROR"`); adjust the error mapping only if the boxed error does not already convert.
- [ ] Add a server-side test that a config with an invalid field (e.g. bad role) makes `load_config` (or a helper over an in-memory config) return `Err`.
- [ ] Add an FFI-path test that `OidcExchange::new` with an invalid TOML config returns an `FfiError` at construction (not at request time).

## Definition of done

- [ ] An invalid config (bad role, bad TTL, malformed allowlist, or served-with-empty-secret) causes `load_config` to return `Err`, before any adapter or router is built.
- [ ] `OidcExchange::new` and `from_file` with the same invalid config return an `FfiError` at construction; a valid config still constructs successfully.
- [ ] Negative-space tests exist for both the server `load_config` path and the FFI construction path; touched functions carry at least two meaningful assertions (or their tests do).
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [ ] Reviewable: run the FFI construction test and a `load_config` test showing an invalid config is rejected at construction/startup, and a valid config passes.
