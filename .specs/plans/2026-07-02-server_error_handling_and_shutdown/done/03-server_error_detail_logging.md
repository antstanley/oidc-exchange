# Task 03 — log the `server_error` internal detail

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-server_error_detail_logging-certificate.md](03-server_error_detail_logging-certificate.md)

**Implements:** [04-http-api.md](../../../service/specs/04-http-api.md) §Error mapping (`server_error` responses log the internal detail via `tracing::error!`, inside the request span, and return a generic message)
**Depends on:** 01 (review — the detail log is reviewed carrying the request id once the span exists)
**Produces:** 500/502/504 responses log their internal source error via `tracing::error!` under the request span, while the client still receives only the generic body.
**Pointers:** `crates/server/src/error.rs:88-106` (`map_domain_error`, the `ProviderError` / `ProviderTimeout` / `StoreError`-class arms that currently drop the detail).

## Steps

- [x] In `map_domain_error` (or `ApiError::into_response` immediately before it returns), emit `tracing::error!` with the underlying error for the `server_error` arms: `Error::ProviderError`, `Error::ProviderTimeout`, and the `StoreError | KeyError | AuditError | SyncError | ConfigError` group.
- [x] Log the structured error (e.g. `error = %err`) so it is captured inside the request span from task 01 and carries the `request_id`; keep the returned client body generic and unchanged (no detail leak).
- [x] Do not log for the client-fault arms (`InvalidGrant`, `InvalidToken`, `InvalidRequest`, `UnknownProvider`, `AccessDenied`, `UserSuspended`, `Unauthorized`) — only the `server_error` class.
- [x] Add two meaningful assertions to the touched function (e.g. assert the mapped status is a 5xx before logging in the server_error arms; assert the returned `error_code` is `"server_error"` for those arms).
- [x] Add a test asserting a `ProviderError`/`StoreError` maps to its 5xx status with the generic body **and** produces a captured `tracing::error!` event carrying the internal detail (tracing test subscriber), while an `InvalidGrant` produces no such error log.

## Definition of done

- [x] A `ProviderError` (502), `ProviderTimeout` (504), and `StoreError`-class (500) each emit a `tracing::error!` carrying the internal detail, captured under the request span.
- [x] Negative-space: a client-fault error (e.g. `InvalidGrant`) returns its 4xx and emits **no** `server_error` detail log; the 5xx client bodies remain generic with no infrastructure detail.
- [x] The touched function carries at least two meaningful assertions.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the error-mapping tests and sees a captured `error`-level log with the internal detail for a 5xx, none for a 4xx, and confirms the client body stays generic.
