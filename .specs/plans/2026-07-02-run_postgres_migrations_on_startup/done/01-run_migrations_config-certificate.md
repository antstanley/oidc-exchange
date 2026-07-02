# Done Certificate — Task 01: run_migrations config

**Task:** [01-run_migrations_config.md](01-run_migrations_config.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `[repository.postgres] run_migrations` deserializes into `PostgresConfig.run_migrations: Option<bool>` (present → `Some(_)`, absent → `None`), and 06-configuration.md documents the key and its `true` default.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break deserialization of existing `[repository.postgres] { url, max_connections? }` config, nor the other `RepositoryConfig` fields in `crates/core/src/config.rs`.

## Obligations

- **O1 — `PostgresConfig` carries `run_migrations: Option<bool>` and deserializes correctly.**
  - *Claim:* a `[repository.postgres]` TOML block with `run_migrations = true|false` yields `Some(true|false)`; without the key yields `None`.
  - *Evidence to collect:* read `crates/core/src/config.rs` around `PostgresConfig` (was lines 148-152); confirm the `run_migrations: Option<bool>` field is present after `max_connections`. Run the new config-deserialization test — expect PASS on all three cases (`Some(true)`, `Some(false)`, `None`).
  - *Checks:* confirm the field type is `Option<bool>` (not `bool`), so an absent key deserializes rather than erroring; confirm `url` remains a required field.
  - *Status:* SATISFIED — `crates/core/src/config.rs:246-254`: `PostgresConfig` carries `pub run_migrations: Option<bool>` immediately after `max_connections`; `url: String` remains required (no `#[serde(default)]` on the struct). Ran `cargo nextest run -p oidc-exchange-core postgres_run_migrations_deserializes_present_and_absent` → PASS, asserting `Some(false)`, `Some(true)`, and `None` for the three cases.

- **O2 — Negative-space test: `run_migrations` omitted still deserializes to `None`.**
  - *Claim:* a `[repository.postgres]` block that omits `run_migrations` deserializes successfully with the field `None`, not a parse error.
  - *Evidence to collect:* run the config-deserialization test's "absent key" case; confirm it constructs a `PostgresConfig` and asserts `run_migrations.is_none()` (not a `Result::Err`).
  - *Status:* SATISFIED — test's third case (`crates/core/src/config.rs:573-583`) parses a `[repository.postgres]` block with only `url`, uses `.expect("omitting run_migrations must still deserialize")` (so a parse error fails the test) and asserts `run_migrations == None`. Test PASSED.

- **O3 — 06-configuration.md documents the key and default.**
  - *Claim:* `06-configuration.md` §`[repository]` lists `run_migrations?` on `[repository.postgres]` and states the `true` default; the page `**Date:**` is bumped.
  - *Evidence to collect:* read `.specs/service/specs/06-configuration.md` §`[repository]` (around line 67-71); confirm `run_migrations?` appears in the `[repository.postgres]` keys and the `true` default / locked-down note is present; confirm the header `**Date:**` differs from `2026-06-29`.
  - *Status:* SATISFIED — `.specs/service/specs/06-configuration.md:70-73` lists `[repository.postgres] { url, max_connections?, run_migrations? }` and states "`run_migrations` defaults to `true`; set it to `false` for locked-down databases where the app role has no DDL rights and migrations are applied out-of-band." Header `**Date:**` bumped `2026-06-29` → `2026-07-02`.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and lint clean, no unnamed limits introduced.
  - *Evidence to collect:* from `.specs/development-guidelines.md` §Definition of done, run `cargo fmt --check` (or `cargo fmt` then confirm no diff), `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean.
  - *Status:* SATISFIED — `cargo fmt --check` exit 0; `cargo clippy --workspace -- -D warnings` finished clean; `cargo nextest run --workspace` → 281 tests run, 281 passed, 27 skipped. No numeric limits introduced (the new field is an `Option<bool>`).

- **O5 — Reviewable: false-key and absent-key deserialize to `Some(false)` and `None` (Reviewable).**
  - *Claim:* a reviewer loads a TOML with `run_migrations = false` and one without the key and observes `Some(false)` and `None` respectively.
  - *Evidence to collect:* run the new test (`cargo nextest run -p oidc-exchange-core <test name>`); observe the assertions on `Some(false)` and `None` passing.
  - *Status:* SATISFIED — exercised `cargo nextest run -p oidc-exchange-core postgres_run_migrations_deserializes_present_and_absent` → PASS; the test loads a TOML with `run_migrations = false` (asserts `Some(false)`) and one without the key (asserts `None`), exactly the reviewer action named.

## Regression check

- `RepositoryConfig` deserialization (`crates/core/src/config.rs`) is used by `bootstrap::load_config`. Trace: an existing `[repository.postgres] { url, max_connections }` TOML (no `run_migrations`) still deserializes to a valid `PostgresConfig` with `run_migrations == None` : PRESERVED — the new optional field cannot make an existing document fail to parse (`Option<bool>` deserializes to `None` when absent; the negative-space test case proves it), and the full workspace suite including all pre-existing config-deserialization tests passed (281/281).

## Residue

- Task 01 makes the field observable at the config layer only; the "absent → migrate" behaviour is realized in tasks 02/04. Not an obligation here.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence (field at config.rs:246-254, new three-case test PASS, spec page updated with Date bump, fmt/clippy/281-test suite clean) and the `bootstrap::load_config` regression surface is PRESERVED.
