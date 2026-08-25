# Done Certificate — Task 03: Thread real client provenance into the core flows

**Task:** [03-thread_client_provenance.md](03-thread_client_provenance.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-25 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 03. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive the
> Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do not
> record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation names
(a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Core-flow (`exchange`/`refresh`/`revoke`) audit events record the middleware's true `ip_address_source` (`peer`/`forwarded`/`unknown`) instead of the flattened `asserted`.
- **P2 — Obligations.** Done iff O1…O4 all hold. One Oi per definition-of-done item, in DoD order; O4 is the Reviewable item.
- **P3 — Invariants.** Must not change the stored `Session.ip_address` value (still derived via `audit_address()`), must keep `RefreshRequest`/`RevokeRequest` `#[derive(Default)]` working, and must leave the `Asserted` variant available for embedder hints.

## Obligations

- **O1 — Terminal audit events record resolved provenance.**
  - *Claim:* a `/token` terminal audit event records `ip_address_source == "peer"`, `"forwarded"` behind a `server.trusted_proxies` proxy, and `"unknown"` when no server-established address exists.
  - *Evidence to collect:* run the new server e2e (beside `crates/server/tests/e2e.rs`) driving `/token` through the production router — read the emitted audit event's `ip_address_source` for the peer, trusted-forwarded, and no-address cases; expect `peer`/`forwarded`/`unknown` respectively, never `asserted`.
  - *Checks:* confirm the `ClientAddr::asserted(request.ip_address)` rebuilds are deleted at `exchange.rs:121-125`, `refresh.rs:161-174` (argument swap only), and `revoke.rs:44-48`; resolve `request.client_addr` to the `ClientAddr` field, not a shadowed local.
  - *Status:* ☐ unverified

- **O2 — Stored value unchanged; `ClientAddr::default()` is `Unknown`.**
  - *Claim:* the stored `Session.ip_address` value is unchanged (via `audit_address()`), `ClientAddr::default()` is `Unknown`, and the `Asserted` variant remains in the domain.
  - *Evidence to collect:* read `exchange.rs:491` and confirm `Session.ip_address` is populated via `request.client_addr.audit_address()`; run a unit test asserting `ClientAddr::default()` matches `Unknown`; grep `domain/audit.rs` and confirm the `Asserted` variant is still present.
  - *Checks:* trace one concrete `Peer` address through `audit_address()` and confirm the stored string equals the prior `ip_address()` output.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* the behaviour is tested with meaningful assertions and format/lint/test gates pass.
  - *Evidence to collect:* run `cargo fmt` (check), `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean, including the updated core-test request constructors across `crates/core/tests/{exchange,exchange_mandatory_outcomes,assertion,refresh,revoke,service_leak_corpus,user_admin}.rs` (per [development-guidelines.md](../../../development-guidelines.md) §Definition of done).
  - *Status:* ☐ unverified

- **O4 — Reviewable: `/token` audit records resolved source, not `asserted` (Reviewable).**
  - *Claim:* a reviewer drives `/token` through the production router and inspects the emitted audit event's `ip_address_source`, confirming it is the resolved `peer`/`forwarded` value rather than `asserted`.
  - *Evidence to collect:* run the provenance e2e and read the `ip_address_source` field of the terminal event for the peer and trusted-forwarded cases; confirm neither is `asserted`.
  - *Status:* ☐ unverified

## Regression check

- Route handlers `token.rs:245,263,275` and `revoke.rs:53` now pass `audit_ctx.client_addr.clone()` instead of `audit_ctx.ip_address()`: trace one `/revoke` call → expect the revoke audit event still carries a well-formed address and the stored session address is value-identical to before : ☐ (PRESERVED / REGRESSION)
- The existing throttle/provenance e2e `production_router_uses_observed_peer_and_trusted_forwarding_for_audit_and_throttle` (`e2e.rs:203`): expect it still passes (throttle keys were already provenance-aware) : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: SIEM queries filtering core-flow events on the old `asserted` constant will now see `peer`/`forwarded`/`unknown` — an intended, documented behaviour change, not a regression.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
