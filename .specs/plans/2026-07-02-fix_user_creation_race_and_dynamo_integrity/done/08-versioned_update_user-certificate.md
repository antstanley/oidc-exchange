# Done Certificate — Task 08: Version-conditional update_user on every backend

**Task:** [08-versioned_update_user.md](08-versioned_update_user.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 08. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 08) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** `update_user` writes conditionally on the `version` it read and increments it on every backend, retrying the read-modify-write on a version conflict up to a named bound, so two racing patches serialize and neither silently overwrites the other.
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not change the patch-field semantics (`Some` replaces, `None` leaves) or the returned `User`; must not break `delete_user`, which routes through `update_user`.

## Obligations

- **O1 — A racing suspend + claims patch ends Suspended on all backends.**
  - *Claim:* two patches read at the same `version` (one setting `Suspended`, one setting claims) serialize; the final state is `Suspended`.
  - *Evidence to collect:* run the racing-patch test for DynamoDB (Local), Postgres, and SQLite — expect the final `status == Suspended` after the retried write.
  - *Checks:* resolve the DynamoDB condition to `version = :read_version OR attribute_not_exists(version)` and the SQL `WHERE id = ? AND version = ?`; confirm each write sets `version = read + 1`.
  - *Status:* ☑ SATISFIED — ran `racing_suspend_and_claims_patch_ends_suspended` on all three backends (DynamoDB Local in Docker, Postgres 16 in Docker, file-backed WAL SQLite with a 4-connection pool): all PASS; each asserts final `status == Suspended`, the concurrent claims patch's `org_id` also applied, and `version == INITIAL_USER_VERSION + 2`. Checks: Dynamo `update_user` (crates/adapters/src/dynamo/mod.rs) puts with `condition_expression("version = :read_version OR attribute_not_exists(version)")` and sets `user.version = read_version + 1`; Postgres runs `UPDATE users SET … version = version + 1 WHERE id = $7 AND version = $8` (RETURNING *, `fetch_optional`); SQLite runs `UPDATE users SET … version = version + 1 WHERE id = ?7 AND version = ?8` with the in-memory `user.version += 1` matching. `retry_on_version_conflict` resolves to the module-local free fn in each adapter (defined in the same module, no import shadowing); `self.update_user` in `delete_user` resolves to the trait impl on the same type — the versioned one.

- **O2 — Negative-space: an unsatisfiable version exhausts the budget and errors.**
  - *Claim:* a read `version` that can never match causes an error after the retry budget, not an unbounded loop or a silent overwrite.
  - *Evidence to collect:* run the test where the row's version keeps advancing — expect `Err` after `UPDATE_MAX_ATTEMPTS`.
  - *Status:* ☑ SATISFIED — `retry_on_version_conflict_errors_when_every_attempt_conflicts` PASSES in all three adapter modules (dynamo, postgres, sqlite): an always-conflicting attempt closure (deterministic stand-in for a row whose version keeps advancing) yields `Err(Error::Conflict)` naming the user and "exhausted", after exactly `UPDATE_MAX_ATTEMPTS` (5) calls — no unbounded loop, no silent success.

- **O3 — Retry bound named; version increments by one.**
  - *Claim:* each adapter's retry bound is a named constant, and a successful write increments `version` by exactly one.
  - *Evidence to collect:* grep each `update_user` for the loop bound — confirm a named constant (e.g. `UPDATE_MAX_ATTEMPTS`); run a single-writer update test asserting `version` goes from N to N+1.
  - *Status:* ☑ SATISFIED — each adapter declares `const UPDATE_MAX_ATTEMPTS: u32 = 5;` with a doc comment and the retry driver loops `for attempt_number in 1..=UPDATE_MAX_ATTEMPTS` — no magic bound. Single-writer +1 increment: the racing tests assert `version == INITIAL_USER_VERSION + 2` after exactly two successful writes (one per write); `update_user_increments_version_each_call` (mock) asserts N→N+1→N+2; `dynamo_repository_crud` and `sqlite_user_crud` update steps PASS.

- **O4 — Mock update_user increments version.**
  - *Claim:* `MockRepository::update_user` increments `version`, matching the durable backends.
  - *Evidence to collect:* read `crates/test-utils/src/lib.rs` `update_user`; run a mock test asserting `version` increments on update.
  - *Status:* ☑ SATISFIED — `MockRepository::update_user` (crates/test-utils/src/lib.rs) now does `user.version += 1;` before returning; `tests::update_user_increments_version_each_call` PASSES, asserting `INITIAL_USER_VERSION` → `+1` → `+2` across two updates.

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; `08-persistence.md` prose updated.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean; confirm the DynamoDB version-conditional-`update_user` sentence is present.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` clean; `cargo clippy --workspace -- -D warnings` clean; `cargo nextest run --workspace` 278/278 PASS (23 skipped = the `#[ignore]` live-backend tests, run separately under O1/O6). `.specs/service/specs/08-persistence.md` §DynamoDB gained the version-conditional-`update_user` paragraph (ConditionExpression, `UPDATE_MAX_ATTEMPTS`, lost-update prose). Note: `kms::tests::test_kms_sign_integration` fails when force-running ignored tests — it needs live AWS KMS credentials; pre-existing environment gap outside this task's surface.

- **O6 — Reviewable: suspend survives a concurrent claims patch on all backends.**
  - *Claim:* a reviewer runs the racing-patch tests on the three backends and observes the suspend surviving.
  - *Evidence to collect:* run the three tests; confirm the final read shows `Suspended` and the concurrent claims patch's fields also applied (via retry), with `version` advanced by two.
  - *Status:* ☑ SATISFIED — exercised as reviewer: started DynamoDB Local and Postgres 16 in Docker and ran `racing_suspend_and_claims_patch_ends_suspended` on dynamo (`--run-ignored`), postgres (`--run-ignored`), and sqlite (default suite): all three PASS, each asserting the final read is `Suspended`, `claims["org_id"] == "org_racing"` (the racing patch landed via retry), and `version == INITIAL_USER_VERSION + 2`.

## Regression check

- `admin_update_user` / `admin_set_claims` / `delete_user` all call `update_user` — trace a single-writer suspend → expect it still succeeds and returns the updated `User` : ☑ PRESERVED — `crates/core/src/service/user_admin.rs` routes all admin ops through `user_repo.update_user` (lines 33/74/120/152/173); `cargo nextest run -p oidc-exchange-core` 86/86 PASS incl. `admin_update_user_partial_patch_reports_changed_fields`; `delete_user` still routes through `self.update_user` in each adapter (dynamo/mod.rs:449, postgres/mod.rs:360, sqlite/mod.rs:425) and CRUD delete steps pass. Patch-field semantics (`Some` replaces, `None` leaves) untouched — the patch-application block is unchanged inside the retry closure.
- Existing `sqlite_user_crud` / `dynamo_repository_crud` update steps → expect still green after the version-conditional change : ☑ PRESERVED — both PASS against live backends; full ignored postgres+dynamo suite (33 tests, excluding the AWS-credentialed KMS test) all PASS.

## Residue

- The DynamoDB delete path becoming a guard-removing transaction is task 09; here `delete_user` still routes through `update_user` and must keep working as a plain versioned write.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with evidence run against live DynamoDB Local, Postgres 16, and file-backed SQLite — racing suspend+claims serializes to Suspended with both patches applied and version +2, the retry budget errors deterministically at UPDATE_MAX_ATTEMPTS on every backend, the mock increments version, fmt/clippy/nextest (278/278) are clean, and all named downstream callers (admin ops, delete_user, CRUD suites) are PRESERVED.
