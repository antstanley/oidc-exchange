# Task 02 — base_path strip layer

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-base_path_strip_layer-certificate.md](02-base_path_strip_layer-certificate.md)

**Implements:** [service/specs/04-http-api.md](../../../service/specs/04-http-api.md) §Bootstrap (step 6 — "both strip a configured `server.base_path` prefix from incoming request paths before routing"); [service/specs/06-configuration.md](../../../service/specs/06-configuration.md) §`[server]` (`base_path` honored in both modes)
**Depends on:** 01
**Produces:** in plain server mode, a request to `/prod/health` routes to the health handler when `server.base_path = "/prod"`; a shared tower layer in `build_router` that both runtimes will strip through
**Pointers:** `crates/server/src/bootstrap.rs:109-138` (`build_router`, where the middleware stack is applied); the E2E harness style in `crates/server/tests/routes.rs:1-40` (`tower::ServiceExt::oneshot` over the real router)

## Steps

- [ ] Add a single strip-prefix layer in `build_router` (`crates/server/src/bootstrap.rs`) that, when `config.server.base_path` is `Some(prefix)`, rewrites the request URI path to drop a leading `prefix` before the router matches; when `None`, the layer is not applied (or is a pass-through) so paths are unchanged.
- [ ] Apply the layer so it wraps the router and runs before route matching, on the one shared code path used by both Lambda and hyper modes (do not fork a Lambda-only branch).
- [ ] Assert the layer's preconditions defensively: the prefix is treated as a whole leading path segment boundary (strip `/prod` from `/prod/health`, not from `/production`); a request whose path does not start with the prefix is left unmodified.
- [ ] Add an E2E test in `crates/server/tests/` driving the real router via `oneshot`: `base_path = Some("/prod")` → `GET /prod/health` returns 200; a request without the prefix and a request equal to the bare prefix are handled per the boundary rule; `base_path = None` → `GET /health` returns 200 unchanged.

## Definition of done

- [ ] With `base_path = Some("/prod")`, `GET /prod/health` routes to the health handler (200); with `base_path = None`, `GET /health` is unchanged (200) and no rewrite occurs.
- [ ] Negative-space test: with `base_path = Some("/prod")`, a request lacking the prefix (e.g. `GET /health`) is not double-stripped and resolves per the boundary rule (not a false 200 on a mismatched prefix such as `/production...`).
- [ ] The layer is applied once on the shared `build_router` path (verified by reading `bootstrap.rs`), with ≥2 meaningful assertions in the layer function.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the server-crate E2E base-path test and observe `/prod/health` → 200 with `base_path = "/prod"` and the no-prefix mismatch case not falsely routed.
