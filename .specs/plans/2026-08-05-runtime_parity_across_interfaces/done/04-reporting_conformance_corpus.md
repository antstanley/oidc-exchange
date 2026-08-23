# Task 04 — Reporting conformance corpus

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [01-ffi-core.md §Conformance corpus](../../../bindings/specs/01-ffi-core.md), [00-overview.md §Decisions](../../../bindings/specs/00-overview.md), source spec §Implementation notes 4
**Depends on:** 01, 03
**Produces:** a shared fixture corpus and non-blocking CI job that reports normalisation disagreements before the breaking migration removes them
**Pointers:** `.github/workflows/ci.yml:1-120`, `crates/ffi/tests/`, `bindings/nodejs/__tests__/`, `bindings/lambda/__tests__/`, `bindings/python/tests/`

## Steps

- [x] Define transport-agnostic fixture and expected-normalisation formats under `conformance/corpus/` with declared `TransportHints` qualifications.
- [x] Add runners for native server, direct FFI, Node plus synthetic Lambda events, and Python ASGI/WSGI under a pinned server.
- [x] Seed every source-specified edge fixture: encoded slash/dot-dot/question/hash, duplicate forwarded-for order, base-path siblings, empty path, malformed/huge content length, and one byte over cap.
- [x] Add a `conformance` CI job in reporting mode that stores or prints baseline disagreements without gating unrelated work.
- [x] Document each currently unachievable shape expectation as a qualification, never a skip.

## Definition of done

- [x] Every named source fixture has a transport-neutral expected record and runs through all applicable shapes.
- [x] Reporting output identifies per-shape differences in method, decoded path, query, ordered headers, and status.
- [x] Known Python/Lambda limitations are declared through hints/qualifications rather than suppressed tests.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run the conformance job and inspect the recorded baseline disagreement report.

## Audit / evidence

- `node conformance/report.mjs`: 12 fixtures across 6 declared shapes; baseline differences native 0, FFI 4, Node 4, Lambda 4, ASGI 6, WSGI 7. Reporting mode remains intentionally non-gating until task 10.
- `cargo test -p oidc-exchange-ffi`: 8 passed, 0 failed.
- Qualification: this first reporting slice records shape-specific replay disagreements and transport limitations in one shared executable report. Host migrations and merge-gated agreement remain tasks 06–10; no divergence was hidden or converted into a skip.
