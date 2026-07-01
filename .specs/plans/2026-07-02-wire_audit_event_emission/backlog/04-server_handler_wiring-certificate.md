# Done Certificate — Task 04: server handler wiring

**Task:** [04-server_handler_wiring.md](04-server_handler_wiring.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 04. A validating agent discharges it: collect
> each obligation's evidence, run its checks, set the Status, then derive the Conclusion by the rubric.
> Do not mark an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not by
assertion.

## Premises

- **P1 — Goal.** The `/token` and `/revoke` handlers consume `Extension<AuditContext>` and thread
  its fields into the core request structs, so a real request populates the stored session.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing grant-type routing in `token_handler`
  (`crates/server/src/routes/token.rs:27-54`) or the RFC 7009 always-200 contract of
  `revoke_handler` (`crates/server/src/routes/revoke.rs:20-28`).

## Obligations

- **O1 — Handlers consume `AuditContext` and thread it in.**
  - *Claim:* `token_handler` and `revoke_handler` take `Extension<AuditContext>` and populate the
    core requests' `ip_address`/`user_agent`/`device_id` from it.
  - *Evidence to collect:* read `token.rs:23-55` and `revoke.rs:16-29`; confirm the extractor is
    present and each request struct is built with the context fields.
  - *Checks:* resolve `AuditContext` to `crates/server/src/middleware/audit_context.rs`; confirm the
    layer is installed at `crates/server/src/bootstrap.rs:135` so the extension is present at
    request time (extractor would 500 otherwise).
  - *Status:* ☐ unverified

- **O2 — Headers reach the stored session.**
  - *Claim:* a request with the audit headers stores a session with those exact values; without
    them, `None` for each.
  - *Evidence to collect:* run the new end-to-end handler test — expect PASS asserting the stored
    session's ip/ua/device equal the `X-Forwarded-For`/`User-Agent`/`X-Device-Id` values.
  - *Status:* ☐ unverified

- **O3 — Negative-space test: no headers → `None`.**
  - *Claim:* a `/token` request with no audit headers stores a session with `None` ip/ua/device.
  - *Evidence to collect:* run the no-headers handler test — expect PASS with all three session
    fields `None` (not empty strings).
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: header-to-session test passes.**
  - *Claim:* a `POST /token` with the three headers stores a session carrying those values.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-server` — expect the
    header-to-session test PASS; inspect the stored session fields in the assertion.
  - *Status:* ☐ unverified

## Regression check

- The grant-type match in `token_handler` (authorization_code / id_token / refresh_token) → trace
  each branch still builds its request and returns the same response after adding the extractor →
  expect unchanged routing : ☐ (PRESERVED / REGRESSION)
- `revoke_handler` still returns `StatusCode::OK` regardless of outcome → expect the 200 contract
  preserved : ☐ (PRESERVED / REGRESSION)

## Residue

- Emission of audit events from these flows is Tasks 05–07; Task 04 only threads the context. Not
  an obligation here.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
