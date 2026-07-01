# Done Certificate — Task 08: Version-conditional update_user on every backend

**Task:** [08-versioned_update_user.md](08-versioned_update_user.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — Negative-space: an unsatisfiable version exhausts the budget and errors.**
  - *Claim:* a read `version` that can never match causes an error after the retry budget, not an unbounded loop or a silent overwrite.
  - *Evidence to collect:* run the test where the row's version keeps advancing — expect `Err` after `UPDATE_MAX_ATTEMPTS`.
  - *Status:* ☐ unverified

- **O3 — Retry bound named; version increments by one.**
  - *Claim:* each adapter's retry bound is a named constant, and a successful write increments `version` by exactly one.
  - *Evidence to collect:* grep each `update_user` for the loop bound — confirm a named constant (e.g. `UPDATE_MAX_ATTEMPTS`); run a single-writer update test asserting `version` goes from N to N+1.
  - *Status:* ☐ unverified

- **O4 — Mock update_user increments version.**
  - *Claim:* `MockRepository::update_user` increments `version`, matching the durable backends.
  - *Evidence to collect:* read `crates/test-utils/src/lib.rs` `update_user`; run a mock test asserting `version` increments on update.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; `08-persistence.md` prose updated.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean; confirm the DynamoDB version-conditional-`update_user` sentence is present.
  - *Status:* ☐ unverified

- **O6 — Reviewable: suspend survives a concurrent claims patch on all backends.**
  - *Claim:* a reviewer runs the racing-patch tests on the three backends and observes the suspend surviving.
  - *Evidence to collect:* run the three tests; confirm the final read shows `Suspended` and the concurrent claims patch's fields also applied (via retry), with `version` advanced by two.
  - *Status:* ☐ unverified

## Regression check

- `admin_update_user` / `admin_set_claims` / `delete_user` all call `update_user` — trace a single-writer suspend → expect it still succeeds and returns the updated `User` : ☐ (PRESERVED / REGRESSION)
- Existing `sqlite_user_crud` / `dynamo_repository_crud` update steps → expect still green after the version-conditional change : ☐ (PRESERVED / REGRESSION)

## Residue

- The DynamoDB delete path becoming a guard-removing transaction is task 09; here `delete_user` still routes through `update_user` and must keep working as a plain versioned write.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
