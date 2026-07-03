# Task 05 — graceful shutdown on SIGTERM

**Plan:** [plan.md](../plan.md) · **Certificate:** [05-graceful_shutdown-certificate.md](05-graceful_shutdown-certificate.md)

**Implements:** [04-http-api.md](../../../service/specs/04-http-api.md) §Bootstrap (step 6 — serve over hyper with graceful shutdown; SIGTERM/ctrl-c stops accepting and drains in-flight requests for up to a 10 s hard deadline, then aborts stragglers and exits)
**Depends on:** —
**Produces:** the non-Lambda server drains in-flight requests on SIGTERM or ctrl-c and exits deterministically within a 10 s hard deadline instead of aborting connections.
**Pointers:** `crates/server/src/main.rs:33-38` (the `axum::serve(listener, app).await?` branch), `crates/server/Cargo.toml` (`tokio` is `features = ["full"]`, so `tokio::signal::unix` and `tokio::time::timeout` are available).

## Steps

- [x] Add a `shutdown_signal()` async helper that resolves on either SIGTERM (`tokio::signal::unix::signal(SignalKind::terminate())`) or ctrl-c (`tokio::signal::ctrl_c`), whichever fires first.
- [x] Wire `axum::serve(listener, app).with_graceful_shutdown(shutdown_signal())` so the server stops accepting connections and drains in-flight requests once a signal arrives.
- [x] Bound the post-signal drain: wrap the draining serve future in `tokio::time::timeout` using a named constant (e.g. `SHUTDOWN_DRAIN_DEADLINE_SECS: u64 = 10`); on expiry, log that stragglers are being aborted and exit rather than hang.
- [x] Add two meaningful assertions on the shutdown path (e.g. assert the deadline constant is non-zero; assert the bind address is non-empty before serving) and keep `main` free of business logic.
- [x] Add a unit test for the drain-deadline behaviour: a `shutdown_signal` future (or a stand-in) plus a never-completing drain wrapped in the `timeout` returns within the deadline (use a short injected duration in the test, the named constant in production).

## Definition of done

- [x] On SIGTERM or ctrl-c the server stops accepting connections and drains in-flight requests, then exits.
- [x] The drain is bounded by a 10 s named constant; if the drain does not finish, stragglers are aborted and the process exits deterministically (proven by a test using an injected short deadline against a non-completing drain).
- [x] Negative-space: the drain deadline fires and the process exits even when an in-flight request never completes (no indefinite hang).
- [x] The shutdown path carries at least two meaningful assertions and the deadline is a named constant.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer starts the server, holds a slow request open, sends SIGTERM, and observes the in-flight request drain and the process exit within the 10 s deadline (unit test covers the deadline; manual drive confirms the signal wiring).

## Open questions

- Whether the manual SIGTERM drive should be promoted to an integration test that spawns the binary and signals it, versus the unit test on the drain-deadline helper here. Recorded in plan.md Open questions.
