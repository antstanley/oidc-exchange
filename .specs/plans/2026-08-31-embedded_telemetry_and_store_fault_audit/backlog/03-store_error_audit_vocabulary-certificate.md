# Done Certificate — Task 03: StoreError joins the audit vocabulary and datamodel schema

**Task:** [03-store_error_audit_vocabulary.md](03-store_error_audit_vocabulary.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-31 — unverified

> Verification protocol for Task 03. A validating agent discharges it: collect each obligation's
> evidence, run its checks, set the Status, then derive the Conclusion by the rubric. Do not mark
> an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The audit vocabulary can name an infrastructure store fault:
  `AuditEventType::StoreError` and `AuditFailure::StoreError` exist (both serialized
  `store_error`), mirrored into both `schemas/datamodel.schema.json` enums and enforced by the
  mirror test, with the closed `SecurityEvent` set untouched.
- **P2 — Obligations.** Done iff O1…O5 all hold, one per definition-of-done item in DoD order;
  O5 is the Reviewable item.
- **P3 — Invariants.** Must not break: every existing `AuditEventType`/`AuditFailure` value and
  serialized name; the closed `SecurityEvent` enum (`crates/core/src/domain/audit.rs:165-190`
  pre-change) and its `into_audit_event` mappings; the existing audit, exchange, refresh, and
  revoke suites. The change is purely additive vocabulary — no flow behaviour changes in this
  task (that is Task 04).

## Obligations

- **O1 — The mirror test passes with `store_error` present in both enum mirrors.**
  - *Claim:* `crates/core/tests/datamodel_schema_mirror.rs` compiles and passes: its exhaustive
    `all_event_types` and `all_failures` builders include the new variants, and the schema
    equality holds against `schemas/datamodel.schema.json`.
  - *Evidence to collect:* read `crates/core/src/domain/audit.rs` — confirm `StoreError` added to
    `AuditEventType` (with a doc comment recording the operational, non-security classification)
    and to `AuditFailure`. Read `schemas/datamodel.schema.json` — confirm `store_error` appended
    to the `event_type` enum (line 69 pre-change) and the `outcome.reason` enum (line 85
    pre-change). Read the mirror test's builders (`all_event_types` at line 26, `all_failures` at
    line 61 pre-change) — confirm both list the new variant. Run
    `cargo nextest run -p oidc-exchange-core -E 'binary(datamodel_schema_mirror)'` — expect PASS.
  - *Checks:* the mirror test is the compile-enforced guard: confirm the builders are exhaustive
    `vec![...]` constructions the compiler ties to the enum (an added variant missing from a
    builder must be a test failure, not silently absent). If the builders use a pattern that does
    not force exhaustiveness, flag it.
  - *Status:* ☐ unverified

- **O2 — Both new variants serialize to `store_error`.**
  - *Claim:* serde renders `AuditEventType::StoreError` and `AuditFailure::StoreError` as
    `"store_error"` via the enums' existing `#[serde(rename_all = "snake_case")]`, with no
    bespoke serializer added.
  - *Evidence to collect:* confirm no per-variant `#[serde(rename)]` or custom `Serialize` impl
    was added; confirm the mirror test's rendering helper (the `rendered` mapping) produces
    `store_error` for both — the mirror equality in O1 is the executable proof; cite the relevant
    assertion lines.
  - *Status:* ☐ unverified

- **O3 — Negative space: `SecurityEvent` unchanged; no existing enum value or serialized name changes; existing suites pass unmodified.**
  - *Claim:* the closed security-outcome set is not extended and nothing existing is renamed or
    reordered in a serialization-visible way.
  - *Evidence to collect:* diff `crates/core/src/domain/audit.rs` — expect the only enum deltas
    to be the two added `StoreError` variants; expect `SecurityEvent` and its `into_audit_event`
    mapping untouched (no store-fault variant, per the change spec's Decisions). Diff
    `schemas/datamodel.schema.json` — expect exactly two enum entries added and nothing else. Run
    `cargo nextest run -p oidc-exchange-core` — expect `audit.rs`, `exchange.rs`,
    `exchange_mandatory_outcomes.rs`, `refresh.rs`, `refresh_mandatory_outcomes.rs`, and
    `revoke.rs` suites green without edits.
  - *Checks:* grep `SecurityEvent` for `StoreError` — expect no match.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* format, lint, and the workspace test suite pass.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done, run
    `cargo fmt --check --all`, `cargo clippy --workspace -- -D warnings`, and
    `cargo nextest run --workspace` — expect all clean/green. (The domain-type change's canonical
    sidecar update travels with the change spec's Merge plan, per the plan's Decisions — do not
    fail this obligation on the sidecar.)
  - *Status:* ☐ unverified

- **O5 — Reviewable: run the mirror test and inspect the schema diff — exactly two enum entries added (Reviewable).**
  - *Claim:* a reviewer can run the mirror test and read the `schemas/datamodel.schema.json`
    diff, observing exactly two additions (`store_error` in `event_type`, `store_error` in
    `outcome.reason`) and nothing else.
  - *Evidence to collect:* run
    `cargo nextest run -p oidc-exchange-core -E 'binary(datamodel_schema_mirror)'` — expect PASS;
    produce the schema diff and confirm its shape.
  - *Status:* ☐ unverified

## Regression check

- The stdout audit adapter (`crates/adapters/src/stdout_audit/mod.rs`) serializes an
  `AuditEvent` carrying an existing type (e.g. `TokenExchange`) → expect identical wire output
  to before the change : ☐ (PRESERVED / REGRESSION)
- `crates/core/tests/exchange_mandatory_outcomes.rs` consumes the terminal-event mapping over
  the unchanged `SecurityEvent` set → expect the suite passes unmodified : ☐ (PRESERVED / REGRESSION)

## Residue

- The canonical sidecar `canonical-types.schema.json` and the `01-domain-model.md` prose list
  (including the republished-list completeness fold of the three operator-auth variants) are the
  change spec's Merge plan, deliberately outside this task — do not treat their absence as a gap.
- SIEM consumers validating against the old schema enums must take the updated schema (change
  spec §Compatibility); no obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric:
NOT_DONE — any load-bearing obligation UNSATISFIED, or a REGRESSION found.
PARTIAL — all obligations SATISFIED except one or more UNVERIFIED, and no regression.
DONE — every obligation SATISFIED, regression PRESERVED, evidence sufficient for each. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
