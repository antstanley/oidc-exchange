# Done Certificate — Task 07: An accurate `prometheus` arm in telemetry init

**Task:** [07-accurate_prometheus_telemetry_arm.md](07-accurate_prometheus_telemetry_arm.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-25 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 07. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive the
> Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do not
> record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 07) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation names
(a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `init_telemetry` matches the closed `TelemetryExporter` enum; `prometheus` warns accurately (accepted, not yet implemented, JSON-formatter fallback) and the unreachable "unknown telemetry exporter" arm is gone.
- **P2 — Obligations.** Done iff O1…O4 all hold. One Oi per definition-of-done item, in DoD order; O4 is the Reviewable item.
- **P3 — Invariants.** Must not add a Prometheus dependency to the workspace and must preserve the existing `None`/`Stdout` JSON-formatter and `Otlp`/`Xray` fallback-warning behaviour.

## Obligations

- **O1 — Every exporter initializes; prometheus warns accurately.**
  - *Claim:* every `TelemetryExporter` variant initializes without error, and `prometheus` selects the JSON formatter and produces its accepted-but-unimplemented warning path (warn-only, no Prometheus dependency).
  - *Evidence to collect:* run the new telemetry test — expect `None`/`Stdout`/`Otlp`/`Xray`/`Prometheus` each init `Ok`; read `init_telemetry` (`telemetry.rs:16-68`) and confirm `Prometheus` maps to the JSON formatter plus an accurate warning (not the old "unknown telemetry exporter" text); grep `Cargo.toml` and confirm no prometheus crate was added.
  - *Checks:* resolve the match subject in `init_telemetry` — confirm it is the closed `TelemetryExporter` enum (`config.rs:1936-1966`), not `config.exporter.as_str()`.
  - *Status:* ☐ unverified

- **O2 — Negative-space: exhaustive match, no catch-all.**
  - *Claim:* the match is exhaustive over the closed enum (no catch-all), so a future variant is a compile error rather than a silent unknown-exporter fallback.
  - *Evidence to collect:* read `init_telemetry` and confirm the `match` has an arm per variant and no `_ =>` catch-all; confirm the unknown-exporter arm (formerly `telemetry.rs:62`) is deleted; reason that adding a variant to `TelemetryExporter` would fail to compile until an arm is added.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* the exporter arms are tested with meaningful assertions and format/lint/test gates pass.
  - *Evidence to collect:* run `cargo fmt` (check), `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean (per [development-guidelines.md](../../../development-guidelines.md) §Definition of done).
  - *Status:* ☐ unverified

- **O4 — Reviewable: prometheus warns and falls back, no unknown branch (Reviewable).**
  - *Claim:* a reviewer initializes telemetry with `exporter = "prometheus"` and confirms the accurate warning and JSON fallback, with the unknown-exporter branch removed from the source.
  - *Evidence to collect:* run the prometheus-arm test and read the warning text plus the selected JSON formatter; grep the source to confirm no "unknown telemetry exporter" string remains.
  - *Status:* ☐ unverified

## Regression check

- Existing telemetry init callers on the server/lambda entry points: trace `init_telemetry` with `exporter = "none"` and `"otlp"` → expect the JSON formatter and the existing otlp fallback warning respectively, unchanged : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: this task ships only the accurate warn-arm; a real Prometheus pipeline is scoped by the pending `2026-06-24-complete_telemetry_exporters.md` change. Whichever change merges second must re-verify `07`'s exporter list against `init_telemetry` (see plan.md Open questions) — a merge-coordination note, not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
