# Task 01 — Idempotent, host-respecting telemetry init

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-idempotent_telemetry_init-certificate.md](01-idempotent_telemetry_init-certificate.md)

**Implements:** [Change spec §The delta → G1](../../../changes/2026-08-31-embedded_telemetry_and_store_fault_audit.md#g1--install-the-telemetry-subscriber-on-the-embedded-entrypoint), first bullet (the `try_init` switch) and the server-side companion test. Makes true the idempotent/host-respecting sentences of [07-telemetry-and-audit.md §Telemetry](../../../service/specs/07-telemetry-and-audit.md).
**Depends on:** —
**Produces:** `init_telemetry` callable any number of times: the first call installs the JSON subscriber exactly as today; every later call — or a call after a host installed its own global dispatcher — returns `Ok(())`, notes the retained subscriber at debug level, and skips the exporter fallback warning. The double-init panic is unrepresentable.
**Pointers:** `crates/server/src/telemetry.rs:22-47` (`init_telemetry`; the `.init()` call at lines 37-40); `crates/server/src/telemetry.rs:58-78` (`exporter_fallback_warning` — unchanged, but the warning emission moves inside the installed-path branch); `crates/server/src/main.rs:29` (the sole current caller, whose behaviour must not change); new integration binary under `crates/server/tests/` (a global dispatcher is process-wide, so the double-init scenario needs its own test binary, not a unit test in `telemetry.rs`).

## Steps

- [x] Replace the `tracing_subscriber::fmt()…​.init()` call with `.try_init()` in `init_telemetry`.
- [x] On `Err` from `try_init` (a global dispatcher already set — an earlier `init_telemetry` call or a host-owned subscriber), return `Ok(())` after emitting a `tracing::debug!` through the existing dispatcher noting the installed subscriber is retained.
- [x] Emit the exporter fallback warning only on the successful-install path — on the retained path nothing was installed, so the "falling back to stdout JSON" claim would be false.
- [x] Keep the first-call behaviour identical: install, then warn for `otlp`/`xray`/`prometheus` when `enabled = true` (the `exporter_fallback_warning` classifier and its unit tests stay as they are).
- [x] Add an integration binary in `crates/server/tests/` (e.g. `telemetry_reinit.rs`): call `init_telemetry` twice with a minimal `TelemetryConfig`; both calls return `Ok`, and `tracing::dispatcher::has_been_set()` is true after the first.
- [x] In its own integration binary (e.g. `crates/server/tests/telemetry_retained.rs`) — or as the sole `#[test]` in its binary — install a capturing subscriber via `tracing::subscriber::set_global_default` first, then call `init_telemetry` with an `otlp` config: the call returns `Ok` and the capture holds the debug retention note but no fallback warning. Global-dispatcher scenarios need process isolation: nextest gives one process per test, but do not rely on same-binary tests sharing state — under plain `cargo test` this scenario and the double-init test would share a process and race for the global dispatcher, making the suite order-dependent.

## Definition of done

- [x] A new `crates/server/tests/` binary proves `init_telemetry` twice returns `Ok` both times — the double-init panic is gone.
- [x] The retained-dispatcher path is pinned: with a pre-installed global subscriber, `init_telemetry` returns `Ok`, emits the debug retention note through the host's subscriber, and emits no exporter fallback warning (negative space for the warning move).
- [x] First-call behaviour is unchanged: the existing `telemetry.rs` unit tests (`exporter_fallback_warning` classification, flush idempotency) and the server e2e suite pass without modification.
- [x] `init_telemetry` keeps its `Result` signature and gains no new dependencies; touched code carries meaningful assertions per the repo baseline.
- [x] Meets the repo definition of done (`cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [x] Reviewable: run the new server-tests binary (`cargo nextest run -p oidc-exchange -E 'binary(telemetry_reinit)'` or equivalent) and observe both init calls succeed in one process.
