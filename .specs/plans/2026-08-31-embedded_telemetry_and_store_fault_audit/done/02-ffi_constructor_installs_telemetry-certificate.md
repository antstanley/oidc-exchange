# Done Certificate — Task 02: FFI constructor installs the telemetry subscriber

**Task:** [02-ffi_constructor_installs_telemetry.md](02-ffi_constructor_installs_telemetry.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-08-31

> Verification protocol for Task 02. A validating agent discharges it: collect each obligation's
> evidence, run its checks, set the Status, then derive the Conclusion by the rubric. Do not mark
> an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Constructing `OidcExchange` installs the process-wide `tracing` subscriber via
  the server crate's `init_telemetry`, so embedded deployments get JSON diagnostics on the host's
  stdout under `RUST_LOG` control; a second instance constructs without panicking, and a
  host-owned subscriber is retained and receives the FFI's boundary diagnostics.
- **P2 — Obligations.** Done iff O1…O6 all hold, one per definition-of-done item in DoD order;
  O6 is the Reviewable item.
- **P3 — Invariants.** Must not break: the constructor's existing error taxonomy (config-parse
  failures reach the host as `CONFIG_ERROR` before any install; service-build failures as
  `SERVICE_ERROR`); the in-crate unit tests that construct via `with_router_for_test`
  (`crates/ffi/src/lib.rs:335`) and their scoped `set_default` captures; the `embedder_tests`
  and `crates/ffi/tests/integration.rs` suites; and the FFI's dependency surface
  (`tracing-subscriber` stays dev-only). Task 01 (idempotent `try_init`) is a precondition —
  confirm it is in `done/` or its behaviour present before discharging.

## Obligations

- **O1 — `telemetry_install.rs` proves construction installs the subscriber and a second construction serves `/health`.**
  - *Claim:* constructing an instance over the minimal admin-role SQLite config leaves
    `tracing::dispatcher::has_been_set()` true, and constructing a second instance neither
    panics nor fails — it serves `/health`.
  - *Evidence to collect:* read `crates/ffi/src/lib.rs::new_with_base_path` and confirm the
    `oidc_exchange::telemetry::init_telemetry(&config.telemetry)` call sits after the base-path
    override (post line 126 pre-change) and before `tokio::runtime::Runtime::new`/`build_service`.
    Read `crates/ffi/tests/telemetry_install.rs`; confirm it asserts `has_been_set()` after the
    first construction and drives `/health` (status 200) through a second instance. Run the binary
    via `cargo nextest run -p oidc-exchange-ffi` — expect PASS. Confirm the file is its own
    integration binary (per-process global dispatcher), not folded into `integration.rs`.
  - *Checks:* resolve `init_telemetry` at the new call site — confirm it is
    `oidc_exchange::telemetry::init_telemetry` (the server crate's, `crates/server/src/lib.rs:9`
    `pub mod telemetry`), not a new FFI-local function; a second install path is exactly the
    divergence the change spec's Decisions reject. Resolve `has_been_set` to
    `tracing::dispatcher::has_been_set`.
  - *Status:* SATISFIED — call at `crates/ffi/src/lib.rs:139-142`, after the base-path override
    (block ends line 126) and before `NormalisationLimits` (143) / `Runtime::new` (146) /
    `build_service` (153). `telemetry_install.rs` is its own binary, asserts `!has_been_set()`
    as a precondition, `has_been_set()` after first construction, and `/health` 200 on both
    instances. `cargo nextest run -p oidc-exchange-ffi` → 16/16 PASS (both new binaries PASS).
    `init_telemetry` resolves via the `oidc-exchange` path dep (`crates/ffi/Cargo.toml:11`) to
    `crates/server/src/telemetry.rs:39` (`try_init`-idempotent); no FFI-local `init_telemetry`
    exists; `has_been_set` resolves to `tracing::dispatcher::has_been_set`. No shadowing.

