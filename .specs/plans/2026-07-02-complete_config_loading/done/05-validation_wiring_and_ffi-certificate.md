# Done Certificate — Task 05: Validation wiring and FFI

**Task:** [05-validation_wiring_and_ffi.md](05-validation_wiring_and_ffi.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 05. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 05) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** `AppConfig::validate()` runs at the tail of `load_config` (after merge, override, and placeholder resolution) and inside `parse_config`, so both a server startup and an FFI `OidcExchange::new`/`from_file` reject invalid config before building anything.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Valid config still loads and constructs on both paths; the FFI error mapping still surfaces config failures as `FfiError { code: "CONFIG_ERROR" }`.

## Obligations

- **O1 — Invalid config fails `load_config` before any build.**
  - *Claim:* an invalid config (bad role, bad TTL, malformed allowlist, or served-with-empty-secret) makes `load_config` return `Err` before an adapter or router is built.
  - *Evidence to collect:* read `load_config` (`crates/server/src/bootstrap.rs:26-47`); confirm `config.validate()?` is the final step, after the Task-04 resolution. Run the server-side invalid-config test — expect `Err`.
  - *Checks:* resolve the `validate()` call to `AppConfig::validate` from `crates/core` (Task 02), not a local stub; confirm it runs on the fully-resolved config, not the pre-merge value.
  - *Status:* ☑ SATISFIED — `config.validate()?` is the final step of `load_config_from_dir` (`crates/server/src/bootstrap.rs:107`), after `builder.build()` (:104), `resolve_placeholders` (:105), and `try_deserialize` (:106); `config` is typed `AppConfig` imported from `oidc_exchange_core::config` (:8), so `.validate()` resolves to the inherent `AppConfig::validate` at `crates/core/src/config.rs:44` — no local stub or trait shadow in scope. Ran `bootstrap::load_config_tests::load_config_rejects_invalid_role_before_building_anything` → PASS (`Err` naming both the field `role` and the value `exchang`).

- **O2 — FFI construction rejects invalid config; valid config still constructs.**
  - *Claim:* `OidcExchange::new` and `from_file` with an invalid TOML config return an `FfiError` at construction; a valid config constructs successfully.
  - *Evidence to collect:* read `parse_config` (`:50-53`) and confirm `config.validate()?` runs before returning; read `crates/ffi/src/lib.rs:51-72` and confirm the boxed error maps to `FfiError { code: "CONFIG_ERROR" }`. Run the FFI-path tests — expect `Err(FfiError)` for invalid and `Ok` for valid.
  - *Checks:* trace the invalid config through `new` → `parse_config` → `validate` and confirm the error surfaces before `build_service`/`build_router` are reached.
  - *Status:* ☑ SATISFIED — `parse_config` calls `config.validate()?` before returning (`crates/server/src/bootstrap.rs:117`); `OidcExchange::new` maps the boxed error to `FfiError { code: "CONFIG_ERROR" }` (`crates/ffi/src/lib.rs:52-55`) and the `?` there exits before `build_service` (:62-63) and `build_router` (:69); `from_file` routes through `new` (:80). Ran `test_invalid_role_rejected_at_construction`, `test_invalid_config_rejected_via_from_file`, `test_valid_config_constructs_successfully` → all PASS (invalid → `Err` with code `CONFIG_ERROR` at construction; valid → `Ok`).

- **O3 — Negative-space tests on both paths; meaningful assertions.**
  - *Claim:* negative tests exist for both the server `load_config` path and the FFI construction path; touched functions/tests carry ≥2 meaningful assertions.
  - *Evidence to collect:* enumerate the new tests; confirm one asserts `load_config`/helper returns `Err` on invalid config and one asserts `OidcExchange::new` returns `FfiError`.
  - *Status:* ☑ SATISFIED — seven new tests: server side `load_config_rejects_invalid_role_before_building_anything` (asserts `Err`, message names field and value — 2 assertions plus `expect_err`), `load_config_accepts_valid_config` (2 field assertions), `parse_config_rejects_invalid_role` (2 assertions), `parse_config_accepts_valid_config`; FFI side `test_invalid_role_rejected_at_construction` (asserts `code == "CONFIG_ERROR"` and message names the field — 2 assertions), `test_invalid_config_rejected_via_from_file`, `test_valid_config_constructs_successfully`. Negative space is covered on both paths with ≥2 meaningful assertions per negative test.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all -- --check` exit 0; `cargo clippy --workspace -- -D warnings` exit 0; `cargo nextest run --workspace` → 185 tests run, 185 passed, 2 skipped.

- **O5 — Reviewable: invalid config rejected at construction/startup; valid passes.**
  - *Claim:* a reviewer runs the FFI construction test and a `load_config` test and sees invalid config rejected at construction/startup and valid config accepted.
  - *Evidence to collect:* run the FFI and server validation tests (e.g. `cargo nextest run -p oidc-exchange-ffi` and the bootstrap test) and observe the `Err`/`Ok` split.
  - *Status:* ☑ SATISFIED — exercised directly: `cargo nextest run -p oidc-exchange -E 'test(load_config_…)|test(parse_config_…)'` → 4/4 PASS and `cargo nextest run -p oidc-exchange-ffi -E '…construction…'` → 3/3 PASS; the invalid configs are rejected (`Err`/`CONFIG_ERROR`) and the valid configs load/construct (`Ok`) on both paths.

## Regression check

- `OidcExchange::new` / `from_file` with a valid config (`crates/ffi/src/lib.rs:51-77`) still build a runtime and router → expect successful construction : ☑ PRESERVED — `test_valid_config_constructs_successfully` PASS, and every pre-existing FFI integration test (health, jwks, discovery, register, token, …) constructs through `setup()` and passed (185/185 workspace-wide).
- The server bootstrap entry calling `load_config` with a valid config → expect a built `AppConfig`, unchanged : ☑ PRESERVED — `load_config_accepts_valid_config` PASS; all pre-existing `load_config_tests` (merge, overlay, env-override, placeholder tests) still pass unchanged.

## Residue

- None noted at authoring. The router-mount consequences of `internal_api.enabled` are Task 06's obligations, not this task's.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence — `validate()` (resolving to `AppConfig::validate` at `crates/core/src/config.rs:44`) runs as the final step of `load_config_from_dir` and inside `parse_config`, the FFI surfaces it as `FfiError { code: "CONFIG_ERROR" }` before `build_service`/`build_router`, all 7 new tests plus the full workspace suite (185/185) pass, fmt and clippy are clean, and both named regression callers are PRESERVED.
