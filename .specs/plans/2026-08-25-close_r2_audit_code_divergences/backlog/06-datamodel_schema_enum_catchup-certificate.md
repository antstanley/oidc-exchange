# Done Certificate — Task 06: Catch `schemas/datamodel.schema.json` up with the code

**Task:** [06-datamodel_schema_enum_catchup.md](06-datamodel_schema_enum_catchup.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-25 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 06. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive the
> Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do not
> record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 06) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation names
(a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `schemas/datamodel.schema.json`'s `AuditEvent` mirrors the shipped `AuditEventType` (18) and `AuditFailure` (9) variants plus an optional `operator`, guarded by a mirror test so the next enum addition fails a test.
- **P2 — Obligations.** Done iff O1…O4 all hold. One Oi per definition-of-done item, in DoD order; O4 is the Reviewable item.
- **P3 — Invariants.** Must not add `operator` to `AuditEvent.required` (it is `None` on the exchange plane), and must keep the operator definitions consistent with the published `schemas/internal-api.schema.json:114-126` shape.

## Obligations

- **O1 — Mirror test asserts enum equality.**
  - *Claim:* a mirror test (no new dependencies) reads the schema and asserts its `event_type` and `outcome.reason` enum arrays equal the serde-rendered variant lists of `AuditEventType` and `AuditFailure`.
  - *Evidence to collect:* run the new mirror test in `crates/core` — expect PASS against the updated schema; read the schema's `event_type` (`datamodel.schema.json:69`) and confirm it lists the 18 values including `refresh_token_reuse`/`missing_credential`/`invalid_credential`/`not_configured`, and `outcome.reason` (`:81`) the 9 values plus `null`.
  - *Checks:* resolve the variant-list source the test compares against — confirm it is the serde serialization of `AuditEventType` (`audit.rs:56-81`) and `AuditFailure` (`audit.rs:344-360`), not a hand-copied literal in the test.
  - *Status:* ☐ unverified

- **O2 — Negative-space: exact-set equality.**
  - *Claim:* the test fails if a variant is added to either enum without updating the schema.
  - *Evidence to collect:* confirm the test asserts exact-set equality (not subset) — read the assertion and verify a scratch variant (or an inline reasoning trace) would fail it; confirm the optional `operator` property and its `OperatorPrincipal`/`OperatorAuthMechanism` definitions were added under `definitions` with `required` unchanged.
  - *Checks:* trace one direction of the equality — a variant present in the enum but absent from the schema array → expect the test to fail.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* the mirror test carries meaningful assertions and format/lint/test gates pass.
  - *Evidence to collect:* run `cargo fmt` (check), `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean (per [development-guidelines.md](../../../development-guidelines.md) §Definition of done).
  - *Status:* ☐ unverified

- **O4 — Reviewable: mirror test passes and would fail on drift (Reviewable).**
  - *Claim:* a reviewer runs the mirror test and confirms it passes against the updated schema and would fail on an un-mirrored enum addition.
  - *Evidence to collect:* run the mirror test (expect PASS) and read the exact-set assertion to confirm the drift-detection property (a variant added to either enum without a schema update fails the test).
  - *Status:* ☐ unverified

## Regression check

- Existing consumers of `datamodel.schema.json` (any schema-validation tests or generators referencing it): expect the widened enums and added optional `operator` to remain backward-compatible — trace one existing `AuditEvent` fixture against the updated schema → expect it still validates : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: this task closes only the code-side leg of `08-persistence.md`'s mirror sentence (`datamodel.schema.json` ↔ typed entities). The sidecar leg (`canonical-types.schema.json`) stays intentionally stale and belongs to the deferred doc-only pass — not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