- **O2 — `telemetry_host_respect.rs` proves a pre-installed host subscriber survives and captures the boundary warning.**
  - *Claim:* with a capturing subscriber installed via `tracing::subscriber::set_global_default`
    before construction, construction succeeds, a request carrying an invalid header name is
    answered, and the host's subscriber captured the `invalid request headers dropped at FFI
    boundary` warning (`crates/ffi/src/lib.rs:311-315` pre-change).
  - *Evidence to collect:* read `crates/ffi/tests/telemetry_host_respect.rs`; confirm the
    set-global-default-before-construction ordering, the successful construction assertion, the
    request with an invalid header name, and the capture assertion on the deterministic warning
    text. Run the binary — expect PASS.
  - *Checks:* trace the scenario: host sets global capture → `OidcExchange::new` → `init_telemetry`
    → `try_init` errs → `Ok` (host retained) → request with invalid header → warning emitted →
    host capture contains it. The trace fails if construction returned `SERVICE_ERROR` (would mean
    the already-set case was not mapped to `Ok` — a Task 01 regression surfacing here).
  - *Status:* SATISFIED — `telemetry_host_respect.rs` sets `tracing::subscriber::set_global_default`
    (registry + capturing layer) before construction, asserts construction succeeds, sends a
    `/health` request via `handle_blocking` carrying header `"invalid header name"` (illegal
    space), asserts status 200 and that the capture contains `invalid request headers dropped
    at FFI boundary` (emitted at `crates/ffi/src/lib.rs:327-332`). Ran the binary → PASS.
    The trace holds end to end: construction returned `Ok`, so the already-set case mapped to
    `Ok(())` (`crates/server/src/telemetry.rs:54-72`) — no Task 01 regression.

- **O3 — No FFI dependency change; no binding change.**
  - *Claim:* `crates/ffi/Cargo.toml` `[dependencies]` is unchanged (`tracing-subscriber` remains
    under `[dev-dependencies]`), and no file under `bindings/` is touched.
  - *Evidence to collect:* diff `crates/ffi/Cargo.toml` — expect `[dependencies]` identical
    (server crate at line 11 already present) and `tracing-subscriber = "0.3"` still dev-only;
    run the VCS diff over `bindings/` — expect empty (built artifacts aside, nothing authored).
  - *Status:* SATISFIED — `jj diff -r @ --summary` lists exactly `M crates/ffi/src/lib.rs`,
    `A crates/ffi/tests/telemetry_host_respect.rs`, `A crates/ffi/tests/telemetry_install.rs`.
    `crates/ffi/Cargo.toml` is untouched: `[dependencies]` identical (`oidc-exchange` at
    line 11), `tracing-subscriber = "0.3"` still under `[dev-dependencies]` (line 29);
    nothing under `bindings/` in the diff.

- **O4 — Existing FFI suites pass unmodified; config-parse failures still surface as `CONFIG_ERROR`.**
  - *Claim:* the in-crate unit tests, `embedder_tests`, and `crates/ffi/tests/integration.rs`
    pass without edits, and an unparseable config still returns
    `FfiError { code: "CONFIG_ERROR" }` — the install sits after parsing, so the error value
    itself still crosses the boundary.
  - *Evidence to collect:* diff the pre-existing FFI test files — expect no behavioural edits;
    run `cargo nextest run -p oidc-exchange-ffi` — expect the full crate suite green. Locate the
    existing invalid-config test (or, if none pins the code, trace `new_with_base_path` with a
    garbage TOML string: `parse_config` errs → `CONFIG_ERROR` returned before `init_telemetry`
    is reached).
  - *Checks:* execution trace for placement: garbage TOML → `parse_config` `Err` → early return
    `CONFIG_ERROR`, `init_telemetry` never called — confirm by reading statement order in
    `new_with_base_path`.
  - *Status:* SATISFIED — no pre-existing test file appears in the diff (only `lib.rs` and the
    two new binaries); `cargo nextest run -p oidc-exchange-ffi` → 16 passed / 0 failed
    (in-crate unit tests incl. `embedder_tests`, `integration.rs`, and the two new binaries).
    `CONFIG_ERROR` is pinned by existing tests at `crates/ffi/tests/integration.rs:136,157,184`
    (invalid role via string, base-path, and `from_file`), all passing. Placement trace:
    `parse_config` `Err` → early return `CONFIG_ERROR` at `lib.rs:107-110` (and invalid
    base-path at 117-121), both before the `init_telemetry` call at line 139.

- **O5 — Meets the repo definition of done.**
  - *Claim:* format, lint, and the workspace test suite pass.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done, run
    `cargo fmt --check --all`, `cargo clippy --workspace -- -D warnings`, and
    `cargo nextest run --workspace` — expect all clean/green.
  - *Status:* SATISFIED — `cargo fmt --check --all` clean; `cargo clippy --workspace --
    -D warnings` finished with no warnings; `cargo nextest run --workspace` → 931 passed /
    0 failed / 78 skipped (baseline 929 + the 2 new tests).

- **O6 — Reviewable: run the two new FFI test binaries and observe both scenarios pass (Reviewable).**
  - *Claim:* a reviewer can run `telemetry_install` and `telemetry_host_respect` and observe both
    pass; optionally, an embedded example run with `RUST_LOG=info` shows JSON tracing lines on
    stdout.
  - *Evidence to collect:* run the two binaries by name (e.g.
    `cargo nextest run -p oidc-exchange-ffi -E 'binary(telemetry_install) or binary(telemetry_host_respect)'`)
    — expect both PASS. The `RUST_LOG` demonstration is optional corroboration, not required
    evidence.
  - *Status:* SATISFIED — ran `cargo nextest run -p oidc-exchange-ffi -E 'binary(telemetry_install)
    or binary(telemetry_host_respect)'` → 2 tests run: 2 passed, 0 skipped (both binaries PASS).
    The optional `RUST_LOG` demonstration was not exercised (not required evidence).

