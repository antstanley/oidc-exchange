# Done Certificate — Task 01: Idempotent, host-respecting telemetry init

**Task:** [01-idempotent_telemetry_init.md](01-idempotent_telemetry_init.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-08-31

> Verification protocol for Task 01. A validating agent discharges it: collect each obligation's
> evidence, run its checks, set the Status, then derive the Conclusion by the rubric. Do not mark
> an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `init_telemetry` is callable any number of times: the first call installs the
  JSON subscriber exactly as today; a later call, or a call after a host set its own global
  dispatcher, returns `Ok(())`, notes the retained subscriber at debug level, and skips the
  exporter fallback warning.
- **P2 — Obligations.** Done iff O1…O6 all hold, one per definition-of-done item in DoD order;
  O6 is the Reviewable item.
- **P3 — Invariants.** Must not break the standalone server's startup: `crates/server/src/main.rs:29`
  is the sole pre-existing caller and runs before any subscriber can exist, so its call must still
  install the subscriber and still warn for `otlp`/`xray`/`prometheus` when telemetry is enabled.
  The `exporter_fallback_warning` classifier and the existing `telemetry.rs` unit tests must be
  untouched in behaviour.

## Obligations

- **O1 — A new `crates/server/tests/` binary proves `init_telemetry` twice returns `Ok` both times.**
  - *Claim:* calling `init_telemetry` a second time in the same process returns `Ok(())` instead
    of panicking — the double-init panic is unrepresentable.
  - *Evidence to collect:* read `crates/server/src/telemetry.rs` and confirm the
    `tracing_subscriber::fmt()` builder chain ends in `.try_init()` (not `.init()`), with the
    `Err` branch mapping to `Ok(())`. Locate the new integration binary under
    `crates/server/tests/` (the task suggests `telemetry_reinit.rs`); confirm it calls
    `init_telemetry` twice and asserts both results are `Ok` and that
    `tracing::dispatcher::has_been_set()` is true after the first call. Run that binary via
    `cargo nextest run` — expect PASS. Confirm it is its own test binary, not a `#[cfg(test)]`
    module in `telemetry.rs` (the global dispatcher is process-wide).
  - *Checks:* resolve `try_init` at the call site — confirm it is
    `tracing_subscriber`'s `SubscriberInitExt`/fmt-builder `try_init` (whose error case is an
    already-set global dispatcher), not a same-named local helper.
  - *Status:* SATISFIED — `telemetry.rs:54-58` ends the builder chain in `.try_init().is_err()`;
    the `Err` branch returns `Ok(())` at `telemetry.rs:71`. `crates/server/tests/telemetry_reinit.rs`
    is its own integration binary (sole test), calls `init_telemetry` twice, asserts both `Ok` and
    `tracing::dispatcher::has_been_set()` after the first call. `cargo nextest run -p oidc-exchange
    -E 'binary(telemetry_reinit)…'` → PASS. `try_init` resolves to the inherent
    `SubscriberBuilder::try_init` (tracing-subscriber-0.3.23 `fmt/mod.rs:503` →
    `SubscriberInitExt::try_init`, `util.rs:61`, which calls `set_global_default` first); no local
    helper shadows it.

- **O2 — The retained-dispatcher path is pinned: `Ok`, a debug retention note, no fallback warning.**
  - *Claim:* with a global subscriber already installed, `init_telemetry` returns `Ok`, emits a
    `tracing::debug!` noting the installed subscriber is retained, and does not emit the exporter
    fallback warning even for a warn-carrying exporter such as `otlp`.
  - *Evidence to collect:* read the `Err` branch in `init_telemetry` — confirm the debug emission
    and confirm the `exporter_fallback_warning` message is emitted only on the successful-install
    path. Run the named test that pre-installs a capturing subscriber via
    `tracing::subscriber::set_global_default`, then calls `init_telemetry` with an `otlp`-exporter
    `TelemetryConfig`; expect PASS with assertions that the capture holds the retention note and
    holds no fallback warning (the negative-space assertion must be present, not implied).
  - *Checks:* trace one concrete input: `TelemetryConfig { enabled: true, exporter: Otlp, .. }`
    with a dispatcher already set → `try_init` errs → `Ok(())` returned, debug emitted, warning
    skipped. Confirm the warning-skip is on the retained path only — the first-install path with
    `otlp` must still warn (O3).
  - *Status:* SATISFIED — the debug retention note is at `telemetry.rs:67-70` inside the `Err`
    branch; the warning emission at `telemetry.rs:76-78` sits after the guard, installed path only.
    `crates/server/tests/telemetry_retained.rs` pre-installs a DEBUG-capturing subscriber via
    `tracing::subscriber::set_global_default`, calls `init_telemetry` with
    `enabled: true, exporter: Otlp`, and asserts `Ok`, capture contains
    "retaining the existing subscriber", and the explicit negative assertion
    `!rendered.contains("falling back to stdout JSON")` (the real otlp warning text at
    `telemetry.rs:98`). PASS. Trace: otlp + dispatcher-set → `try_init` errs → `Ok(())`, debug
    emitted, warning skipped; first-install with otlp still reaches the warn (see O3/regression).

