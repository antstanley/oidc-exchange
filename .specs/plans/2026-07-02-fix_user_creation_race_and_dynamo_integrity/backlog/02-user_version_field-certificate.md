# Done Certificate — Task 02: Store-managed `version` field on User

**Task:** [02-user_version_field.md](02-user_version_field.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** `User` carries a store-managed integer `version` that `create_user` writes as `1` and every backend persists and round-trips, with a missing value reading as the migration default `1`.
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not add `version` to `NewUser` or `UserPatch`; must not break the existing user round-trip tests in `schema.rs`, `postgres`, `sqlite`, or the mock, nor change existing non-`version` fields.

## Obligations

- **O1 — create_user returns version == 1 and it round-trips per backend.**
  - *Claim:* on DynamoDB, Postgres, and SQLite, `create_user` yields `version == 1` and a re-read returns `1`.
  - *Evidence to collect:* run the per-backend create/round-trip tests (`schema.rs` user round-trip; `sqlite` `sqlite_user_crud`; the Postgres/Dynamo Local integration create test) — expect `version == 1` after create and after re-read.
  - *Checks:* resolve `version` reads to `item_to_user`/`row_to_user` in each adapter, not a shadowed local.
  - *Status:* ☐ unverified

- **O2 — A pre-existing record with no version reads as 1.**
  - *Claim:* a DynamoDB item lacking the `version` attribute and a SQL row via `DEFAULT 1` both yield `version == 1`.
  - *Evidence to collect:* run a `schema.rs` test that builds an item without `version` and asserts `item_to_user` returns `1`; inspect the SQL `CREATE TABLE` / `ALTER TABLE` DDL for `version BIGINT NOT NULL DEFAULT 1`. Expect PASS / DDL present.
  - *Status:* ☐ unverified

- **O3 — Both schemas and the domain page list version.**
  - *Claim:* `version` is in `properties` and `required` of `User` in the service `canonical-types.schema.json` and `datamodel.schema.json`, and the `01-domain-model.md` struct listing matches.
  - *Evidence to collect:* read `$defs.User` in `.specs/service/specs/canonical-types.schema.json` and `definitions.User` in `schemas/datamodel.schema.json`; confirm `version` in both `properties` and `required`. Read the `01-domain-model.md` User struct — confirm the `version: u64` line and store-managed prose.
  - *Status:* ☐ unverified

- **O4 — Every User constructor sets version; the workspace builds.**
  - *Claim:* no `User { … }` literal omits `version`.
  - *Evidence to collect:* `grep -rn "User {" crates bindings` and confirm each construction sets `version`; run `cargo build --workspace` — expect no missing-field error.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; schema + prose updated together.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean.
  - *Status:* ☐ unverified

- **O6 — Reviewable: round-trip and schema validation show version == 1 and required.**
  - *Claim:* a reviewer runs the per-backend round-trip tests and schema validation and observes `version == 1` on create and `version` required.
  - *Evidence to collect:* run the round-trip tests for the three backends; validate a `User` missing `version` against the updated schemas — expect INVALID (required), and a create-produced `User` — expect valid with `version: 1`.
  - *Status:* ☐ unverified

## Regression check

- `item_to_user`/`row_to_user` are called by `get_user_by_id`, `get_user_by_external_id`, `list_users`; trace `get_user_by_id` after the field addition → expect the returned `User` populated with all prior fields plus `version` : ☐ (PRESERVED / REGRESSION)
- `MockRepository::create_user` callers in `crates/core/tests/*` → expect they still compile and return a `User` : ☐ (PRESERVED / REGRESSION)

## Residue

- The Deleted freed-identity prose added to `01-domain-model.md` here is realized by task 09; a validator should verify only that the prose exists, not the deletion behaviour.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
