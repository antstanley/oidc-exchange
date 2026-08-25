# Task 04 — Put the refresh flow on the mandatory security-audit channel

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-refresh_on_mandatory_channel-certificate.md](04-refresh_on_mandatory_channel-certificate.md)

**Implements:** [00-overview.md](../../../service/specs/00-overview.md) §Goals (mandatory channel), [03-service-flows.md](../../../service/specs/03-service-flows.md) §Token refresh / §Audit emission, [07-telemetry-and-audit.md](../../../service/specs/07-telemetry-and-audit.md) §Audit; change spec §The delta → S3
**Depends on:** 01, 03
**Produces:** Refresh success, suspension (both the rotation and rotation-disabled gates), and reuse emit on the mandatory security channel (`emit_threshold`-immune, `audit.durability`-governed); the Debug `ValidationFailed` refusals stay best-effort.
**Pointers:** `crates/core/src/domain/audit.rs:156-238` (`SecurityEvent`, `severity`/`event_type`), `:353` (`AuditFailure::RefreshTokenReuse`); `crates/core/src/service/refresh.rs:185-242` (reuse), `:345-360` (suspension), `:453-468` (rotation-disabled suspension in `refresh_without_rotation`), `:500-533` (`audit_successful_refresh`), `:151-178` (retained `ValidationFailed`); `crates/core/src/service/mod.rs:280-318` (`emit_security_event*`)

## Steps

- [ ] Add `SecurityEvent::RefreshTokenReuse` (`audit.rs:156`): `severity()` → `Warning`, `event_type()` → `AuditEventType::RefreshTokenReuse`; keep the rendered event byte-compatible (`refresh_token_reuse`, warning, outcome `success`, detail `{family_id, sessions_revoked}`).
- [ ] `revoke_family_for_reuse` (`refresh.rs:185`): swap `create_audit_event` + `emit_audit` for `emit_security_event_with_detail(SecurityEvent::RefreshTokenReuse, …)`, preserving revoke-before-emit ordering.
- [ ] Both suspension gates (`refresh.rs:345-360` and `:453-468`): `emit_security_event(SecurityEvent::PrincipalSuspended, AuditOutcome::Failure(AuditFailure::PrincipalSuspended), …)`.
- [ ] `audit_successful_refresh` (`refresh.rs:500`): `emit_security_event_with_detail(SecurityEvent::AuthenticationSucceeded { kind: Refresh }, …)`, constructing the long-mapped `audit.rs:213-215` arm; each call takes `request.client_addr` (task 03).
- [ ] Leave the shared `ValidationFailed` refusal path (`refresh.rs:151-178`) on best-effort `emit_audit` (spec'd at Debug below `emit_threshold`).

## Definition of done

- [ ] New `crates/core/tests/refresh_mandatory_outcomes.rs` (modeled on `exchange_mandatory_outcomes.rs`): refresh success, suspension on the rotation path, suspension on the rotation-disabled path (`token.refresh_rotation = false`), and reuse are all emitted with `emit_threshold` raised above their severities (e.g. `error`).
- [ ] Negative-space + durability: `ValidationFailed` refusals stay filtered by the default `emit_threshold`; with `audit.durability = "enforce"` and a failing sink, the reuse family is already revoked when the emission error propagates while success/suspension fail the request, and with `"observe"` degradation is recorded and the flow's outcome stands.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [ ] Reviewable: a reviewer runs `refresh_mandatory_outcomes.rs` with a raised `emit_threshold` and sees the three security outcomes still emitted while the `ValidationFailed` refusal is dropped.
