# Task 07 — An accurate `prometheus` arm in telemetry init

**Plan:** [plan.md](../plan.md) · **Certificate:** [07-accurate_prometheus_telemetry_arm-certificate.md](07-accurate_prometheus_telemetry_arm-certificate.md)

**Implements:** [07-telemetry-and-audit.md](../../../service/specs/07-telemetry-and-audit.md) §Telemetry (exporter behaviour); change spec §The delta → S16-code
**Depends on:** —
**Produces:** `init_telemetry` matches the closed `TelemetryExporter` enum; `prometheus` warns accurately (accepted, not yet implemented, JSON-formatter fallback) and the unreachable "unknown telemetry exporter" arm is gone.
**Pointers:** `crates/server/src/telemetry.rs:16-68` (`init_telemetry`, `exporter.as_str()` at `:28`, unknown-exporter arm at `:62`); `crates/core/src/config.rs:1936-1966` (`TelemetryExporter`: `Prometheus` at `:1941`)

## Steps

- [ ] Replace the `config.exporter.as_str()` string match in `init_telemetry` (`telemetry.rs:28`) with a match on the closed `TelemetryExporter` enum.
- [ ] Map `None`/`Stdout` → the JSON formatter, `Otlp`/`Xray` → their existing fallback warnings, `Prometheus` → JSON formatter plus a warning naming prometheus as accepted but not yet implemented (no metrics exported, no metrics endpoint exposed).
- [ ] Delete the unreachable "unknown telemetry exporter" arm (`:62`); the closed domain makes a config-valid value unrepresentable as unknown.

## Definition of done

- [ ] A test asserts every `TelemetryExporter` variant initializes without error, and `prometheus` selects the JSON formatter and produces its accepted-but-unimplemented warning path (warn-only, no Prometheus dependency).
- [ ] Negative-space / exhaustiveness: the match is exhaustive over the closed enum (no catch-all), so a future variant is a compile error rather than a silent unknown-exporter fallback.
- [ ] Meets the repo definition of done (test, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [ ] Reviewable: a reviewer initializes telemetry with `exporter = "prometheus"` and confirms the accurate warning and JSON fallback, with the unknown-exporter branch removed from the source.

## Open questions

- Coordinate the merge with the pending `2026-06-24-complete_telemetry_exporters.md` change (see plan.md Open questions) — this task ships only the warn-arm, not a real Prometheus pipeline.
