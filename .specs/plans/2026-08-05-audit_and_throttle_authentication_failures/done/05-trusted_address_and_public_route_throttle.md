# Task 05 — Trusted address and public-route throttle

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/service/specs/04-http-api.md](../../../service/specs/04-http-api.md) §Middleware stack, §Error mapping, §Bootstrap, and §Assumptions; [.specs/service/specs/01-domain-model.md](../../../service/specs/01-domain-model.md) §Entities; [source spec](../../../changes/merged/2026-08-05-audit_and_throttle_authentication_failures.md) §Proposed changes and §Implementation notes 6–8
**Depends on:** 01, 02
**Produces:** Public routes that derive audit provenance and rate keys from peer/trusted-forwarded addresses, throttle safely before handlers, return OAuth 429 responses, and write request-scoped access logs.
**Pointers:** `crates/server/src/main.rs:67`; `crates/server/src/bootstrap.rs:302`; `crates/server/src/routes/mod.rs:13`; `crates/server/src/middleware/audit_context.rs:21`; `crates/server/src/middleware/mod.rs`; `crates/server/src/error.rs:80`; `crates/server/src/lambda.rs`; `crates/server/Cargo.toml`

## Steps

- [x] Add client-address middleware that obtains peer address through `into_make_service_with_connect_info::<SocketAddr>()`, chooses configured trusted proxy hops from the right, and records asserted/unknown provenance when no trusted source applies.
- [x] Adapt Lambda request-context source IP and FFI/no-peer behavior to the same `ClientAddr` contract; bound copied `User-Agent` and device headers with named constants.
- [x] Add the fixed-window throttle middleware only to public routes, consume per-IP failure budgets from authentication responses, and place load shedding/concurrency bounds as specified.
- [x] Add `TooManyRequests` domain/error mapping to 429 `slow_down` with `Retry-After`, ensuring middleware denial occurs before a provider call.
- [x] Add public-route access logging inside the request span and focused middleware/router tests for peer, trusted forwarded, asserted, unknown, limits, and header behavior.

## Definition of done

- [x] Per-IP limiter keys derive only from server-observed peer or validated trusted-forwarded addresses; asserted and unknown values never become keys.
- [x] Trusted proxy hop selection is right-to-left, bounded and validated; Lambda uses platform context before headers and FFI records unknown provenance.
- [x] Public throttle denial returns 429 `slow_down` with `Retry-After`, is confined to public routes, and is applied before provider work.
- [x] Access logs inherit `request_id`, and copied headers/address chains respect named length/count bounds with tests for below/at/above boundaries.
- [x] Meets the repo definition of done (targeted tests, Rust format/clippy/nextest, assertions, and named-constant limits — see plan.md baseline).
- [x] Reviewable: exercise the real public router with a forged forwarding header and show one observed-address budget, correct provenance, and a 429 before handler/provider work.
