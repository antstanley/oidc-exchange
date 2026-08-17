# Task 03: Config check CLI

**Status:** Done
**Plan:** [plan.md](../plan.md)
**Implements:** [source spec](../../../changes/merged/2026-08-05-resolve_config_placeholders_all_channels.md) → Pre-flight check / Implementation notes 7–8; [04-http-api.md](../../../service/specs/04-http-api.md) → future Bootstrap steps 1–2
**Depends on:** 02
**Produces:** `oidc-exchange config check [--dir <config-dir>] [--file <path>]`, a preflight-only caller of the shared resolver that emits a redacted summary on success and exits non-zero on `ConfigError`.
**Pointers:** `crates/server/src/main.rs:10-85`; `crates/server/src/bootstrap.rs:58-128`; redacting `Debug` implementations in `crates/core/src/config.rs`; no existing argument-parsing dependency.

## Steps

- [x] Define and implement the minimal CLI grammar for `config check`, `--dir`, and `--file`, including mutually exclusive/invalid-argument handling; decide whether a small parser dependency or explicit bounded argument parsing best fits the existing binary and document the choice in the PR.
- [x] Make the file-backed loader callable by the CLI and add a single-file loader using the same inline/file source shape as FFI. Both must delegate to task 01's shared resolver, not reimplement source merging or validation.
- [x] For `--dir` (default `config/`), layer default, selected overlay, and structural environment overrides. For `--file`, layer that named document plus structural overrides, matching FFI's source semantics.
- [x] On success, print only the configuration through established redacting `Debug` output and exit before telemetry initialization, adapter construction, router creation, socket binding, or writes. On resolution/validation failure, return non-zero and preserve the safe diagnostic.
- [x] Add CLI-level tests or a testable command runner covering directory and file successes, unset-placeholder non-zero failure, invalid argument combinations, and output absence of the raw secret.
- [x] Verify `--version` remains unchanged and Rust Lambda/server startup continues to load configuration before runtime selection.

## Definition of done

- [x] `oidc-exchange config check` accepts the documented forms, uses the shared resolve exactly once, and does not construct adapters, bind a socket, initialize telemetry, or write state.
- [x] An unset placeholder exits non-zero and names the safe failure context without printing its raw secret; a successful run prints a redacted summary with `internal_api.shared_secret` and `user_sync.webhook.secret` protected.
- [x] `--dir` and `--file` reflect their respective source shapes, including `OIDC_EXCHANGE__…` overrides, and invalid CLI combinations fail deterministically.
- [x] Positive and negative command tests pass along with `cargo fmt --all --check` and `cargo clippy --workspace -- -D warnings`; the final `cargo nextest run --workspace` result was 391 passed, 27 skipped.
