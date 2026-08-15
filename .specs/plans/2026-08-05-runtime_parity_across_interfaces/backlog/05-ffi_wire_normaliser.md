# Task 05 — FFI wire normaliser

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [01-ffi-core.md §Responsibilities](../../../bindings/specs/01-ffi-core.md), [01-ffi-core.md §Public API](../../../bindings/specs/01-ffi-core.md), [01-ffi-core.md §Request flow](../../../bindings/specs/01-ffi-core.md), [01-ffi-core.md §Panic containment](../../../bindings/specs/01-ffi-core.md), [canonical-types.schema.json §HttpRequest and NormalisationLimits](../../../canonical-types.schema.json), source spec §Type changes and §Implementation notes 5
**Depends on:** 02, 03, 04
**Produces:** an async total FFI `handle(WireRequest)` that owns method/path/query/header/body normalisation, publishes limits, and maps shaping failures to native-parity HTTP responses
**Pointers:** `crates/ffi/src/lib.rs:42-148`, `crates/server/src/middleware/base_path.rs:148-160`, `crates/core/src/config.rs:149-160`, `crates/ffi/tests/`

## Steps

- [ ] Introduce `WireRequest`, `TransportHints`, and `NormalisationLimits`; retain a deprecated synchronous legacy shim that splits on the first query delimiter.
- [ ] Build origin-form URI validation, empty-path root normalisation, separate query attachment, ordered `HeaderMap::append`, dropped-invalid-header accounting, and body-limit response mapping in Rust.
- [ ] Reuse the server segment-boundary implementation for base-path stripping rather than copy it.
- [ ] Make `handle` async over the owned router/runtime and catch escaped unwinds into only boundary-meaningful `FfiError` values.
- [ ] Add direct FFI tests for every shaping outcome and compare status/normalised output with the native runner fixtures.

## Definition of done

- [ ] No host-controlled method, path, query, headers, or body panics; malformed values yield the native-equivalent HTTP response where one exists.
- [ ] Empty path becomes `/`, encoded path delimiters remain data, duplicate headers preserve wire order, and body values over `limits().max_body_bytes` yield 413.
- [ ] Base-path stripping calls the shared segment-boundary helper and never creates a second implementation.
- [ ] The deprecated shim remains documented and tested only as the compatibility route; the async wire API is the primary interface.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run direct FFI fixture tests and compare their records with native-server records.
