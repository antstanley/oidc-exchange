# Task 06 — Node async binding

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [02-nodejs.md §API](../../../bindings/specs/02-nodejs.md), [02-nodejs.md §Decisions](../../../bindings/specs/02-nodejs.md), [00-overview.md §Shape](../../../bindings/specs/00-overview.md), source spec §Implementation notes 6
**Depends on:** 05
**Produces:** Node `handleRequest` as a Promise-backed wire-request API with limits and a deprecated synchronous compatibility path
**Pointers:** `bindings/nodejs/src/lib.rs:8-97`, `bindings/nodejs/Cargo.toml`, `bindings/nodejs/__tests__/`

## Steps

- [x] Replace path-only Node request fields with raw path, optional query, ordered headers, body, and raw-path hint; expose `limits()`.
- [x] Implement `handleRequest` as a napi async task over FFI `handle`; preserve `handleRequestSync` as a deprecated compatibility method with once-per-process warning.
- [x] Map normalised FFI responses to Node objects without reinterpreting shaping failures as thrown request-build errors.
- [x] Evaluate napi unwind containment against the forced-panic experiment and record the selected feature/behaviour.
- [x] Add Node tests for Promise behaviour, malformed wire request responses, duplicates, body limit publication, and sync deprecation compatibility.

## Definition of done

- [x] `handleRequest` returns and resolves a Promise without holding the host thread through router I/O.
- [x] Node callers receive native-parity HTTP responses for malformed request shaping; only boundary failures surface as binding errors.
- [x] Header order, raw path/query separation, and `limits().maxBodyBytes` cross the binding unchanged.
- [x] The napi unwind decision is backed by an executable forced-panic test or documented measured limitation.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: await `handleRequest` in binding tests and inspect Promise, parity-response, and legacy-sync cases.

## Evidence

- `npx -y pnpm@11.9.0 --ignore-workspace ... build/lint/typecheck/test`: build succeeded; lint 0 warnings/errors; typecheck clean; 7/7 tests passed.
- `cargo check -p oidc-exchange-nodejs`: passed. The async work is a napi `AsyncTask`, so router work executes on libuv's worker pool rather than Node's host thread.
- Malformed method/path and over-limit body resolve to 400/413 responses; duplicate ordered headers, separate query bytes, the 32-byte configured limit, and deprecated sync calls are covered.
- Unwind decision: no napi `noop`/abort-on-panic feature is enabled. FFI catches request-normalisation panics and returns the stable `PANIC` boundary error. A forced router panic cannot currently be injected through the public binding, so unwind containment beyond that measured FFI boundary remains a documented test limitation rather than an unverified claim.
