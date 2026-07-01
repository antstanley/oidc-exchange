# Done Certificate — Task 06: refresh flow emission

**Task:** [06-refresh_flow_emission.md](06-refresh_flow_emission.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 06. A validating agent discharges it: collect
> each obligation's evidence, run its checks, set the Status, then derive the Conclusion by the rubric.
> Do not mark an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 06) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not by
assertion.

## Premises

- **P1 — Goal.** The refresh flow emits `ValidationFailed` (debug), `UserSuspended`, and
  `TokenRefresh` at their named points, recording the request's ip/ua.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing refresh returns (`InvalidToken` on unknown/expired
  token/user, `UserSuspended` error, success `TokenResponse`) in `crates/core/src/service/refresh.rs`.

## Obligations

- **O1 — Each named point emits its named event.**
  - *Claim:* unknown/expired token and unknown user → `ValidationFailed` (debug, failure); suspended
    user → `UserSuspended`; success → `TokenRefresh` (info, success) — each with `request` ip/ua.
  - *Evidence to collect:* read `refresh.rs:22`, `:28`, `:38`, `:43`, `:50`; confirm each emits the
    correct event via `create_audit_event` + `emit_audit`.
  - *Checks:* resolve `ValidationFailed`/`UserSuspended`/`TokenRefresh` to
    `crates/core/src/domain/audit.rs`; confirm the debug severity on `ValidationFailed`.
  - *Status:* ☐ unverified

- **O2 — `ValidationFailed` gated at `debug` by `emit_threshold`.**
  - *Claim:* `ValidationFailed` is emitted at `debug` and suppressed under the default `info`
    threshold, surfacing only when the threshold is lowered to `debug`.
  - *Evidence to collect:* run the threshold-behaviour tests — expect PASS: default `info` → no
    event recorded on `MockAuditLog` for an unknown-token refresh; `debug` threshold → `ValidationFailed` recorded.
  - *Checks:* confirm suppression is via `emit_audit`'s Task-01 filter, not a per-call-site `if`.
  - *Status:* ☐ unverified

- **O3 — Negative-space test.**
  - *Claim:* an unknown/expired refresh records nothing under the default threshold; the same
    records `ValidationFailed` under a `debug` threshold.
  - *Evidence to collect:* run the new refresh emission tests — expect PASS for both threshold cases.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: threshold-gated behaviour and success event observed.**
  - *Claim:* `ValidationFailed` is threshold-gated and `TokenRefresh` fires on success.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core refresh` — expect PASS;
    inspect the `MockAuditLog` events for each case.
  - *Status:* ☐ unverified

## Regression check

- The refresh happy path returns a `TokenResponse` with no new refresh token → trace that emission
  does not alter the response → expect unchanged : ☐ (PRESERVED / REGRESSION)
- The `InvalidToken` returns at `:22`/`:28`/`:38` → trace that emitting before returning preserves
  the same error → expect unchanged : ☐ (PRESERVED / REGRESSION)

## Residue

- The abuse-detection use of `ValidationFailed` (lowering the threshold in production) is an
  operator choice; the task only makes it available. Note only.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
