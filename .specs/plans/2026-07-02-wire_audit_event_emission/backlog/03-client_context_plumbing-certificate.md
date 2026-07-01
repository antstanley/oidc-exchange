# Done Certificate — Task 03: client context plumbing

**Task:** [03-client_context_plumbing.md](03-client_context_plumbing.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 03. A validating agent discharges it: collect
> each obligation's evidence, run its checks, set the Status, then derive the Conclusion by the rubric.
> Do not mark an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not by
assertion.

## Premises

- **P1 — Goal.** The core request structs carry `ip_address`/`user_agent`/`device_id`, the exchange
  flow stores them on the session, and `create_audit_event` takes the client context.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing exchange path (session store, token issuance) in
  `crates/core/src/service/exchange.rs` or the existing `create_audit_event` callers in
  `crates/core/tests/audit.rs`.

## Obligations

- **O1 — Request fields plumbed and session populated.**
  - *Claim:* `ExchangeRequest`/`RefreshRequest`/`RevokeRequest` each carry the three
    `Option<String>` fields, and the exchange flow writes all three onto the stored `Session`.
  - *Evidence to collect:* read `exchange.rs:10-15`, `refresh.rs:8-10`, `revoke.rs:8-11` (fields
    present) and `exchange.rs:152-162` (session `device_id`/`user_agent`/`ip_address` set from
    `request.*`, no longer `None`).
  - *Checks:* resolve the `Session` field assignments to `crate::domain::Session`
    (`crates/core/src/domain/session.rs`); confirm ip/ua/device map to the matching request fields.
  - *Status:* ☐ unverified

- **O2 — `create_audit_event` records the context.**
  - *Claim:* `create_audit_event` sets `ip_address`/`user_agent` from new parameters (not hardcoded
    `None`) and takes no `device_id`.
  - *Evidence to collect:* read `crates/core/src/service/mod.rs:132-151`; confirm the signature adds
    ip/ua params and `mod.rs:146-147` uses them; confirm no `device_id` param (the `AuditEvent`
    shape has none — see `crates/core/src/domain/audit.rs`).
  - *Status:* ☐ unverified

- **O3 — Negative-space test: `None` stays `None`, values stored verbatim.**
  - *Claim:* an `ExchangeRequest` with all-`None` context stores a session with `None` for each;
    with values, stores them exactly.
  - *Evidence to collect:* run the new core session-population test — expect PASS for both the
    all-`None` and populated cases; confirm no default-string substitution.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named; every request-struct caller updated (no
    backwards-compat shim, per AI-agent rule 5).
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean (the workspace compiles with the new fields).
  - *Status:* ☐ unverified

- **O5 — Reviewable: session-population test passes, workspace builds.**
  - *Claim:* the new test proves the exchange stores the request's ip/ua/device, and every caller
    of the three request structs compiles with the new fields.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core` — expect PASS; confirm the
    workspace builds (`cargo build --workspace`).
  - *Status:* ☐ unverified

## Regression check

- `crates/server/src/routes/token.rs` and `revoke.rs` construct these request structs; trace that
  they still compile after the field addition (set to `None` here, wired in Task 04) → expect
  build success : ☐ (PRESERVED / REGRESSION)
- `crates/core/tests/audit.rs` calls `create_audit_event`; trace the updated call sites → expect the
  audit tests still pass : ☐ (PRESERVED / REGRESSION)

## Residue

- Task 04 supplies the real header values into these fields; here they may be `None` from the
  server handlers until then. Not an obligation of Task 03.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
