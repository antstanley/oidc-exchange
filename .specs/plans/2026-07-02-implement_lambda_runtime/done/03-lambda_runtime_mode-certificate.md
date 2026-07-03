# Done Certificate — Task 03: lambda runtime mode

**Task:** [03-lambda_runtime_mode.md](03-lambda_runtime_mode.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a build result, or a test result) — not by assertion.

## Premises

- **P1 — Goal.** The task serves the identical router through `lambda_http` when
  `AWS_LAMBDA_RUNTIME_API` is set, translating API Gateway / Function URL / ALB events to tower
  `Service` calls — the same router, middleware, state, and base-path layer as the hyper path.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item,
  in DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the hyper server branch (`main.rs:33-38`) or the shared
  `build_service`/`build_router` calls (`main.rs:25-26`); no Lambda-specific routes or middleware
  fork; `main`'s `Result<(), Box<dyn std::error::Error>>` signature is preserved.

## Obligations

- **O1 — `lambda_http` is a declared dependency and the workspace builds.**
  - *Claim:* `lambda_http` is in `crates/server/Cargo.toml` (default features) and the workspace
    compiles with it.
  - *Evidence to collect:* read `crates/server/Cargo.toml` and confirm the `lambda_http` entry;
    run `cargo build -p oidc-exchange` — expect success.
  - *Status:* ☑ SATISFIED — `crates/server/Cargo.toml:27` declares `lambda_http = "1"`
    (default features); `Cargo.lock` resolves `lambda_http 1.2.1` (+ `lambda_runtime`,
    `aws_lambda_events`, `query_map`, etc.). `oidc-exchange` compiles clean as part of the
    full `cargo nextest run --workspace` build (384 tests compiled + ran) and
    `cargo clippy --workspace -- -D warnings` (Finished, no diagnostics).

- **O2 — The Lambda branch runs `lambda_http::run(app)` with reconciled errors.**
  - *Claim:* the `main.rs:29-33` log-and-return stub is replaced by `lambda_http::run(app).await`,
    with `lambda_http::Error` reconciled to `Box<dyn std::error::Error>` and no `unwrap`/`expect`
    added.
  - *Evidence to collect:* read the Lambda branch in `crates/server/src/main.rs`; confirm the
    "Lambda runtime detected, but not yet implemented" log and the `// TODO` are gone, the branch
    awaits `lambda_http::run(app)`, and the error propagates via `?`/`From`.
  - *Checks:* resolve `run` at the call site — confirm it is `lambda_http::run`, not a local or
    another crate's `run`; confirm the error path uses `?`/`From`, not `unwrap`/`expect`.
  - *Status:* ☑ SATISFIED — `crates/server/src/main.rs:30-43`: the "Lambda runtime detected,
    but not yet implemented" log and the `// TODO: lambda_http::run(app)` are both gone; the
    branch now awaits `lambda_http::run(app)` and propagates failure with
    `.map_err(|err| -> Box<dyn std::error::Error> { err.to_string().into() })?` — via `?`,
    no `unwrap`/`expect` on the run path. Resolution check: the call is fully path-qualified
    `lambda_http::run` (external crate from `Cargo.toml`); the only local `fn run*` in the
    crate is `shutdown::run_with_drain_deadline` (a different name), so no local/other-crate
    `run` shadows it.

