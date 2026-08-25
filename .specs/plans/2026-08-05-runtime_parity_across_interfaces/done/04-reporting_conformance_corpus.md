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

- Remediation audit (2026-08-23): the original `report.mjs` was only a static table and did not execute any host. That evidence was incomplete and has been replaced rather than promoted.
- `node conformance/report.mjs`: built the Rust conformance driver and production Node/Python bindings, then executed all 72 runner/fixture pairings. Results: native 12/0 qualified, direct FFI 12/0, Node 12/0, Lambda 12/8, ASGI 12/6, WSGI 12/6; zero unqualified field mismatches.
- Qualified inputs are explicit host-fidelity variants only: API Gateway v1 decoded paths/body framing and duplicate-header representation; ASGI without `raw_path` and without Content-Length framing; WSGI without raw-target or ordered-header extensions. All other fixtures use the host's faithful production path.
- The orchestrator rejects a short/missing runner result, compares method/decoded path/query/ordered headers/status, prints shape/fixture/field values, and exits nonzero on every unqualified mismatch. Task 10 still owns promotion from reporting CI to a required merge gate.
