# Done Certificate — Task 05: Validation wiring and FFI

**Task:** [05-validation_wiring_and_ffi.md](05-validation_wiring_and_ffi.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Status:* ☐ unverified

- **O2 — FFI construction rejects invalid config; valid config still constructs.**
  - *Claim:* `OidcExchange::new` and `from_file` with an invalid TOML config return an `FfiError` at construction; a valid config constructs successfully.
  - *Evidence to collect:* read `parse_config` (`:50-53`) and confirm `config.validate()?` runs before returning; read `crates/ffi/src/lib.rs:51-72` and confirm the boxed error maps to `FfiError { code: "CONFIG_ERROR" }`. Run the FFI-path tests — expect `Err(FfiError)` for invalid and `Ok` for valid.
  - *Checks:* trace the invalid config through `new` → `parse_config` → `validate` and confirm the error surfaces before `build_service`/`build_router` are reached.
  - *Status:* ☐ unverified

- **O3 — Negative-space tests on both paths; meaningful assertions.**
  - *Claim:* negative tests exist for both the server `load_config` path and the FFI construction path; touched functions/tests carry ≥2 meaningful assertions.
  - *Evidence to collect:* enumerate the new tests; confirm one asserts `load_config`/helper returns `Err` on invalid config and one asserts `OidcExchange::new` returns `FfiError`.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: invalid config rejected at construction/startup; valid passes.**
  - *Claim:* a reviewer runs the FFI construction test and a `load_config` test and sees invalid config rejected at construction/startup and valid config accepted.
  - *Evidence to collect:* run the FFI and server validation tests (e.g. `cargo nextest run -p oidc-exchange-ffi` and the bootstrap test) and observe the `Err`/`Ok` split.
  - *Status:* ☐ unverified

## Regression check

- `OidcExchange::new` / `from_file` with a valid config (`crates/ffi/src/lib.rs:51-77`) still build a runtime and router → expect successful construction : ☐ (PRESERVED / REGRESSION)
- The server bootstrap entry calling `load_config` with a valid config → expect a built `AppConfig`, unchanged : ☐ (PRESERVED / REGRESSION)

## Residue

- None noted at authoring. The router-mount consequences of `internal_api.enabled` are Task 06's obligations, not this task's.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