- **O3 — First-call behaviour is unchanged; existing telemetry unit tests and the server e2e suite pass unmodified.**
  - *Claim:* the first `init_telemetry` call still installs the JSON subscriber and still emits
    the fallback warning for `otlp`/`xray`/`prometheus`; no pre-existing test was edited to make
    the suite pass.
  - *Evidence to collect:* diff `crates/server/src/telemetry.rs`'s `exporter_fallback_warning`
    (lines 58-78 pre-change) — expect no behavioural change; diff the in-file unit tests
    (`flush_telemetry_is_idempotent_and_does_not_panic`, `disabled_telemetry_does_not_panic`,
    `every_exporter_is_classified_and_prometheus_warns_accurately`) — expect untouched. Run
    `cargo nextest run -p oidc-exchange` — expect the full server suite (including
    `crates/server/tests/e2e.rs`) green.
  - *Status:* SATISFIED — the diff touches `exporter_fallback_warning` not at all (only a nearby
    comment in `init_telemetry` was reworded) and leaves the three in-file unit tests byte-untouched.
    `cargo nextest run --workspace` (superset of `-p oidc-exchange`, including the e2e binary):
    929 passed / 0 failed / 78 skipped — baseline 927 + the 2 new tests, no pre-existing test
    modified or lost.

- **O4 — Signature preserved, no new dependencies, meaningful assertions.**
  - *Claim:* `init_telemetry` keeps its `Result<(), Box<dyn std::error::Error>>` signature,
    `crates/server/Cargo.toml` gains no new dependency, and the touched function carries
    meaningful assertions per the repo baseline.
  - *Evidence to collect:* read the function signature; diff `crates/server/Cargo.toml` — expect
    no `[dependencies]` change; inspect the touched code for assertions (or a written reason in
    the change description where the two-assertion guideline is not met — it is a review gate,
    not a lint).
  - *Status:* SATISFIED — signature unchanged:
    `pub fn init_telemetry(config: &TelemetryConfig) -> Result<(), Box<dyn std::error::Error>>`
    (`telemetry.rs:39`). `jj diff -r @ --stat` lists only `telemetry.rs` and the two new test
    files — no Cargo.toml change, no new dependency (both test binaries use only existing deps:
    `tracing`, `tracing-subscriber`, `oidc-exchange-core`). Each new test carries 4+ message-bearing
    assertions pinning the exact claim (both `Ok`s, `has_been_set`, retention note present,
    fallback warning absent).

- **O5 — Meets the repo definition of done.**
  - *Claim:* format, lint, and the workspace test suite pass.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done, run
    `cargo fmt --check --all`, `cargo clippy --workspace -- -D warnings`, and
    `cargo nextest run --workspace` — expect all clean/green.
  - *Status:* SATISFIED — all three run in the task workspace: `cargo fmt --check --all` exit 0;
    `cargo clippy --workspace -- -D warnings` exit 0; `cargo nextest run --workspace`
    929 passed / 0 failed / 78 skipped.

- **O6 — Reviewable: run the new server-tests binary and observe both init calls succeed in one process (Reviewable).**
  - *Claim:* a reviewer can run the new binary in isolation and observe the double-init scenario
    pass.
  - *Evidence to collect:* run the binary alone (e.g.
    `cargo nextest run -p oidc-exchange -E 'binary(telemetry_reinit)'`, adjusting to the actual
    binary name) — expect PASS with the double-init test listed in the output.
  - *Status:* SATISFIED — ran `cargo nextest run -p oidc-exchange -E 'binary(telemetry_reinit) or
    binary(telemetry_retained)'`: 2 tests across 2 binaries, both PASS —
    `telemetry_reinit::init_telemetry_twice_returns_ok_both_times` (both init calls succeed in one
    process) and `telemetry_retained::init_telemetry_retains_host_subscriber_and_skips_fallback_warning`.

## Regression check

- `crates/server/src/main.rs:29` calls `init_telemetry(&config.telemetry)?` at startup with no
  dispatcher yet set → expect it still installs the subscriber (first-wins path) and propagates
  no error; the server e2e suite (`crates/server/tests/e2e.rs`) still passes : PRESERVED —
  grep confirms `main.rs:29` is the sole pre-existing caller; with no dispatcher set,
  `try_init` succeeds, the guard is false, and the warn/`Ok(())` tail is identical to the
  pre-change flow. Workspace suite (including the e2e binary) green, 929/929.
- `exporter_fallback_warning` callers: the install path with `enabled = true, exporter = otlp`
  → expect the warning is still emitted after a successful install : PRESERVED — the classifier
  is untouched, the warning is computed before install exactly as before and emitted at
  `telemetry.rs:76-78` on the successful-install path (a first call can only take that path, so
  moving the emission inside the branch changes nothing for existing callers); the classifier
  unit test passes unmodified.

## Residue

- The retained-path debug note is observable only through the host's subscriber; if the validator
  cannot capture debug-level events in the test harness, the note's presence may need a
  `with_max_level(DEBUG)` capture — a harness detail, not an obligation.
- Task 02 depends on this task; its FFI tests re-pin idempotency across `OidcExchange` instances.
  Do not count those as evidence here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric:
NOT_DONE — any load-bearing obligation UNSATISFIED, or a REGRESSION found.
PARTIAL — all obligations SATISFIED except one or more UNVERIFIED, and no regression.
DONE — every obligation SATISFIED, regression PRESERVED, evidence sufficient for each. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with collected evidence — both new test binaries pass in isolation,
the workspace suite is green at 929/929 (baseline 927 + 2), fmt and clippy are clean, and both
named regression paths (main.rs:29 first-install and the otlp fallback warning) are PRESERVED.
Validation note: `SubscriberInitExt::try_init` can also err when `set_global_default` succeeded
but the `tracing-log` `LogTracer::init()` failed (a pre-set `log` logger in the host process);
in that narrow corner the subscriber IS installed yet the retained-path debug note claims
otherwise and the fallback warning is skipped — strictly safer than the old panic, unreachable
for the server binary, and outside this task's DoD; recorded as residue, not an obligation.
