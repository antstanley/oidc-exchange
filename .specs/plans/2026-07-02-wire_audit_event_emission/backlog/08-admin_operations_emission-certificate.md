# Done Certificate — Task 08: admin operations emission

**Task:** [08-admin_operations_emission.md](08-admin_operations_emission.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 08. A validating agent discharges it: collect
> each obligation's evidence, run its checks, set the Status, then derive the Conclusion by the rubric.
> Do not mark an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 08) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not by
assertion.

## Premises

- **P1 — Goal.** Admin mutations emit their named audit events and read-only operations emit none.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing admin returns or the best-effort user-sync
  notify (warn-and-continue) in `crates/core/src/service/user_admin.rs`.

## Obligations

- **O1 — Each mutation emits its named event; reads emit none.**
  - *Claim:* `admin_create_user` → `UserCreated`; `admin_update_user` → `UserUpdated` (and
    `UserSuspended` when the patch sets `status = Suspended`); `admin_delete_user` → `UserDeleted`;
    the three claims mutations → `UserUpdated` with the operation in `detail`; reads emit nothing.
  - *Evidence to collect:* read `user_admin.rs:14`, `:33`, `:74`, `:120`, `:152`, `:173`; confirm
    each mutation emits via `create_audit_event` (`None` ip/ua) + `emit_audit`, and that
    `admin_get_user`/`admin_list_users`/`admin_stats`/`admin_get_claims` have no `emit_audit` call.
  - *Checks:* resolve the `AuditEventType` variants to `crates/core/src/domain/audit.rs`; trace the
    `admin_update_user` suspend branch selects `UserSuspended` when `patch.status == Some(Suspended)`.
  - *Status:* ☐ unverified

- **O2 — Admin audit follows blocking rules.**
  - *Claim:* an admin audit failure under the blocking threshold propagates as `Err`, unlike the
    best-effort user-sync notify that only logs a warning.
  - *Evidence to collect:* trace one mutation's emission — confirm it propagates `emit_audit`'s
    `Result` (via `?`), distinct from the `if let Err(e) = self.user_sync…` warn-only pattern at
    `user_admin.rs:16-18`.
  - *Status:* ☐ unverified

- **O3 — Negative-space test: reads record nothing.**
  - *Claim:* `admin_get_user`/`admin_list_users`/`admin_stats`/`admin_get_claims` each record zero
    events on `MockAuditLog`.
  - *Evidence to collect:* run the new admin emission tests — expect PASS asserting zero events for
    each read and the expected event for each mutation (including the suspend-path `UserSuspended`).
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: mutations emit, reads do not.**
  - *Claim:* each mutation's event (and the suspend-path `UserSuspended`) is recorded while reads
    record nothing.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core user_admin` — expect PASS;
    inspect `MockAuditLog` events per operation.
  - *Status:* ☐ unverified

## Regression check

- The best-effort user-sync notify calls (`notify_user_created`/`updated`/`deleted`) → trace that
  adding a blocking `emit_audit` does not change their warn-and-continue behaviour → expect the
  sync path unchanged : ☐ (PRESERVED / REGRESSION)
- `admin_delete_user` revokes all sessions before returning → trace that emission is added after the
  revoke, preserving the delete semantics → expect unchanged : ☐ (PRESERVED / REGRESSION)

## Residue

- Whether claims mutations should each carry a distinct `detail` operation name (`set`/`merge`/
  `clear`) is specified as "operation in `detail`"; the exact key is an implementation detail, not
  a separate obligation.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
