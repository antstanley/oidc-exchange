# Task 06 — Public audit and throttle end to end

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/service/specs/03-service-flows.md](../../../service/specs/03-service-flows.md) §Token exchange, §Token refresh, §Revocation, and §Audit emission and blocking; [.specs/service/specs/04-http-api.md](../../../service/specs/04-http-api.md) §Middleware stack and §Error mapping; source spec §Implementation notes tests
**Depends on:** 03, 04, 05
**Produces:** Full-router evidence that public authentication failures are recorded once, bounded by trusted keys, and denied without excess upstream work while documented baseline failures remain isolated.
**Pointers:** `crates/server/tests/e2e.rs`; `crates/server/tests/routes.rs`; `crates/server/tests/base_path.rs`; `crates/core/tests/exchange.rs`; `crates/core/tests/refresh.rs`; `crates/core/tests/revoke.rs`; `crates/test-utils/src/lib.rs:355`

## Steps

- [ ] Build deterministic server E2E fixtures for configurable audit failure, limiter decisions/state, provider call counts, peer `ConnectInfo`, trusted proxy configuration, and clock/window boundaries.
- [ ] Add outcome-space tests that verify exactly one security event for public core-reached authentication failure classes and threshold immunity under `emit_threshold = "emergency"`.
- [ ] Add enforce-mode E2E tests showing failed `/token` leaves no session and failed `/revoke` has identical statuses for existing and nonexistent token cases.
- [ ] Add router tests for forged and trusted forwarding chains, missing headers, the 61st-request-style boundary, `Retry-After`, provider non-invocation after denial, and access-log request correlation.
- [ ] Run scoped Rust checks and record the known `cargo test --workspace` three-test configuration failure as baseline-only, without changing or masking it.

## Definition of done

- [ ] End-to-end tests prove one mandatory security event for each public core-reached authentication result and prove security events bypass `emit_threshold`.
- [ ] Enforce-mode `/token` failure leaves no session, and enforce-mode `/revoke` remains status-indistinguishable for existing versus nonexistent tokens.
- [ ] Trusted/forged/missing-forwarded-header tests prove correct provenance and peer-budget behavior; limiter boundary denial includes `Retry-After` and has no provider call afterward.
- [ ] Test fixtures are deterministic and test positive, negative, and one-below/at/one-above limit boundaries.
- [ ] Meets the repo definition of done (targeted tests, Rust format/clippy/nextest, assertions, and named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the public-route E2E suite and inspect its proof of mandatory audit, trusted-address throttling, 429 behavior, and unchanged unrelated workspace-test baseline.
