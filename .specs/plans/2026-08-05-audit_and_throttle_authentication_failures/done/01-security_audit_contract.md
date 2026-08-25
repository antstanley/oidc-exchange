# Task 01 — Security audit contract

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/service/specs/01-domain-model.md](../../../service/specs/01-domain-model.md) §Entities; [.specs/service/specs/02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §Port traits and §Adapter inventory; [.specs/service/specs/07-telemetry-and-audit.md](../../../service/specs/07-telemetry-and-audit.md) §Audit; [.specs/service/specs/canonical-types.schema.json](../../../service/specs/canonical-types.schema.json) AuditEvent definitions; [source spec](../../../changes/merged/2026-08-05-audit_and_throttle_authentication_failures.md) §Proposed changes and §Type changes
**Depends on:** —
**Produces:** Typed security outcomes, client-address provenance, and a rate-limit port that core and server code can use without raw stringly-typed audit or throttle state.
**Pointers:** `crates/core/src/domain/audit.rs`; `crates/core/src/domain/mod.rs`; `crates/core/src/ports/mod.rs`; `crates/core/src/ports/audit.rs`; `crates/core/src/error.rs`; `crates/test-utils/src/lib.rs:355`; `crates/adapters/src/noop/mod.rs`; `crates/core/src/service/mod.rs`

## Steps

- [x] Add `SecurityEvent`, `ClientAddr`, `ClientAddrSource`, and rate-limit key/decision domain types with exhaustive mappings to `AuditSeverity` and `AuditEventType`.
- [x] Extend `AuditEvent` construction and serialization with address provenance and add `ThrottleExceeded` without serializing raw subject identifiers or upstream error displays.
- [x] Define and export the `RateLimiter` port; add bounded mock/noop implementations and error/test seams needed by later core and server slices.
- [x] Refactor audit dispatch into explicit mandatory-security and threshold-filtered best-effort paths while preserving a typed error result for durability policy callers.
- [x] Add focused unit and mock tests for exhaustive security-event mapping, provenance rendering, subject hashing, no-op allow behavior, and event payload redaction.

## Definition of done

- [x] Every listed `SecurityEvent` maps exhaustively to the specified severity and audit event type, including `ThrottleExceeded` at warning.
- [x] `ClientAddr::Peer` and `Forwarded` are eligible rate keys while `Asserted` and `Unknown` are not, and serialized audit records retain their provenance.
- [x] The `RateLimiter` port, its key/decision contract, and mock/noop test implementations compile without placing I/O in `crates/core`.
- [x] Tests cover invalid/untrusted address and raw-subject/error-display negative space; all new bounds use named constants.
- [x] Meets the repo definition of done (targeted tests, Rust format/clippy/nextest, assertions, and synchronized schema/prose work as applicable — see plan.md baseline).
- [x] Reviewable: inspect typed security mappings and run focused core tests showing an untrusted client address cannot become a rate-limit key.