## Regression check

- `bindings/nodejs/src/lib.rs:78-100` (the napi constructor) calls
  `OidcExchange::new`/`new_with_base_path` with a valid config → expect construction still
  succeeds with the same signature; no binding code changed : PRESERVED — `new_with_base_path`'s
  signature and error/success values are unchanged (the install is an added internal side effect);
  the diff touches nothing under `bindings/`.
- The deprecated `handle_request` path (exercised by
  `embedder_tests::persistent_embedder_builds_serves_and_drops_with_a_reaper_hosted`, which
  constructs via `OidcExchange::new` — `crates/ffi/src/lib.rs:620-671`) → expect the embedder
  test still passes; its process now carries the global subscriber, which is harmless (JSON
  lines on test stdout) : PRESERVED — `persistent_embedder_builds_serves_and_drops_with_a_reaper_hosted`
  passed unmodified in the crate run.
- `crates/ffi/src/lib.rs::from_file` (`lib.rs:186-192` — delegates to `Self::new`, which now
  installs telemetry, so `from_file` installs by construction; covered by
  `test_invalid_config_rejected_via_from_file` and `test_valid_config_constructs_via_from_file`
  in `crates/ffi/tests/integration.rs:171,200`) → expect both tests still pass
  : PRESERVED — both passed unmodified; the invalid-config path returns `CONFIG_ERROR` before
  the install is reached.
- In-crate unit tests constructing via `with_router_for_test` → expect their scoped
  `set_default` captures still observe their own events (no global install on that path)
  : PRESERVED — `with_router_for_test` (`crates/ffi/src/lib.rs:351`) still bypasses `new` and
  installs nothing; the panic-boundary tests' thread-scoped `set_default` (`lib.rs:579`)
  captures passed in the crate run.

## Residue

- Rust hosts embedding `oidc-exchange-ffi` directly that want their own global subscriber must
  install it before construction (change spec §Compatibility) — a documentation fact, not an
  obligation here.
- Stdout consumers that assumed every line is an audit event now see interleaved tracing lines;
  covered by the change spec's Compatibility notes, out of this task's DoD.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric:
NOT_DONE — any load-bearing obligation UNSATISFIED, or a REGRESSION found.
PARTIAL — all obligations SATISFIED except one or more UNVERIFIED, and no regression.
DONE — every obligation SATISFIED, regression PRESERVED, evidence sufficient for each. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with named evidence (both new per-process binaries PASS, crate
suite 16/16, workspace 931/931 with fmt and clippy clean, no dependency or binding change in
the diff), and all four regression callers PRESERVED — Task 01's idempotent `try_init` was
confirmed present in the base before discharge.
