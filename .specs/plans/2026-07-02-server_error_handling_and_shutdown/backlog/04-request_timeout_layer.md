# Task 04 — request-timeout middleware layer

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-request_timeout_layer-certificate.md](04-request_timeout_layer-certificate.md)

**Implements:** [04-http-api.md](../../../service/specs/04-http-api.md) §Middleware stack (new entry 2, Request timeout) and [06-configuration.md](../../../service/specs/06-configuration.md) §Sections → `[server]` and §Defaults summary (`request_timeout` = `"30s"`)
**Depends on:** —
**Produces:** a request that runs longer than `server.request_timeout` (default 30 s) is aborted with a 408 response, and the bound is configuration, not a constant.
**Pointers:** `crates/core/src/config.rs:23-41` (`ServerConfig` + its `Default`), `crates/core/src/service/mod.rs:168` (`parse_duration_secs`, the humantime-style parser to reuse), `crates/server/src/bootstrap.rs:134-136` (the middleware stack), `crates/server/Cargo.toml:18` (`tower-http` features).

## Steps

- [ ] Add a `request_timeout: String` field to `ServerConfig` and set its `Default` to a named constant (e.g. `DEFAULT_REQUEST_TIMEOUT: &str = "30s"`), not a bare literal; extend the config deserialization tests to assert the default and a parsed override.
- [ ] Parse `request_timeout` into a `Duration` reusing the `[token]`-TTL humantime parsing (make `parse_duration_secs` reachable from the server, or parse equivalently), and reject an unparseable value at startup rather than silently defaulting.
- [ ] Enable the `timeout` feature on `tower-http` in `crates/server/Cargo.toml`.
- [ ] Insert `tower_http::timeout::TimeoutLayer::new(request_timeout)` into the stack in `bootstrap.rs` as **entry 2 of the outermost-first ordering** — inside the request-id layer (so a timeout response still carries the request id) and outside audit-context and catch-panic (so the bound covers the remaining middleware and the handler).
- [ ] Confirm `TimeoutLayer` responds `408 Request Timeout` on expiry against the pinned `tower-http` 0.6; add two meaningful assertions where the duration is built (e.g. assert the parsed duration is non-zero and within a sane upper bound).
- [ ] Add a server test: a handler that sleeps past a short configured `request_timeout` yields 408 (and, given task 01, the response carries the request id); a fast handler under the bound yields 200.

## Definition of done

- [ ] A request exceeding `server.request_timeout` is aborted with 408; a request under it completes normally.
- [ ] `request_timeout` is a `[server]` config key defaulting to `"30s"` via a named constant, parsed as a humantime duration; the config tests cover the default and an override.
- [ ] Negative-space: an unparseable `request_timeout` fails fast at startup (config error), not a silent fallback; the timeout layer sits inside the request-id layer (verified by the request id on the 408).
- [ ] The duration-building code carries at least two meaningful assertions and the default bound is a named constant.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the timeout test and sees a slow request return 408 and a fast one 200, and confirms `request_timeout` reads from config with a 30 s default.
