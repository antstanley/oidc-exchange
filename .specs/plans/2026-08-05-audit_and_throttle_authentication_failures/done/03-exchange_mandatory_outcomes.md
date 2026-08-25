# Task 03 — Exchange mandatory outcomes

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/service/specs/03-service-flows.md](../../../service/specs/03-service-flows.md) §Token exchange and §Audit emission and blocking; [.specs/service/specs/07-telemetry-and-audit.md](../../../service/specs/07-telemetry-and-audit.md) §Audit; [source spec](../../../changes/merged/2026-08-05-audit_and_throttle_authentication_failures.md) §Proposed changes and §Implementation notes 5 and 7
**Depends on:** 01, 02
**Produces:** A token-exchange flow with one terminal, mandatory security event per result, fixed safe failure reasons, provider/subject limiting, and session cleanup on enforce-mode audit failure.
**Pointers:** `crates/core/src/service/exchange.rs:29`; `crates/core/src/service/mod.rs:102`; `crates/core/src/error.rs`; `crates/core/tests/exchange.rs`; `crates/test-utils/src/lib.rs:355`

## Steps

- [x] Split exchange into a fallible inner body and one outer terminal-outcome mapper that emits exactly one `SecurityEvent` for every core-reached result.
- [x] Map exchange error classes to fixed classification strings rather than `Display`, preserve creation-site principal events, and keep concurrent losing registration paths free of duplicate creation events.
- [x] Consume per-provider budget before outbound code/JWKS work and per-subject budget after validated claims; map denials to `TooManyRequests` and mandatory `ThrottleExceeded` records.
- [x] On enforce-mode terminal success audit failure, revoke the newly stored session before returning the audit error; retain observe-mode degraded logging behavior.
- [x] Add property-style outcome-space tests for exactly one event, threshold immunity, safe classifications, deny-before-provider behavior, and no surviving session after enforce failure.

## Definition of done

- [x] Every core-reached exchange success and failure class emits exactly one mandatory security event with the source-spec classification and no upstream response body.
- [x] `emit_threshold = "emergency"` cannot suppress exchange security events.
- [x] Provider and subject denials return `TooManyRequests`, audit `ThrottleExceeded`, and prevent the denied downstream provider work.
- [x] A failing enforcing audit sink causes token exchange failure and leaves no stored refresh session; observe mode remains explicitly observable rather than silently swallowing the error.
- [x] Meets the repo definition of done (targeted tests, Rust format/clippy/nextest, assertions, and named-constant limits — see plan.md baseline).
- [x] Reviewable: run exchange outcome tests showing one event per outcome and no live session after an enforce-mode sink failure.
