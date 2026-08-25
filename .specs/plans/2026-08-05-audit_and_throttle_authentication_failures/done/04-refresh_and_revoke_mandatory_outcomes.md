# Task 04 — Refresh and revoke mandatory outcomes

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/service/specs/03-service-flows.md](../../../service/specs/03-service-flows.md) §Token refresh, §Revocation, §Audit emission and blocking, and §Admin operations; [.specs/service/specs/07-telemetry-and-audit.md](../../../service/specs/07-telemetry-and-audit.md) §Audit; [source spec](../../../changes/merged/2026-08-05-audit_and_throttle_authentication_failures.md) §Proposed changes and §Implementation notes 5 and 7
**Depends on:** 01, 02
**Produces:** Refresh and revocation flows with terminal mandatory security outcomes, subject limiting, and RFC 7009-safe enforce-mode behavior.
**Pointers:** `crates/core/src/service/refresh.rs:22`; `crates/core/src/service/revoke.rs:29`; `crates/core/src/service/user_admin.rs`; `crates/core/tests/refresh.rs`; `crates/core/tests/revoke.rs`

## Steps

- [x] Convert refresh to an inner body plus one terminal security-event mapping, including unknown/expired token, unknown/suspended principal, success, and limiter-denial paths.
- [x] Apply the per-subject limit only after a session resolves to a user and ensure rate-limiter errors are logged before proceeding under the documented fail-open policy.
- [x] Convert revoke to one terminal outcome record on both valid and rejected token paths while preserving 200 behavior for RFC 7009 token-state failures.
- [x] Ensure enforce-mode audit failure cannot turn revoke into a token-existence oracle; keep administrative mutation audit failure behavior aligned with mandatory durability semantics.
- [x] Replace existing suppression/no-event tests with focused exactly-one-event, reason-redaction, limiting, and equal-status tests.

## Definition of done

- [x] Refresh records exactly one mandatory event for each core-reached success and failure outcome, including unknown/expired tokens and suspended users.
- [x] Refresh consumes a subject budget only after subject resolution and limiter failures are explicitly logged while allowing the request to proceed.
- [x] Revoke emits exactly one event on both valid and rejected token paths and returns the same status for existing and nonexistent tokens when an enforcing sink fails.
- [x] Existing tests that assert missing audit records are replaced with positive and negative-space assertions for mandatory outcomes and safe fixed reasons.
- [x] Meets the repo definition of done (targeted tests, Rust format/clippy/nextest, assertions, and named-constant limits — see plan.md baseline).
- [x] Reviewable: run refresh and revoke tests demonstrating one record per outcome and RFC 7009 indistinguishability under enforce-mode sink failure.

## Open questions

- Coordinate final revoke behavior with sibling `2026-08-05-validate_revoke_token_claims`; do not add its claim-validation work here.
- Coordinate final refresh audit taxonomy with sibling `2026-08-05-rotate_refresh_tokens_with_reuse_detection`; do not add family rotation or reuse detection here.
