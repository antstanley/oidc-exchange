# Task 03 — lambda runtime mode

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-lambda_runtime_mode-certificate.md](03-lambda_runtime_mode-certificate.md)

**Implements:** [service/specs/04-http-api.md](../../../service/specs/04-http-api.md) §Bootstrap (step 6 — "`AWS_LAMBDA_RUNTIME_API` present → the router is served through `lambda_http::run` as a tower service"); [service/specs/00-overview.md](../../../service/specs/00-overview.md) (run as an axum server or an AWS Lambda from one binary)
**Depends on:** 02
**Produces:** when `AWS_LAMBDA_RUNTIME_API` is set, the binary serves the identical router (middleware, state, and the base-path layer of Task 02) through `lambda_http`, translating API Gateway REST/HTTP-API, Function URL, and ALB events to tower `Service` calls
**Pointers:** `crates/server/src/main.rs:29-33` (the log-and-return Lambda stub to replace); `crates/server/Cargo.toml:6-26` (dependencies — `lambda_http` is absent, grep-confirmed); `crates/server/src/bootstrap.rs:109-138` (`build_router` produces the `app` passed to `lambda_http::run`)

## Steps

- [x] Add `lambda_http` to `crates/server/Cargo.toml` with default features (they cover the API Gateway and ALB event types); confirm the workspace resolves and builds.
- [x] Replace the `main.rs:29-33` log-and-return branch with `lambda_http::run(app).await`, reconciling `lambda_http::Error` with `main`'s `Box<dyn std::error::Error>` return (propagate via `?`/`From`, no `unwrap`/`expect` on the run path).
- [x] Keep the hyper branch and the shared `build_router`/`build_service` calls unchanged — no Lambda-specific routes or middleware fork.
- [x] Add an integration test in `crates/server/tests/` that drives an API Gateway HTTP-API (v2) event through `lambda_http` into `build_router`'s output and asserts `GET /keys` returns 200 with a `keys` array; pair it with an unknown-path event that returns 404.

## Definition of done

- [x] `lambda_http` is a declared dependency of the server crate and the workspace builds with it.
- [x] The Lambda branch runs `lambda_http::run(app).await` (the "not yet implemented" log-and-return is gone), with the error type reconciled to `Box<dyn std::error::Error>` and no `unwrap`/`expect` added.
- [x] Integration test: an API Gateway v2 event routed through `lambda_http` into the shared router returns 200 + JWKS for `/keys`; a negative-space event for an unknown path returns 404.
- [x] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, named-constant limits — see plan.md baseline).
- [x] Reviewable: run the Lambda integration test and observe `/keys` returning a JWKS body through the `lambda_http` path and the unknown-path event returning 404.
