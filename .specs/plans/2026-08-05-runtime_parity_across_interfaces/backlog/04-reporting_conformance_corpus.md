# Task 04 — Reporting conformance corpus

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [01-ffi-core.md §Conformance corpus](../../../bindings/specs/01-ffi-core.md), [00-overview.md §Decisions](../../../bindings/specs/00-overview.md), source spec §Implementation notes 4
**Depends on:** 01, 03
**Produces:** a shared fixture corpus and non-blocking CI job that reports normalisation disagreements before the breaking migration removes them
**Pointers:** `.github/workflows/ci.yml:1-120`, `crates/ffi/tests/`, `bindings/nodejs/__tests__/`, `bindings/lambda/__tests__/`, `bindings/python/tests/`

## Steps

- [ ] Define transport-agnostic fixture and expected-normalisation formats under `conformance/corpus/` with declared `TransportHints` qualifications.
- [ ] Add runners for native server, direct FFI, Node plus synthetic Lambda events, and Python ASGI/WSGI under a pinned server.
- [ ] Seed every source-specified edge fixture: encoded slash/dot-dot/question/hash, duplicate forwarded-for order, base-path siblings, empty path, malformed/huge content length, and one byte over cap.
- [ ] Add a `conformance` CI job in reporting mode that stores or prints baseline disagreements without gating unrelated work.
- [ ] Document each currently unachievable shape expectation as a qualification, never a skip.

## Definition of done

- [ ] Every named source fixture has a transport-neutral expected record and runs through all applicable shapes.
- [ ] Reporting output identifies per-shape differences in method, decoded path, query, ordered headers, and status.
- [ ] Known Python/Lambda limitations are declared through hints/qualifications rather than suppressed tests.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the conformance job and inspect the recorded baseline disagreement report.
