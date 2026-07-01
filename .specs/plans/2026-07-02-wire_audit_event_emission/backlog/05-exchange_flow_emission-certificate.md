# Done Certificate — Task 05: exchange flow emission

**Task:** [05-exchange_flow_emission.md](05-exchange_flow_emission.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 05. A validating agent discharges it: collect
> each obligation's evidence, run its checks, set the Status, then derive the Conclusion by the rubric.
> Do not mark an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 05) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not by
assertion.

## Premises

- **P1 — Goal.** The exchange flow emits `UserSuspended`, `RegistrationDenied`, `UserCreated`, and
  `TokenExchange` at their named points, each recording the request's ip/ua.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing exchange happy path (token response shape,
  session store) or the registration-policy rejection returns in
  `crates/core/src/service/exchange.rs:92-168`.

## Obligations

- **O1 — Each named point emits its named event.**
  - *Claim:* suspension → `UserSuspended` (warning, failure); each denial → `RegistrationDenied`
    (warning, failure); new user → `UserCreated` (notice, success); success → `TokenExchange` (info,
    success) after the response is assembled — each carrying `request.ip_address`/`user_agent`.
  - *Evidence to collect:* read `exchange.rs:92`, `:105`/`:110`/`:116`/`:126`, `:137`, `:168`;
    confirm each emits the correct event via `create_audit_event` + `emit_audit` with ip/ua.
  - *Checks:* resolve each `AuditEventType` variant to `crates/core/src/domain/audit.rs`; confirm
    the suspension path uses `UserSuspended`, not `Unauthorized`.
  - *Status:* ☐ unverified

- **O2 — Gated by `emit_threshold` and blocking rules.**
  - *Claim:* emission runs through `emit_audit` (so Task 01's threshold applies) and a
    blocking-threshold failure propagates as `Err` from the flow.
  - *Evidence to collect:* trace one emission call — confirm it awaits `emit_audit` and propagates
    its `Result` (via `?` or explicit) at the audited point, not `let _ =`.
  - *Status:* ☐ unverified

- **O3 — Negative-space test.**
  - *Claim:* an allowlist rejection emits `RegistrationDenied` and no `TokenExchange`; a suspended
    user emits only `UserSuspended`.
  - *Evidence to collect:* run the new exchange emission tests — expect PASS asserting exactly the
    expected event types recorded on `MockAuditLog` for each branch.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: emission tests show each event with ip/ua.**
  - *Claim:* each named event is recorded with the request's ip/ua.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core exchange` — expect PASS;
    inspect that `MockAuditLog` events carry the ip/ua from the `ExchangeRequest`.
  - *Status:* ☐ unverified

## Regression check

- `crates/server/src/routes/token.rs` `exchange` call → trace that a successful exchange still
  returns the same `TokenResponse` after emission is added → expect unchanged response : ☐ (PRESERVED / REGRESSION)
- Existing exchange tests in `crates/core` (domain allowlist, registration mode) → expect their
  rejection returns unchanged : ☐ (PRESERVED / REGRESSION)

## Residue

- Whether a failed `emit_audit` on the success path should still return the assembled token is
  governed by `emit_audit`'s blocking threshold (Task 01), not re-decided here.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
