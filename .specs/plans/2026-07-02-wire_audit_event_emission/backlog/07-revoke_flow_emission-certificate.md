# Done Certificate — Task 07: revoke flow emission

**Task:** [07-revoke_flow_emission.md](07-revoke_flow_emission.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — Silent on failed verification / unknown token.**
  - *Claim:* a failed signature verification and an unknown token emit no event and still return
    `Ok(())`.
  - *Evidence to collect:* read the `None` branch of `verify_and_extract_sub` handling and the
    unknown-session path; confirm no `emit_audit` call and the `Ok(())` return preserved.
  - *Status:* ☐ unverified

- **O3 — Negative-space test.**
  - *Claim:* an invalid-signature access token and a no-matching-session refresh token each record
    nothing on `MockAuditLog`.
  - *Evidence to collect:* run the new revoke emission tests — expect PASS asserting zero events for
    both failure cases and the correct event for each success case.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: emit-on-success, silence-on-failure observed.**
  - *Claim:* success paths emit their event; failure paths record nothing.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core revoke` — expect PASS;
    inspect `MockAuditLog` for the presence/absence of events per case.
  - *Status:* ☐ unverified

## Regression check

- `crates/server/src/routes/revoke.rs` always returns `StatusCode::OK` and ignores the `revoke`
  result → trace that emission inside `revoke` does not change the `Ok(())` return → expect the
  200 contract preserved : ☐ (PRESERVED / REGRESSION)

## Residue

- `SessionRevoked` stays reserved; single-session revocation is not audited here (the refresh path
  audits `TokenRevocation`). Note only, per the change spec's decision.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
