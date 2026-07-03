# Done Certificate — Task 05: exchange flow emission

**Task:** [05-exchange_flow_emission.md](05-exchange_flow_emission.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — `exchange.rs:104` (primary suspended branch) and `:242` (race
    re-lookup branch) emit `UserSuspended`/`Warning`/`Failure` with `Some(user.id)` actor, provider,
    and `request.ip_address`/`user_agent`; `:132`/`:148`/`:166`/`:186` each emit
    `RegistrationDenied`/`Warning`/`Failure` (actor `None`) with provider + ip/ua at the four denial
    branches; `:209` emits `UserCreated`/`Notice`/`Success` with `Some(created.id)`; `:317` emits
    `TokenExchange`/`Info`/`Success` with `Some(user.id)` after `TokenResponse` is assembled. All
    variants resolve to `crates/core/src/domain/audit.rs:40/45/47/50`; suspension uses `UserSuspended`
    (40:TokenExchange, 47:UserSuspended), not `Unauthorized`.

- **O2 — Gated by `emit_threshold` and blocking rules.**
  - *Claim:* emission runs through `emit_audit` (so Task 01's threshold applies) and a
    blocking-threshold failure propagates as `Err` from the flow.
  - *Evidence to collect:* trace one emission call — confirm it awaits `emit_audit` and propagates
    its `Result` (via `?` or explicit) at the audited point, not `let _ =`.
  - *Status:* ☑ SATISFIED — every emission is `self.emit_audit(create_audit_event(...)).await?`
    (`exchange.rs:114/143/159/177/197/218/256/326`), propagating the `Result` via `?`, never
    `let _ =`. `emit_audit` (`service/mod.rs:102`) applies Task 01's `emit_threshold` pre-dispatch
    and the `blocking_threshold` on adapter failure. Test
    `exchange_success_audit_failure_under_blocking_threshold_propagates_err` PASS confirms a blocking
    (`blocking_threshold: "info"`) audit failure on the `TokenExchange` emission propagates as
    `Err(AuditError)`.

- **O3 — Negative-space test.**
  - *Claim:* an allowlist rejection emits `RegistrationDenied` and no `TokenExchange`; a suspended
    user emits only `UserSuspended`.
  - *Evidence to collect:* run the new exchange emission tests — expect PASS asserting exactly the
    expected event types recorded on `MockAuditLog` for each branch.
  - *Status:* ☑ SATISFIED — `exchange_domain_allowlist_rejection_emits_registration_denied_and_no_token_exchange`
    PASS (asserts exactly one `RegistrationDenied`, `Failure` outcome, and no `TokenExchange`);
    `exchange_suspended_user_emits_only_user_suspended_event` PASS (asserts exactly one
    `UserSuspended`, `Failure` outcome, and nothing else).

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` exit 0; `cargo clippy --workspace --all-targets -- -D
    warnings` exit 0; `cargo nextest run --workspace` → 321 passed, 0 failed. No new magic-number
    limits introduced (only existing named config thresholds reused).

- **O5 — Reviewable: emission tests show each event with ip/ua.**
  - *Claim:* each named event is recorded with the request's ip/ua.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core exchange` — expect PASS;
    inspect that `MockAuditLog` events carry the ip/ua from the `ExchangeRequest`.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange-core exchange` → 28 passed, 0 failed.
    The new-user test asserts `UserCreated` and `TokenExchange` both carry `ip_address=203.0.113.9`
    / `user_agent=test-agent/2.0`; suspended test asserts `203.0.113.11` / `test-agent/4.0`;
    allowlist-rejection test asserts `203.0.113.12` / `test-agent/5.0` — each ip/ua pulled from the
    driving `ExchangeRequest`.

## Regression check

- `crates/server/src/routes/token.rs` `exchange` call → trace that a successful exchange still
  returns the same `TokenResponse` after emission is added → expect unchanged response : ☑ PRESERVED
  — `TokenResponse` is assembled unchanged (`exchange.rs:308-313`), the new `TokenExchange` emission
  runs after it, and `Ok(response)` is returned with identical fields; no error-path return values
  changed (each denial keeps its original `Error::AccessDenied`/`UserSuspended`).
- Existing exchange tests in `crates/core` (domain allowlist, registration mode) → expect their
  rejection returns unchanged : ☑ PRESERVED — all pre-existing exchange tests
  (`exchange_domain_allowlist_rejects_non_matching_domain`, `exchange_existing_users_only_rejects_new_user`,
  `exchange_suspended_user_is_rejected`, `exchange_no_email_rejected_when_allowlist_configured`, etc.)
  pass unchanged within the 28-test run.

## Residue

- Whether a failed `emit_audit` on the success path should still return the assembled token is
  governed by `emit_audit`'s blocking threshold (Task 01), not re-decided here.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence — every named branch emits its correct event/severity/
outcome carrying the request's ip/ua via `emit_audit(...).await?`, blocking-failure propagation is
test-proven, and fmt/clippy/321-test workspace runs are clean; both named regression callers
(token.rs response shape, existing rejection tests) are PRESERVED.
