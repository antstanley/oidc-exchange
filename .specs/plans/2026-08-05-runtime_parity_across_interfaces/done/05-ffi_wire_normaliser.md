# Task 05 — FFI wire normaliser

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [01-ffi-core.md §Responsibilities](../../../bindings/specs/01-ffi-core.md), [01-ffi-core.md §Public API](../../../bindings/specs/01-ffi-core.md), [01-ffi-core.md §Request flow](../../../bindings/specs/01-ffi-core.md), [01-ffi-core.md §Panic containment](../../../bindings/specs/01-ffi-core.md), [canonical-types.schema.json §HttpRequest and NormalisationLimits](../../../canonical-types.schema.json), source spec §Type changes and §Implementation notes 5
**Depends on:** 02, 03, 04
**Produces:** an async total FFI `handle(WireRequest)` that owns method/path/query/header/body normalisation, publishes limits, and maps shaping failures to native-parity HTTP responses
**Pointers:** `crates/ffi/src/lib.rs:42-148`, `crates/server/src/middleware/base_path.rs:148-160`, `crates/core/src/config.rs:149-160`, `crates/ffi/tests/`

## Steps

- [x] Introduce `WireRequest`, `TransportHints`, and `NormalisationLimits`; retain a deprecated synchronous legacy shim that splits on the first query delimiter.
- [x] Build origin-form URI validation, empty-path root normalisation, separate query attachment, ordered `HeaderMap::append`, dropped-invalid-header accounting, and body-limit response mapping in Rust.
- [x] Reuse the server segment-boundary implementation for base-path stripping rather than copy it.
- [x] Make `handle` async over the owned router/runtime and catch escaped unwinds into only boundary-meaningful `FfiError` values.
- [x] Add direct FFI tests for every shaping outcome and compare status/normalised output with the native runner fixtures.

## Definition of done

- [x] No host-controlled method, path, query, headers, or body panics; malformed values yield the native-equivalent HTTP response where one exists.
- [x] Empty path becomes `/`, encoded path delimiters remain data, duplicate headers preserve wire order, and body values over `limits().max_body_bytes` yield 413.
- [x] Base-path stripping calls the shared segment-boundary helper and never creates a second implementation.
- [x] The deprecated shim remains documented and tested only as the compatibility route; the async wire API is the primary interface.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run direct FFI fixture tests and compare their records with native-server records.

## Audit / evidence

- `cargo test -p oidc-exchange-ffi`: 14 passed, 0 failed; boundaries cover empty/non-origin paths, encoded delimiters, invalid/duplicate headers, invalid methods, exact/one-over body limits, and first-query shim splitting.
- `cargo fmt --all --check`: clean. `cargo clippy --workspace --all-targets -- -D warnings`: clean. `cargo nextest run --workspace --no-fail-fast`: 405 passed, 0 failed, 27 skipped.
- `node conformance/report.mjs`: 12 fixtures / 6 shapes; reporting baseline preserved. Ruff lint/format and Pyright: clean (0 errors/warnings).
- Node/Lambda lint/typecheck could not start because Corepack selected pnpm 11.20.0 while the repository requires 11.9.0; exact failure: `This project is configured to use 11.9.0 of pnpm. Your current pnpm is v11.20.0`.
- The router remains the sole base-path normaliser and invokes the exported `strip_prefix_at_segment_boundary`; the FFI does not splice or strip paths. Binding migrations intentionally remain tasks 06–08.
