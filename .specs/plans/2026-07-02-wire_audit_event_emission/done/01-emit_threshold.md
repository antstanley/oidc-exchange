# Task 01 — emit threshold filter

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-emit_threshold-certificate.md](01-emit_threshold-certificate.md)

**Implements:** [06-configuration.md](../../../service/specs/06-configuration.md) §Sections → `[audit]` and §Defaults summary; [07-telemetry-and-audit.md](../../../service/specs/07-telemetry-and-audit.md) §Audit (the `emit_threshold` pre-dispatch filter)
**Depends on:** —
**Produces:** `emit_audit` drops events strictly less severe than `[audit] emit_threshold` before any adapter dispatch; a new `emit_threshold` config key defaults to `info`.
**Pointers:** `crates/core/src/config.rs:81` (`AuditConfig` struct + `Default`); `crates/core/src/service/mod.rs:102` (`emit_audit`), `mod.rs:153` (`parse_severity`); `config/default.toml` (`[audit]` section); `crates/core/tests/audit.rs` (existing threshold tests)

## Steps

- [x] Add `emit_threshold: String` to `AuditConfig` (`config.rs:81`) and set its `Default` to `"info"` (`config.rs:87`).
- [x] At the top of `AppService::emit_audit` (`mod.rs:102`), parse `self.config.audit.emit_threshold` with `parse_severity`, falling back to `AuditSeverity::Info` when unparseable, and return `Ok(())` before the adapter dispatch when the event's severity is strictly less severe than the threshold (`event.severity as u8 > threshold as u8`).
- [x] Reflect the `info` default in `config/default.toml`'s `[audit]` section if the section pins values explicitly.
- [x] Add unit/integration tests: a debug event under the default `info` threshold is dropped (adapter never sees it, returns `Ok`); an `info` event at `info` threshold is dispatched; lowering the threshold to `debug` lets the debug event through.

## Definition of done

- [x] `emit_audit` returns `Ok(())` without dispatching when the event severity is strictly less severe than `emit_threshold`, and dispatches (retaining the existing fallback/blocking behaviour) otherwise.
- [x] `emit_threshold` is a named config field with an `info` default; the severity comparison reuses `parse_severity` and no new numeric severity literal is introduced.
- [x] Negative-space test: a `Debug`-severity event under the default `info` threshold is suppressed and never reaches the adapter (asserted against `MockAuditLog`).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run `cargo nextest run -p oidc-exchange-core audit` and observe the suppression test pass, then confirm the existing `crates/core/tests/audit.rs` blocking-threshold tests still pass unchanged.