- **O3 — Integration test: same router answers Lambda events.**
  - *Claim:* an API Gateway v2 event routed through `lambda_http` into the shared router returns
    200 + a `keys` array for `/keys`; an unknown-path event returns 404.
  - *Evidence to collect:* run the new Lambda integration test in `crates/server/tests/` — expect
    the `/keys` event → 200 with a JSON body containing `keys`, and the unknown-path event → 404.
  - *Checks:* confirm the test drives `build_router`'s output (the same router as hyper mode), not
    a separately constructed Lambda-only router.
  - *Status:* ☑ SATISFIED — `crates/server/tests/lambda.rs` (new file, 2 tests). Ran
    `cargo nextest run -p oidc-exchange --test lambda` → both PASS:
    `apigw_v2_event_for_keys_returns_200_with_jwks` asserts 200 and `json["keys"].is_array()`;
    `apigw_v2_event_for_unknown_path_returns_404` asserts 404. Router check: `build_app()`
    calls `bootstrap::build_router(&config, service)` — the same production router `main.rs`
    hands to `lambda_http::run`, not a Lambda-only fork; events are parsed by
    `lambda_http::request::from_str` (the real `lambda_http` event-to-`http::Request`
    translation) and driven via `tower::ServiceExt::oneshot`. Note: the runtime-API polling
    loop of `lambda_http::run` itself is not exercised (it needs a live Runtime API); the test
    exercises the identical tower `Service` call `run` makes per invocation.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and clippy clean, no new magic-number bound introduced.
  - *Evidence to collect:* run the repo's Rust gates from
    [.specs/development-guidelines.md](../../../development-guidelines.md) §Definition of done —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` → exit 0 (clean);
    `cargo clippy --workspace -- -D warnings` → Finished, no diagnostics;
    `cargo nextest run --workspace` → 384 passed, 27 skipped, 0 failed. No new magic-number
    bound introduced (the diff adds only a dependency, the Lambda run branch, and a test
    with literal event-fixture JSON).

- **O5 — Reviewable: `/keys` answered through the `lambda_http` path (Reviewable).**
  - *Claim:* a reviewer runs the Lambda integration test and sees `/keys` return a JWKS body
    through the `lambda_http` path and the unknown-path event return 404.
  - *Evidence to collect:* run the Lambda integration test and observe the `/keys` 200 + JWKS
    result and the 404 negative case reported as passed.
  - *Status:* ☑ SATISFIED — exercised: `cargo nextest run -p oidc-exchange --test lambda`
    reports `apigw_v2_event_for_keys_returns_200_with_jwks` PASS (`/keys` → 200 with a
    `keys` array JWKS body through the `lambda_http` parse path) and
    `apigw_v2_event_for_unknown_path_returns_404` PASS (unknown path → 404).

## Regression check

- The hyper server path is unchanged: trace `main.rs` when `AWS_LAMBDA_RUNTIME_API` is unset —
  expect it still binds `server.host:server.port` and calls `axum::serve(listener, app)` with the
  same `app` from `build_router` : ☑ PRESERVED — the diff touches only the Lambda (`if`)
  branch of `main.rs`; the `else` branch (now `main.rs:44-76`) still formats
  `{server.host}:{server.port}`, binds a `TcpListener`, and serves the same `app` from
  `build_router` via `axum::serve(listener, app)` (wrapped in the prior task's graceful-drain
  logic). No Lambda-specific route or middleware fork; `main`'s
  `Result<(), Box<dyn std::error::Error>>` signature is unchanged. (Certificate's cited line
  numbers `33-38`/`25-26` predate the graceful-shutdown task that expanded the `else` branch;
  the substance holds.)

## Residue

Notes for the validator: the `examples/aws-web` API Gateway end-to-end acceptance
(`/token`, `/keys` through a deployed stage) is the change spec's field acceptance and is exercised
by a reviewer against a deployed stack, not in CI; the CI proof here is the in-repo Lambda event
integration test. The per-invocation flush is Task 04, not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence (lambda_http declared + workspace builds; the stub
replaced by `lambda_http::run(app).await` with `?`/`map_err` error reconciliation and no
`unwrap`/`expect`; the two-case `tests/lambda.rs` drives API Gateway v2 events through
`lambda_http` into the shared `build_router` output for 200+JWKS on `/keys` and 404 on an
unknown path; fmt/clippy/nextest all clean) and the hyper `else` branch is PRESERVED. The live
Lambda Runtime API polling loop is out of CI scope per Residue; everything provable in-repo is
proven.
