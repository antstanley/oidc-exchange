# Done Certificate — Task 07: revoke flow emission

**Task:** [07-revoke_flow_emission.md](07-revoke_flow_emission.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 07. A validating agent discharges it: collect
> each obligation's evidence, run its checks, set the Status, then derive the Conclusion by the rubric.
> Do not mark an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 07) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not by
assertion.

## Premises

- **P1 — Goal.** The access-token path emits `AllSessionsRevoked` on verified revocation; the
  refresh-token path emits `TokenRevocation` on an actual session removal; failures emit nothing.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the RFC 7009 always-`Ok(())` contract of `revoke`
  (`crates/core/src/service/revoke.rs:14-38`) or the silent-on-failure behaviour.

## Obligations

- **O1 — Emit on verified revocation.**
  - *Claim:* the access-token path emits `AllSessionsRevoked` only when `verify_and_extract_sub`
    returns a user id and sessions are revoked; the refresh-token path emits `TokenRevocation` only
    when a session was actually removed — both with `request` ip/ua.
  - *Evidence to collect:* read `revoke.rs:19-21` (access path) and `:28`/`:34` (refresh path);
    confirm emission is conditional on the revocation actually happening.
  - *Checks:* resolve `AllSessionsRevoked`/`TokenRevocation` to `crates/core/src/domain/audit.rs`;
    trace that the emission is inside the success branch, not before the `verify`/`revoke` result.
  - *Status:* ☑ SATISFIED — Access path (`revoke.rs:30-42`): `emit_audit(AllSessionsRevoked …)` sits
    inside `if let Some(user_id) = verify_and_extract_sub(&request.token).await`, after
    `revoke_all_user_sessions`, with `request.ip_address`/`request.user_agent`. Refresh path was
    refactored into `revoke_refresh_token` (`revoke.rs:63-86`), called from both the
    `refresh_token`/`None` arm (`:46`) and the unknown-hint arm (`:51`); it queries
    `get_session_by_refresh_token` first (the port's `revoke_session` is idempotent and always
    `Ok(())`, so it cannot report a removal), and only under `Some(session)` does it revoke and emit
    `TokenRevocation` with `session.user_id` and `request` ip/ua. Both variants resolve to
    `crates/core/src/domain/audit.rs:42-44` (`TokenRevocation`, `AllSessionsRevoked`). Emission is in
    the success branch in both paths.

- **O2 — Silent on failed verification / unknown token.**
  - *Claim:* a failed signature verification and an unknown token emit no event and still return
    `Ok(())`.
  - *Evidence to collect:* read the `None` branch of `verify_and_extract_sub` handling and the
    unknown-session path; confirm no `emit_audit` call and the `Ok(())` return preserved.
  - *Status:* ☑ SATISFIED — `verify_and_extract_sub` returns `None` on a bad signature
    (`revoke.rs:106-108`); its `None` skips the `if let` body, falling straight through to `Ok(())`
    (`:43`) with no `emit_audit`. In `revoke_refresh_token`, an unknown token yields
    `existing = None` (`get_session_by_refresh_token → .ok().flatten()`), so the `if let Some` block
    (revoke + emit) is skipped and the helper returns `Ok(())` (`:85`). Every match arm of `revoke`
    returns `Ok(())`; the RFC 7009 always-`Ok` contract is intact.

- **O3 — Negative-space test.**
  - *Claim:* an invalid-signature access token and a no-matching-session refresh token each record
    nothing on `MockAuditLog`.
  - *Evidence to collect:* run the new revoke emission tests — expect PASS asserting zero events for
    both failure cases and the correct event for each success case.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange-core revoke` → 13 passed / 0 failed.
    `revoke_failed_verification_access_token_emits_nothing` (forged EdDSA sig) asserts
    `events.len() == baseline` and the session survives untouched;
    `revoke_unknown_refresh_token_emits_nothing` asserts `events.len() == 0`; both assert
    `result.is_ok()`. Positive cases `revoke_valid_access_token_emits_all_sessions_revoked` and
    `revoke_valid_refresh_token_emits_token_revocation` assert exactly one added event of the correct
    type, `Success`, correct actor, and the request ip/ua.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` → clean (exit 0); `cargo clippy --workspace
    -- -D warnings` → clean (exit 0); `cargo nextest run --workspace` → 330 passed, 0 failed,
    27 skipped. No new magic-number limits introduced (the change only wires event emission).

- **O5 — Reviewable: emit-on-success, silence-on-failure observed.**
  - *Claim:* success paths emit their event; failure paths record nothing.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core revoke` — expect PASS;
    inspect `MockAuditLog` for the presence/absence of events per case.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange-core revoke` → 13 passed. Via the
    caller-supplied `MockAuditLog` (`make_service_with_audit`), the two success cases show exactly
    one recorded event of the expected type and the two failure cases show none. Emission is not
    suppressed by the emit-threshold filter: default `emit_threshold = "info"` (Info=6); both
    `AllSessionsRevoked` (Notice=5) and `TokenRevocation` (Info=6) satisfy `severity as u8 <= 6`.

## Regression check

- `crates/server/src/routes/revoke.rs` always returns `StatusCode::OK` and ignores the `revoke`
  result → trace that emission inside `revoke` does not change the `Ok(())` return → expect the
  200 contract preserved : ☑ PRESERVED — `revoke_handler` binds the call with `let _ = state.service
  .revoke(...).await` and unconditionally returns `StatusCode::OK`, so no `revoke` outcome can alter
  the response. Independently, `revoke` still returns `Ok(())` on every arm; the added `emit_audit(...)
  .await?` cannot fail the success paths either, since default `blocking_threshold = "warning"` (4)
  and both emitted events (Notice=5, Info=6) fall below it (`severity as u8 <= 4` is false), so
  `emit_audit` returns `Ok(())` even if the audit provider errored. The 200 contract is intact.

## Residue

- `SessionRevoked` stays reserved; single-session revocation is not audited here (the refresh path
  audits `TokenRevocation`). Note only, per the change spec's decision.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence in hand — the access path emits `AllSessionsRevoked`
inside the verified-`Some(user_id)` branch and the refactored `revoke_refresh_token` emits
`TokenRevocation` only when a session actually matched (querying `get_session_by_refresh_token` first
because the idempotent `revoke_session` cannot report a removal), both carrying the request ip/ua;
failed verification and unknown tokens emit nothing and still return `Ok(())`; the four new
`MockAuditLog` tests pass, the full workspace is green (330 passed), clippy/fmt are clean, and the
server's always-200 revoke contract is PRESERVED.
