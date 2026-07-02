# Done Certificate — Task 03: Overlay and env overrides

**Task:** [03-overlay_and_env_overrides.md](03-overlay_and_env_overrides.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** `load_config` deep-merges the env-specific TOML over the default (tables recurse, scalars/arrays replace) and applies `OIDC_EXCHANGE__…` env overrides down to nested and map-valued keys.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the `Reviewable:` item.
- **P3 — Invariants.** The existing fallbacks in `load_config` (`crates/server/src/bootstrap.rs:26-47`) — missing default file, missing/empty env file, no files at all → `AppConfig::default()` — still hold.

## Obligations

- **O1 — Overlay merges, does not wholesale-replace.**
  - *Claim:* a value present only in `config/default.toml` survives when an env overlay sets a *different* key; a key set in both takes the env value.
  - *Evidence to collect:* read the rewritten `load_config`; confirm the env TOML is layered *over* the default (not the `if env_config.is_empty() { default } else { env }` replacement it has today). Run the overlay test — expect the default-only key present and the shared key at the env value.
  - *Checks:* confirm the merge recurses into tables rather than replacing a whole section when only one key changes.
  - *Status:* ☑ SATISFIED — `load_config` now delegates to `load_config_from_dir` (`crates/server/src/bootstrap.rs:46-90`), which layers `File(default)` → `File({env}.toml)` → `Environment` via the `config` crate builder; the old `if env_config.is_empty() { default } else { env }` replacement is gone. Test `load_config_tests::overlay_merges_over_default_rather_than_replacing_it` PASSED: overlay sets only `server.port`; `server.host = "0.0.0.0"` and `registration.mode = "open"` survive from default while `server.port = 9090` takes the overlay value — proving table-recursive merge, not section replacement.

- **O2 — Env overrides reach nested and map-valued paths.**
  - *Claim:* `OIDC_EXCHANGE__PROVIDERS__GOOGLE__CLIENT_ID` sets `providers.google.client_id` and `OIDC_EXCHANGE__SERVER__PORT` sets `server.port`.
  - *Evidence to collect:* read the `Environment` source setup (prefix `OIDC_EXCHANGE`, separator `__`); run the override tests with those vars exported and assert the resulting `AppConfig` fields.
  - *Checks:* confirm segments are lowercased and `__` is the separator so a nested provider key resolves.
  - *Status:* ☑ SATISFIED — `Environment::with_prefix("OIDC_EXCHANGE").separator("__").try_parsing(true)` at `bootstrap.rs:83-86` (constants at lines 27-40). Test `env_var_override_reaches_nested_and_map_valued_paths` PASSED: `OIDC_EXCHANGE__SERVER__PORT=9999` lands on `server.port` (parsed to int) and `OIDC_EXCHANGE__PROVIDERS__GOOGLE__CLIENT_ID` lands on `providers.google.client_id` in the map-valued `providers` section — segments lowercased, unrelated provider fields preserved.

- **O3 — Missing-files fallback and single-underscore addressability.**
  - *Claim:* missing files fall back to defaults without error; a single-underscore segment (`my_idp`) is addressed as one segment, not split.
  - *Evidence to collect:* run the missing-files test (expect `AppConfig::default()`, no error) and the `my_idp` test (an `OIDC_EXCHANGE__PROVIDERS__MY_IDP__…` override lands on `providers.my_idp`).
  - *Status:* ☑ SATISFIED — tests `missing_config_files_fall_back_to_compiled_in_defaults` (empty dir → `AppConfig::default()` fields, no error), `missing_or_empty_env_overlay_file_is_not_an_error` (both empty and nonexistent overlay files), and `single_underscore_provider_name_is_addressable_not_split` (`OIDC_EXCHANGE__PROVIDERS__MY_IDP__CLIENT_ID` yields exactly one provider `my_idp` with the overridden value) all PASSED. Both `File` sources are `required(false)`.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all -- --check` exit 0; `cargo clippy --workspace -- -D warnings` clean (also clean with `--all-targets`, covering the new test module); `cargo nextest run --workspace` → 172 passed, 0 failed, 2 skipped.

- **O5 — Reviewable: merged config carries both default and overlaid/overridden values.**
  - *Claim:* with a default TOML, an env overlay, and an `OIDC_EXCHANGE__…` var exported, the loaded `AppConfig` shows the un-overridden default value plus the overlaid and overridden values.
  - *Evidence to collect:* run the integration-style test (or a manual `load_config` invocation under a temp config dir) and assert all three value sources are reflected.
  - *Status:* ☑ SATISFIED — test `default_overlay_and_env_var_all_apply_together` PASSED: with a default TOML, a `full-stack` overlay TOML, `OIDC_EXCHANGE_ENV=full-stack`, and `OIDC_EXCHANGE__SERVER__HOST=203.0.113.5` exported, the loaded `AppConfig` carries the un-overridden default (`registration.mode = "open"`), the overlaid value (`server.port = 9090`), and the env-overridden value (`server.host = "203.0.113.5"`) simultaneously.

## Regression check

- Callers of `load_config` (the server `main`/bootstrap entry) still receive a valid `AppConfig` when no `OIDC_EXCHANGE_ENV` and no override vars are set → expect the committed default config : ☑ PRESERVED — sole caller is `crates/server/src/main.rs:14`; ran the binary from the repo root with no env vars set → "configuration loaded" logged from the committed `config/default.toml`. (The subsequent `repository.adapter is not configured` exit comes from the later `build_app_state` step, untouched by this diff and pre-existing — the committed default.toml has no `[repository]` section.)

## Residue

- Whether the `config` crate `0.15` deep-merges tables as required or needs a `toml::Value` recursive merge is an Open question in the task; the choice is a note for the validator, not a separate obligation.
- **Validator note (open question resolved):** the implementation uses the `config` crate's layered builder directly (no manual `toml::Value` merge); the overlay test proves its file layering deep-merges tables recursively (scalar in one section replaced, sibling keys and other sections preserved), so no recursive-merge shim was needed. Tests use a per-test `EnvVarGuard` and process-isolated nextest execution, so process-global env mutation does not cross-contaminate.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence — the layered `config`-crate builder replaces the old wholesale file swap, all six new `load_config_tests` pass (overlay merge, nested/map-valued env overrides, missing-file fallbacks, `my_idp` addressability, three-layer reviewable case), fmt/clippy/nextest are clean (172/172), and the sole caller `main.rs:14` is PRESERVED against the committed default config.
