# Task 01 — Config and panic containment

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [06-configuration.md §[server]](../../../service/specs/06-configuration.md), [04-http-api.md §Middleware stack](../../../service/specs/04-http-api.md), source spec §Implementation notes 1–2
**Depends on:** —
**Produces:** a server whose load-time base-path contract is canonical and whose outer router catches middleware/base-path panics without losing the inner handler-panic request ID behaviour
**Pointers:** `crates/core/src/config.rs:149-160`, `crates/server/src/middleware/base_path.rs:90-145`, `crates/server/src/bootstrap.rs:352-362,1569-1575`

## Steps

- [x] Normalise and validate `server.base_path` at configuration load: unset empty/root, trim one trailing slash policy, and reject missing leading slash.
- [x] Replace host-reachable URI reconstruction panics/assertions in base-path stripping with a fallible pass-through plus structured logging.
- [x] Export or relocate the segment-boundary helper only as needed for the later FFI normaliser; retain one implementation.
- [x] Add the outer `CatchPanicLayer` around the base-path service while retaining the inner guard nearest handlers in production and test routers.
- [x] Add focused configuration and router tests for normalisation, sibling paths, URI-rebuild failure containment, and request-ID retention.

## Definition of done

- [x] Empty/root/trailing-slash/missing-leading-slash base-path cases have explicit load-time tests and the resulting router never asserts on a request path.
- [x] A panic in outer middleware/base-path becomes the standard 500 response, while a handler panic still returns `x-request-id`.
- [x] The segment-boundary regression keeps `/authorize` distinct from an `/auth` prefix.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run the focused config/router tests and inspect the two-guard stack with a forced outer and inner panic.

## Audit outcome

**Verdict:** Complete after fixing one clippy gap (`normalise_base_path` now uses `?`). Audited every DoD item: canonical empty/root/trailing-slash and rejected missing-leading-slash config cases; total URI rebuilding and sibling `/authorize` versus `/auth`; forced inner/outer panic responses and handler request-ID retention; canonical spec links; no new numeric limit in this task.

**Evidence (2026-08-23):** focused nextest runs: config 28 passed / 0 failed / 113 skipped; base path 14 passed / 0 failed / 91 skipped; panic containment 3 passed / 0 failed / 102 skipped. `cargo fmt --all --check` passed; `cargo clippy --workspace -- -D warnings` passed; `cargo nextest run --workspace --no-fail-fast` passed 399 / failed 0 / skipped 27.
