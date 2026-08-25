# Done Certificate — Task 04: Put the refresh flow on the mandatory security-audit channel

**Task:** [04-refresh_on_mandatory_channel.md](04-refresh_on_mandatory_channel.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-25 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 04. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive the
> Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do not
> record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation names
(a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Refresh success, suspension (both the rotation and rotation-disabled gates), and reuse emit on the mandatory security channel (`emit_threshold`-immune, `audit.durability`-governed); the Debug `ValidationFailed` refusals stay best-effort.
- **P2 — Obligations.** Done iff O1…O4 all hold. One Oi per definition-of-done item, in DoD order; O4 is the Reviewable item.
- **P3 — Invariants.** Must preserve revoke-before-emit ordering for reuse (the family is dead before the emission can fail), keep the reuse event wire-compatible (`refresh_token_reuse`, warning, outcome `success`, detail `{family_id, sessions_revoked}`), and depends on task 01 (`refresh_rotation = false` functional) and task 03 (`client_addr` threaded into the request structs).

## Obligations

- **O1 — Security outcomes emit above a raised threshold.**
  - *Claim:* refresh success, suspension on the rotation path, suspension on the rotation-disabled path (`token.refresh_rotation = false`), and reuse are all emitted with `emit_threshold` raised above their severities (e.g. `error`).
  - *Evidence to collect:* run the new `crates/core/tests/refresh_mandatory_outcomes.rs` — expect all four events present with `emit_threshold = error`; confirm the test models `exchange_mandatory_outcomes.rs`.
  - *Checks:* resolve the emission calls in `revoke_family_for_reuse` (`refresh.rs:185`), both suspension gates (`:345-360`, `:453-468`), and `audit_successful_refresh` (`:500`) — confirm each is now `emit_security_event`/`emit_security_event_with_detail` (`service/mod.rs:280-318`), not `emit_audit`; confirm `SecurityEvent::RefreshTokenReuse` (`audit.rs:156`) maps `severity()` → `Warning`, `event_type()` → `AuditEventType::RefreshTokenReuse`, and that `AuthenticationSucceeded { kind: Refresh }` (`audit.rs:213-215`) is now constructed.
  - *Status:* ☐ unverified

- **O2 — Negative-space and durability behaviour.**
  - *Claim:* `ValidationFailed` refusals stay filtered by the default `emit_threshold`; with `audit.durability = "enforce"` and a failing sink, the reuse family is already revoked when the emission error propagates while success/suspension fail the request, and with `"observe"` degradation is recorded and the flow's outcome stands.
  - *Evidence to collect:* run the `ValidationFailed`-filtered test — expect the refusal dropped at default threshold; run the enforce-mode reuse test — trace `revoke_family` completing before the emission error, expect the family revoked and the request failing; run the observe-mode test — expect degradation recorded and the refusal standing.
  - *Checks:* resolve the retained refusal path `refuse_with_validation_failed` (`refresh.rs:151-178`) — confirm it still calls best-effort `emit_audit`, only swapping its `ClientAddr` argument (task 03).
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* the new event and emission swaps are tested with meaningful assertions and format/lint/test gates pass.
  - *Evidence to collect:* run `cargo fmt` (check), `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean, including the existing `refresh.rs`/`exchange.rs`/`revoke.rs` suites (per [development-guidelines.md](../../../development-guidelines.md) §Definition of done).
  - *Status:* ☐ unverified

- **O4 — Reviewable: outcomes survive a raised threshold, refusal dropped (Reviewable).**
  - *Claim:* a reviewer runs `refresh_mandatory_outcomes.rs` with a raised `emit_threshold` and sees the three security outcomes still emitted while the `ValidationFailed` refusal is dropped.
  - *Evidence to collect:* run the new test module and read the assertion that success/suspension/reuse are present while the `ValidationFailed` refusal is absent at the raised threshold.
  - *Status:* ☐ unverified

## Regression check

- The reuse flow's revoke-before-emit ordering: trace `revoke_family_for_reuse` with a live reused family and a failing enforce-mode sink → expect the family already revoked when the emission error returns (unchanged from the pre-fix ordering) : ☐ (PRESERVED / REGRESSION)
- Downstream consumers of the reuse audit event (audit sinks / `datamodel.schema.json` shape): the rendered event stays `refresh_token_reuse`/warning/`success`/`{family_id, sessions_revoked}` — trace one emitted event and confirm wire-shape equality : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: under committed defaults (`durability = "observe"`, `blocking_threshold = "warning"`) a reuse-alarm sink failure previously failed the request and now records degradation feeding `/health` — an intended, documented routing change per the change spec's Compatibility note.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
