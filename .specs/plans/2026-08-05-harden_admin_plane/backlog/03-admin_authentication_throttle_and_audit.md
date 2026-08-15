# 03 · Admin authentication throttle and audit

**Status:** Blocked — external prerequisite  
**Implements:** [source spec](../../../changes/2026-08-05-harden_admin_plane.md) §"Implementation notes" step 2; [04-http-api](../../../service/specs/04-http-api.md) internal-auth target; [06-configuration](../../../service/specs/06-configuration.md) validation target; [07-telemetry-and-audit](../../../service/specs/07-telemetry-and-audit.md) target  
**Depends on:** — (external blocker: `2026-08-05-audit_and_throttle_authentication_failures` must merge)  
**Produces:** Internal-auth failures consume an `OperatorAuth` budget and emit required security/audit records without logging credentials; enabled shared secrets have a 32-byte floor.

**Pointers:** `crates/server/src/middleware/internal_auth.rs`; `crates/core/src/config.rs:74-90`; `crates/core/src/domain/audit.rs`; sibling-owned `RateLimiter`, `RateLimitKey`, `ClientAddr`, `SecurityEvent`, error mapping, and tests.

## Work

- After the sibling merges, add only `RateLimitKey::OperatorAuth(IpAddr)` and `OperatorAuthenticationFailed` extensions required by this spec; reuse its rate limiter, peer provenance, mandatory channel, and 429 mapping.
- Preserve `constant_time_eq`, including its length-handling path. On every missing, invalid, unknown, or unconfigured credential path, emit one warning/security event without recording presented credentials; consume only failed attempts.
- Consult the peer-address budget before credential evaluation; on denial return sibling-defined 429/`Retry-After` and emit `ThrottleExceeded`.
- Require a 32-byte minimum shared secret whenever that mechanism serves the internal API, with 31/32-byte startup tests and deployment migration documentation.

## Definition of done

- [ ] No code duplicates the sibling limiter, provenance, event channel, or 429 mapping; extensions compile against its merged contracts.
- [ ] Tests cover valid credential (no budget consumption), missing/invalid/unconfigured credentials (one failed attempt and one event each), lockout before credential evaluation, and no credential in logs/events.
- [ ] Tests verify peer `ConnectInfo`/`ClientAddr::Peer` is used rather than forwarded headers and retain constant-time comparison behaviour.
- [ ] Validation rejects a 31-byte enabled shared secret and accepts 32 bytes; all bounds/configuration are named and documented.
- [ ] Rust format/clippy/affected nextest tests pass; unrelated failures are recorded but not fixed.
- [ ] Reviewable: failed admin authentication is visible and bounded through the common security primitives.
