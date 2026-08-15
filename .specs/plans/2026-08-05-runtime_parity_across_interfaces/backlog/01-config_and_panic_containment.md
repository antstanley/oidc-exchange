# Task 01 — Config and panic containment

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [06-configuration.md §[server]](../../../service/specs/06-configuration.md), [04-http-api.md §Middleware stack](../../../service/specs/04-http-api.md), source spec §Implementation notes 1–2
**Depends on:** —
**Produces:** a server whose load-time base-path contract is canonical and whose outer router catches middleware/base-path panics without losing the inner handler-panic request ID behaviour
**Pointers:** `crates/core/src/config.rs:149-160`, `crates/server/src/middleware/base_path.rs:90-145`, `crates/server/src/bootstrap.rs:352-362,1569-1575`

## Steps

- [ ] Normalise and validate `server.base_path` at configuration load: unset empty/root, trim one trailing slash policy, and reject missing leading slash.
- [ ] Replace host-reachable URI reconstruction panics/assertions in base-path stripping with a fallible pass-through plus structured logging.
- [ ] Export or relocate the segment-boundary helper only as needed for the later FFI normaliser; retain one implementation.
- [ ] Add the outer `CatchPanicLayer` around the base-path service while retaining the inner guard nearest handlers in production and test routers.
- [ ] Add focused configuration and router tests for normalisation, sibling paths, URI-rebuild failure containment, and request-ID retention.

## Definition of done

- [ ] Empty/root/trailing-slash/missing-leading-slash base-path cases have explicit load-time tests and the resulting router never asserts on a request path.
- [ ] A panic in outer middleware/base-path becomes the standard 500 response, while a handler panic still returns `x-request-id`.
- [ ] The segment-boundary regression keeps `/authorize` distinct from an `/auth` prefix.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the focused config/router tests and inspect the two-guard stack with a forced outer and inner panic.
