# 03 · Admin authentication throttle and audit

**Status:** Done — reconciled against the merged audit/throttle primitives  
**Implements:** [source spec](../../../changes/merged/2026-08-05-harden_admin_plane.md) §"Implementation notes" step 2; [04-http-api](../../../service/specs/04-http-api.md) internal-auth target; [06-configuration](../../../service/specs/06-configuration.md) validation target; [07-telemetry-and-audit](../../../service/specs/07-telemetry-and-audit.md) target  
**Depends on:** `2026-08-05-audit_and_throttle_authentication_failures` (merged)  
**Produces:** Internal-auth failures consume an `OperatorAuth` budget and emit required security/audit records without logging credentials; enabled shared secrets have a 32-byte floor.

**Pointers:** `crates/server/src/middleware/internal_auth.rs`; `crates/core/src/config.rs:74-90`; `crates/core/src/domain/audit.rs`; sibling-owned `RateLimiter`, `RateLimitKey`, `ClientAddr`, `SecurityEvent`, error mapping, and tests.

## Resolution (at merge)

The sibling (`audit_and_throttle_authentication_failures`) merged first, so this
task completed as the reconciliation pass of this branch's merge:

- The vendored seam was deleted wholesale: `domain::operator`'s copies of
  `ClientAddr`, `RateLimitKey`, `RateLimitDecision`, and `SecurityEvent`, the
  vendored `RateLimiter` port, the string `security_failure_reasons` constants,
  and the vendored `emit_security_event` all yielded to the sibling's canonical
  types; call sites were re-pointed.
- The canonical types gained exactly this spec's extensions:
  `RateLimitKey::OperatorAuth(IpAddr)`,
  `SecurityEvent::OperatorAuthenticationFailed { reason }` (rendering as
  `AuditEventType::Unauthorized` at warning severity), and typed
  `AuditFailure::{MissingCredential, InvalidCredential, NotConfigured}`
  replacing the string reasons.
- The consult-without-consume contract survives as `RateLimiter::check` /
  `RateLimiter::consume` alongside the sibling's `check_and_consume`; a
  `CompositeRateLimiter` in bootstrap routes `OperatorAuth` keys to
  `AdminAuthRateLimiter` and every exchange-plane key to the fixed-window
  limiter, so neither plane can exhaust the other's budget.
- Mandatory-channel emission goes through the sibling's
  `emit_security_event_with_detail` (route detail, typed failure reason);
  sink-failure handling follows the sibling's `audit.durability` policy.
- The 32-byte shared-secret floor (`MIN_SHARED_SECRET_BYTES`) is enforced in
  `Config::resolve` whenever the mechanism serves the internal API, with
  31/32-byte boundary tests and the migration documented in
  `docs/guides/configuration.md`.

## Work

- After the sibling merges, add only `RateLimitKey::OperatorAuth(IpAddr)` and `OperatorAuthenticationFailed` extensions required by this spec; reuse its rate limiter, peer provenance, mandatory channel, and 429 mapping.
- Preserve `constant_time_eq`, including its length-handling path. On every missing, invalid, unknown, or unconfigured credential path, emit one warning/security event without recording presented credentials; consume only failed attempts.
- Consult the peer-address budget before credential evaluation; on denial return sibling-defined 429/`Retry-After` and emit `ThrottleExceeded`.
- Require a 32-byte minimum shared secret whenever that mechanism serves the internal API, with 31/32-byte startup tests and deployment migration documentation.

## Definition of done

- [x] No code duplicates the sibling limiter, provenance, event channel, or 429 mapping; extensions compile against its merged contracts.
- [x] Tests cover valid credential (no budget consumption), missing/invalid/unconfigured credentials (one failed attempt and one event each), lockout before credential evaluation, and no credential in logs/events.
- [x] Tests verify peer `ConnectInfo`/`ClientAddr::Peer` is used rather than forwarded headers and retain constant-time comparison behaviour.
- [x] Validation rejects a 31-byte enabled shared secret and accepts 32 bytes; all bounds/configuration are named and documented.
- [x] Rust format/clippy/affected nextest tests pass; unrelated failures are recorded but not fixed.
- [x] Reviewable: failed admin authentication is visible and bounded through the common security primitives.
