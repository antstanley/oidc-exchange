# Done Certificate — Task 06: refresh flow emission

**Task:** [06-refresh_flow_emission.md](06-refresh_flow_emission.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — `refresh.rs:35-47` (unknown token), `:54-66` (expired token, actor
    `session.user_id`), `:74-86` (unknown user, actor `session.user_id`) each emit
    `AuditEventType::ValidationFailed` at `AuditSeverity::Debug` with `AuditOutcome::Failure` and
    `request.ip_address`/`request.user_agent`; `:91-102` emits `UserSuspended` (Warning, actor
    `user.id`); `:110-119` emits `TokenRefresh` (Info, `AuditOutcome::Success`, actor `user.id`).
    Types resolve to `domain/audit.rs`: `AuditEventType::{TokenRefresh,UserSuspended,ValidationFailed}`
    (lines 41/47/49) and `AuditSeverity::Debug` (=7, line 34). Each is built via `create_audit_event`
    (mod.rs:146, arg order actor/provider/ip/ua confirmed) + `emit_audit` (mod.rs:102).

- **O2 — `ValidationFailed` gated at `debug` by `emit_threshold`.**
  - *Claim:* `ValidationFailed` is emitted at `debug` and suppressed under the default `info`
    threshold, surfacing only when the threshold is lowered to `debug`.
  - *Evidence to collect:* run the threshold-behaviour tests — expect PASS: default `info` → no
    event recorded on `MockAuditLog` for an unknown-token refresh; `debug` threshold → `ValidationFailed` recorded.
  - *Checks:* confirm suppression is via `emit_audit`'s Task-01 filter, not a per-call-site `if`.
  - *Status:* ☑ SATISFIED — `refresh_unknown_token_under_default_threshold_emits_nothing` PASS
    (default `info` → `MockAuditLog` empty); `refresh_unknown_token_under_debug_threshold_emits_validation_failed`
    PASS (debug → exactly one `ValidationFailed`, Failure, ip `203.0.113.21`/ua `test-agent/2.0`).
    Suppression is the emit-threshold filter in `emit_audit` (mod.rs:106-109: drops when
    `event.severity as u8 > emit_threshold as u8`; Debug=7 > Info=6 → dropped), not a call-site `if`
    — the refresh sites emit unconditionally.

- **O3 — Negative-space test.**
  - *Claim:* an unknown/expired refresh records nothing under the default threshold; the same
    records `ValidationFailed` under a `debug` threshold.
  - *Evidence to collect:* run the new refresh emission tests — expect PASS for both threshold cases.
  - *Status:* ☑ SATISFIED — the default-threshold negative case
    (`refresh_unknown_token_under_default_threshold_emits_nothing`) and the debug-threshold positive
    cases (`refresh_unknown_token_under_debug_threshold_emits_validation_failed`,
    `refresh_expired_token_under_debug_threshold_emits_validation_failed_with_actor` — actor =
    session user id) all PASS.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` clean, `cargo clippy --workspace --all-targets
    -- -D warnings` clean, `cargo nextest run --workspace` → 326 passed / 0 failed. Limits use
    named constants (no new magic numbers introduced by this task).

- **O5 — Reviewable: threshold-gated behaviour and success event observed.**
  - *Claim:* `ValidationFailed` is threshold-gated and `TokenRefresh` fires on success.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core refresh` — expect PASS;
    inspect the `MockAuditLog` events for each case.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange-core refresh` → 12 passed / 0 failed.
    `refresh_success_emits_token_refresh_event` observes exactly one `TokenRefresh` (Success, actor
    user id, ip `203.0.113.23`/ua `test-agent/4.0`) under the default `info` threshold, and the
    unknown-token default vs debug pair shows the threshold gate on `ValidationFailed`.

## Regression check

- The refresh happy path returns a `TokenResponse` with no new refresh token → trace that emission
  does not alter the response → expect unchanged : ☑ PRESERVED — `refresh.rs:122-127` still returns
  `refresh_token: None`; the success emit at `:110-119` precedes and does not touch the response.
  `refresh_happy_path_returns_new_access_token` PASS.
- The `InvalidToken` returns at `:22`/`:28`/`:38` → trace that emitting before returning preserves
  the same error → expect unchanged : ☑ PRESERVED — each site emits then `return Err(Error::InvalidToken
  { reason })` with the same reason string (`:47`, `:66`, `:86`); `refresh_unknown_token_returns_invalid_token`
  and `refresh_expired_token_returns_invalid_token` PASS. Suspended path still returns
  `Error::UserSuspended { user_id }` (`:103`).

## Residue

- The abuse-detection use of `ValidationFailed` (lowering the threshold in production) is an
  operator choice; the task only makes it available. Note only.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence — each named refresh point emits its event
(`ValidationFailed`/`UserSuspended`/`TokenRefresh`) with correct severity/outcome and request ip/ua,
`ValidationFailed` is threshold-gated by `emit_audit`'s filter (suppressed at default `info`, surfaced
at `debug`), full suite is green (326 passed, clippy/fmt clean), and both regression traces (happy-path
response, `InvalidToken`/`UserSuspended` returns) are PRESERVED.
