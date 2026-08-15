# Task 02 — Total Python sync boundary

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [03-python.md §API](../../../bindings/specs/03-python.md), [01-ffi-core.md §Responsibilities](../../../bindings/specs/01-ffi-core.md), source spec §Implementation notes 1
**Depends on:** —
**Produces:** a direct Python synchronous boundary that accepts an empty path and reports missing/ill-typed request fields as typed Python errors rather than asserting
**Pointers:** `bindings/python/src/lib.rs:46-121`, `bindings/python/tests/test_handle_request.py`

## Steps

- [ ] Remove request-derived `assert!` calls from `handle_request_sync`, treating an empty path as valid root input.
- [ ] Preserve typed missing-field extraction and make ill-typed method/path/body failures explicit `ValueError`-class boundary errors.
- [ ] Keep GIL release around the blocking legacy FFI call until task 07 replaces the async surface.
- [ ] Add direct binding tests for empty path, missing required fields, and invalid field types without a process panic.

## Definition of done

- [ ] An empty direct Python path reaches the existing FFI path without assertion and is covered by a regression test.
- [ ] Missing or ill-typed request fields yield documented typed errors; no host-supplied value reaches `assert!` or `unwrap`.
- [ ] Existing GIL-release coverage remains valid for the synchronous compatibility path.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the direct Python binding tests showing empty-path success and typed invalid-input failures.
