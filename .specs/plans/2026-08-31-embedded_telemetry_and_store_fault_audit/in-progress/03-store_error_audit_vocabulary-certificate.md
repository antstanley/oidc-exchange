# Done Certificate — Task 03: StoreError joins the audit vocabulary and datamodel schema

**Task:** [03-store_error_audit_vocabulary.md](03-store_error_audit_vocabulary.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-08-31

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
  - *Status:* ☑ SATISFIED — `StoreError` appended last to `AuditEventType` (audit.rs:82-86, doc
    comment records the operational, non-security classification and the deliberate exclusion
    from `SecurityEvent`) and to `AuditFailure` (audit.rs:378-381); `store_error` appended to
    `event_type` (schema:69) and `outcome.reason` (schema:85, before the trailing `null`);
    builders list it (mirror test:47, :74) with wildcard-free `match` guards (:50-56, :77-81 —
    a missing variant is a compile error, exhaustiveness confirmed).
    `cargo nextest run -p oidc-exchange-core -E 'binary(datamodel_schema_mirror)'` → 3 passed.

- **O2 — Both new variants serialize to `store_error`.**
  - *Claim:* serde renders `AuditEventType::StoreError` and `AuditFailure::StoreError` as
    `"store_error"` via the enums' existing `#[serde(rename_all = "snake_case")]`, with no
    bespoke serializer added.
  - *Evidence to collect:* confirm no per-variant `#[serde(rename)]` or custom `Serialize` impl
    was added; confirm the mirror test's rendering helper (the `rendered` mapping) produces
    `store_error` for both — the mirror equality in O1 is the executable proof; cite the relevant
    assertion lines.
  - *Status:* ☑ SATISFIED — the diff adds no per-variant `#[serde(rename)]` and no custom
    `Serialize` (only variants + doc comments); both enums keep their existing
    `#[serde(rename_all = "snake_case")]`. The mirror's `rendered()` (test:16-22) serde-renders
    each variant; the order-exact equality assertions (test:101-105, :127-131) passed, proving
    both new variants render `"store_error"`.

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
  - *Status:* ☑ SATISFIED — `jj diff` touches only audit.rs (+8: the two variants and doc
    comments), the mirror test builders, and the two schema enum lines; `SecurityEvent` and
    `into_audit_event` (audit.rs:263) untouched; the `SecurityEvent` enum body greps clean for
    `StoreError` (0 matches). The audit, exchange, exchange_mandatory_outcomes, refresh,
    refresh_mandatory_outcomes, and revoke binaries ran unmodified: 91 tests, 91 passed.
    The only wildcard-free matches over either enum in the workspace are the mirror builders
    (both updated); all other matches carry `_` arms (operator_auth.rs:62-66,
    internal_auth.rs:198-201).

- **O4 — Meets the repo definition of done.**
  - *Claim:* format, lint, and the workspace test suite pass.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done, run
    `cargo fmt --check --all`, `cargo clippy --workspace -- -D warnings`, and
    `cargo nextest run --workspace` — expect all clean/green. (The domain-type change's canonical
    sidecar update travels with the change spec's Merge plan, per the plan's Decisions — do not
    fail this obligation on the sidecar.)
  - *Status:* ☑ SATISFIED — `cargo fmt --check --all` clean (exit 0);
    `cargo clippy --workspace -- -D warnings` clean (exit 0); `cargo nextest run --workspace`
    → 927 passed, 0 failed, 78 skipped — identical to the plan's green baseline. Sidecar
    absent as the residue directs.

- **O5 — Reviewable: run the mirror test and inspect the schema diff — exactly two enum entries added (Reviewable).**
  - *Claim:* a reviewer can run the mirror test and read the `schemas/datamodel.schema.json`
    diff, observing exactly two additions (`store_error` in `event_type`, `store_error` in
    `outcome.reason`) and nothing else.
  - *Evidence to collect:* run
    `cargo nextest run -p oidc-exchange-core -E 'binary(datamodel_schema_mirror)'` — expect PASS;
    produce the schema diff and confirm its shape.
  - *Status:* ☑ SATISFIED — exercised: mirror binary run → 3 tests, 3 passed. The
    `schemas/datamodel.schema.json` diff (`jj diff -r @ --git`) contains exactly two changed
    lines, each appending one entry: `"store_error"` at the end of `event_type` and
    `"store_error"` before the trailing `null` in `outcome.reason` — nothing else.

## Regression check

- The stdout audit adapter (`crates/adapters/src/stdout_audit/mod.rs`) serializes an
  `AuditEvent` carrying an existing type (e.g. `TokenExchange`) → expect identical wire output
  to before the change : ☑ PRESERVED — both enums keep derived
  `#[serde(rename_all = "snake_case")]`; an end-appended variant changes no existing variant's
  wire name, and the adapter's own serialization tests passed in the 927-green workspace run.
- `crates/core/tests/exchange_mandatory_outcomes.rs` consumes the terminal-event mapping over
  the unchanged `SecurityEvent` set → expect the suite passes unmodified : ☑ PRESERVED — the
  suite file is untouched by the diff and passed in the targeted six-binary run (91/91) and the
  workspace run; `SecurityEvent` gains no variant, so its terminal-event mapping is unchanged.

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
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 are all SATISFIED on collected evidence — mirror binary 3/3 green, both
variants serde-render `store_error` via the existing snake_case derives, the diff is exactly
the two end-appended variants plus the two schema entries with `SecurityEvent` untouched, and
fmt/clippy/workspace (927 passed, 78 skipped, baseline-identical) are clean — with both named
regression callers PRESERVED.
