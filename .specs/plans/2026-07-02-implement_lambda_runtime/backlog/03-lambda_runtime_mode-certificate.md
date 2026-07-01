# Done Certificate — Task 03: lambda runtime mode

**Task:** [03-lambda_runtime_mode.md](03-lambda_runtime_mode.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Status:* ☐ unverified

- **O2 — The Lambda branch runs `lambda_http::run(app)` with reconciled errors.**
  - *Claim:* the `main.rs:29-33` log-and-return stub is replaced by `lambda_http::run(app).await`,
    with `lambda_http::Error` reconciled to `Box<dyn std::error::Error>` and no `unwrap`/`expect`
    added.
  - *Evidence to collect:* read the Lambda branch in `crates/server/src/main.rs`; confirm the
    "Lambda runtime detected, but not yet implemented" log and the `// TODO` are gone, the branch
    awaits `lambda_http::run(app)`, and the error propagates via `?`/`From`.
  - *Checks:* resolve `run` at the call site — confirm it is `lambda_http::run`, not a local or
    another crate's `run`; confirm the error path uses `?`/`From`, not `unwrap`/`expect`.
  - *Status:* ☐ unverified

- **O3 — Integration test: same router answers Lambda events.**
  - *Claim:* an API Gateway v2 event routed through `lambda_http` into the shared router returns
    200 + a `keys` array for `/keys`; an unknown-path event returns 404.
  - *Evidence to collect:* run the new Lambda integration test in `crates/server/tests/` — expect
    the `/keys` event → 200 with a JSON body containing `keys`, and the unknown-path event → 404.
  - *Checks:* confirm the test drives `build_router`'s output (the same router as hyper mode), not
    a separately constructed Lambda-only router.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and clippy clean, no new magic-number bound introduced.
  - *Evidence to collect:* run the repo's Rust gates from
    [.specs/development-guidelines.md](../../../development-guidelines.md) §Definition of done —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: `/keys` answered through the `lambda_http` path (Reviewable).**
  - *Claim:* a reviewer runs the Lambda integration test and sees `/keys` return a JWKS body
    through the `lambda_http` path and the unknown-path event return 404.
  - *Evidence to collect:* run the Lambda integration test and observe the `/keys` 200 + JWKS
    result and the 404 negative case reported as passed.
  - *Status:* ☐ unverified

## Regression check

- The hyper server path is unchanged: trace `main.rs` when `AWS_LAMBDA_RUNTIME_API` is unset —
  expect it still binds `server.host:server.port` and calls `axum::serve(listener, app)` with the
  same `app` from `build_router` : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: the `examples/aws-web` API Gateway end-to-end acceptance
(`/token`, `/keys` through a deployed stage) is the change spec's field acceptance and is exercised
by a reviewer against a deployed stack, not in CI; the CI proof here is the in-repo Lambda event
integration test. The per-invocation flush is Task 04, not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
