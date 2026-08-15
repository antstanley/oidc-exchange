# Task 06 — Node async binding

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [02-nodejs.md §API](../../../bindings/specs/02-nodejs.md), [02-nodejs.md §Decisions](../../../bindings/specs/02-nodejs.md), [00-overview.md §Shape](../../../bindings/specs/00-overview.md), source spec §Implementation notes 6
**Depends on:** 05
**Produces:** Node `handleRequest` as a Promise-backed wire-request API with limits and a deprecated synchronous compatibility path
**Pointers:** `bindings/nodejs/src/lib.rs:8-97`, `bindings/nodejs/Cargo.toml`, `bindings/nodejs/__tests__/`

## Steps

- [ ] Replace path-only Node request fields with raw path, optional query, ordered headers, body, and raw-path hint; expose `limits()`.
- [ ] Implement `handleRequest` as a napi async task over FFI `handle`; preserve `handleRequestSync` as a deprecated compatibility method with once-per-process warning.
- [ ] Map normalised FFI responses to Node objects without reinterpreting shaping failures as thrown request-build errors.
- [ ] Evaluate napi unwind containment against the forced-panic experiment and record the selected feature/behaviour.
- [ ] Add Node tests for Promise behaviour, malformed wire request responses, duplicates, body limit publication, and sync deprecation compatibility.

## Definition of done

- [ ] `handleRequest` returns and resolves a Promise without holding the host thread through router I/O.
- [ ] Node callers receive native-parity HTTP responses for malformed request shaping; only boundary failures surface as binding errors.
- [ ] Header order, raw path/query separation, and `limits().maxBodyBytes` cross the binding unchanged.
- [ ] The napi unwind decision is backed by an executable forced-panic test or documented measured limitation.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: await `handleRequest` in binding tests and inspect Promise, parity-response, and legacy-sync cases.
