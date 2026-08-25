# Task 02 — Total Python sync boundary

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [03-python.md §API](../../../bindings/specs/03-python.md), [01-ffi-core.md §Responsibilities](../../../bindings/specs/01-ffi-core.md), source spec §Implementation notes 1
**Depends on:** —
**Produces:** a direct Python synchronous boundary that accepts an empty path and reports missing/ill-typed request fields as typed Python errors rather than asserting
**Pointers:** `bindings/python/src/lib.rs:46-121`, `bindings/python/tests/test_handle_request.py`

## Steps

- [x] Remove request-derived `assert!` calls from `handle_request_sync`, treating an empty path as valid root input.
- [x] Preserve typed missing-field extraction and make ill-typed method/path/body failures explicit `ValueError`-class boundary errors.
- [x] Keep GIL release around the blocking legacy FFI call until task 07 replaces the async surface.
- [x] Add direct binding tests for empty path, missing required fields, and invalid field types without a process panic.

## Definition of done

- [x] An empty direct Python path reaches the existing FFI path without assertion and is covered by a regression test.
- [x] Missing or ill-typed request fields yield documented typed errors; no host-supplied value reaches `assert!` or `unwrap`.
- [x] Existing GIL-release coverage remains valid for the synchronous compatibility path.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run the direct Python binding tests showing empty-path success and typed invalid-input failures.

## Audit outcome

**Verdict:** Complete. Removed host-derived assertions, preserved KeyError for missing required fields, added explicit ValueError mapping for ill-typed method/path/body/headers, retained `py.allow_threads`, and verified an empty path reaches the legacy FFI boundary (which returns its typed request-build RuntimeError rather than panicking). No numeric limit was introduced.

**Evidence (2026-08-23):** direct Python tests 16 passed / 0 failed; ruff format/check passed; pyright 0 errors / 0 warnings; `cargo fmt --all --check` passed; `cargo clippy --workspace -- -D warnings` passed; `cargo nextest run --workspace --no-fail-fast` passed 399 / failed 0 / skipped 27.
