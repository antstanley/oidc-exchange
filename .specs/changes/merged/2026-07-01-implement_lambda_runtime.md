# Change: Implement Lambda runtime mode in the server binary

**Status:** Merged · **Date:** 2026-07-01 · **Merged:** 2026-07-03 · **Owner:** Ant Stanley · **Target:** crates/server

Make the server binary actually run as an AWS Lambda: when `AWS_LAMBDA_RUNTIME_API` is present,
serve the same axum router through `lambda_http` instead of logging "not yet implemented" and
exiting 0. This change covers the Rust binary path only; the TypeScript
[`@oidc-exchange/lambda`](../bindings/specs/04-lambda.md) binding is a separate, already
implemented mechanism and is out of scope.

---

## Motivation

`main.rs` detects `AWS_LAMBDA_RUNTIME_API`, logs "Lambda runtime detected, but not yet
implemented", and returns `Ok(())`. Deployed as a Lambda, initialisation "succeeds" and the
process immediately exits with status 0 — the runtime restarts it in a loop and every
invocation fails. The canonical spec is ahead of the code here:
[00-overview.md](../service/specs/00-overview.md) lists "run as an axum server or an AWS Lambda
from one binary" among the design goals, and [04-http-api.md](../service/specs/04-http-api.md)
step 6 states "`AWS_LAMBDA_RUNTIME_API` present → Lambda mode" with no divergence note. The
shipped `examples/aws-web` stack deploys this exact binary as a `provided.al2023` function, so
the flagship AWS example is broken end to end.

The bindings spec's Lambda page describes a different mechanism — a pure-TypeScript event
adapter over the Node binding (`createHandler`), which works today. This change therefore
scopes to the server binary: wrap the router `bootstrap::build_router` already produces with
`lambda_http`, which speaks the runtime API and translates API Gateway / Function URL / ALB
events to tower `Service` calls — the same router, middleware, and state as the hyper path.

---

## Affected spec pages

| Canonical page                                                           | Nature of change                                                                                                             |
| ------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | Bootstrap step 6 already reads correctly for the end state (spec is ahead of the code). Expand it with what Lambda mode does |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | Add the optional `base_path` key to the `[server]` section                                                          |
| [`.specs/service/specs/00-overview.md`](../service/specs/00-overview.md) | Already correct for the end state ("axum server or AWS Lambda from one binary"); no delta                                    |
| [`.specs/bindings/specs/04-lambda.md`](../bindings/specs/04-lambda.md)   | Not affected — documents the TS event adapter over the Node binding, a distinct mechanism                                    |

---

## Proposed changes

### `.specs/service/specs/04-http-api.md` → Bootstrap (Modify)

> 6. Detect runtime: `AWS_LAMBDA_RUNTIME_API` present → the router is served through
>    `lambda_http::run` as a tower service, accepting API Gateway REST/HTTP-API, Function URL,
>    and ALB events; otherwise bind `server.host:server.port` and serve over hyper. Both paths
>    run the identical router, middleware stack, and `AppState`, and both strip a configured
>    `server.base_path` prefix from incoming request paths before routing
>    ([06-configuration.md](06-configuration.md)) — covering API Gateway stages and mount
>    prefixes. In Lambda mode, telemetry and blocking audit writes flush synchronously before
>    each invocation's response is returned, since the execution environment may freeze
>    immediately after the response.

### `.specs/service/specs/06-configuration.md` → Sections → `[server]` (Modify)

> `host` (`0.0.0.0`), `port` (`8080`), `issuer` (the `iss` claim / discovery issuer, default
> empty), `role` (`all` | `exchange` | `admin`, default `all`), `base_path` (optional, default
> unset — a leading prefix such as `/prod` stripped from incoming request paths before routing;
> honored in both Lambda and server mode, though it exists chiefly for API Gateway stages and
> mount prefixes).

---

## Type changes

`ServerConfig` (`crates/core/src/config.rs:25-30`) gains `base_path: Option<String>` (default
`None`), surfaced as the optional `server.base_path` TOML key. No other type changes.

---

## Implementation notes

1. `crates/server/src/main.rs:29-33` — replace the log-and-return branch with
   `lambda_http::run(app).await` (axum 0.8's `Router` implements the tower `Service` contract
   `lambda_http` expects). Reconcile the error type (`lambda_http::Error`) with `main`'s
   `Box<dyn std::error::Error>`.
2. Add `lambda_http` to `crates/server/Cargo.toml` — it is not currently a dependency (grep
   confirms); default features cover the API Gateway and ALB event types.
3. Base path: add `base_path: Option<String>` to `ServerConfig`
   (`crates/core/src/config.rs:25-30`, default `None` in the `Default` impl at `:32-41`). Strip
   the prefix with a single tower layer applied in `bootstrap::build_router`
   (`crates/server/src/bootstrap.rs`), so Lambda and hyper modes share one code path.
4. Per-invocation flush: once the OTLP/X-Ray exporters land
   ([2026-06-24-complete_telemetry_exporters.md](2026-06-24-complete_telemetry_exporters.md)),
   force-flush the tracer provider synchronously in the Lambda path before each invocation's
   response is returned (`crates/server/src/telemetry.rs`); audit adapters that buffer flush
   the same way. No Lambda extension or `SIGTERM` handling is involved.
5. Integration check: `examples/aws-web/infra/lib/stack.ts:52-68` deploys the binary as the
   `provided.al2023` auth function (`handler: 'bootstrap'`); the example's `/token` and `/keys`
   routes working through API Gateway is the acceptance test for this change.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page (`04-http-api.md`,
   `06-configuration.md`); bump each page's `**Date:**`.
2. No change to `00-overview.md` or the bindings pages; no schema change (the sidecar does not
   model `ServerConfig`).
3. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- `lambda_http`'s current release supports axum 0.8 routers directly; no compatibility shim is
  needed.
- The deployment packages the binary as `bootstrap` for `provided.al2023` (as `examples/aws-web`
  already does).

### Decisions

- _Server binary only._ **The TS `@oidc-exchange/lambda` binding is untouched.** It is a
  different mechanism (event adapter over the Node binding) and already works; this change
  gives the Rust binary the runtime it claims.
- _One router, two runtimes._ **Lambda mode wraps the same `build_router` output.** No
  Lambda-specific routes or middleware forks.
- _Base path via config._ **An optional `server.base_path` key (default unset) strips the
  prefix from request paths before routing, honored in both Lambda and server mode.** One
  shared tower layer mirrors the TS binding's `basePath` without forking a Lambda-only option.
- _Synchronous flush per invocation._ **Telemetry and buffered audit writes flush synchronously
  before each invocation's response is returned.** This adds tail latency but guarantees
  delivery without a Lambda extension or `SIGTERM` machinery; revisit if latency becomes a
  problem.

### Open questions

- (None at this stage.)
