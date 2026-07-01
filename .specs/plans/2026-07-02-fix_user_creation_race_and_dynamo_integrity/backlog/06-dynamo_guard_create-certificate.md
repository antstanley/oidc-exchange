# Done Certificate — Task 06: DynamoDB transactional create_user with uniqueness guard

**Task:** [06-dynamo_guard_create.md](06-dynamo_guard_create.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 06. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 06) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** `create_user` on DynamoDB is a `TransactWriteItems` of the user item plus an `EXT#<provider>#<external_id>` / `UNIQUE` guard item carrying `user_id`, each conditioned on `attribute_not_exists(pk)`; a cancelled transaction surfaces as `Conflict`.
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not change the returned `User` shape (including `version` from task 02); must not break the existing `dynamo_repository_crud` integration test's create step.

## Obligations

- **O1 — Create writes both the profile and the guard item.**
  - *Claim:* after `create_user`, the profile item (`USER#<id>`/`PROFILE`) and the guard item (`EXT#<provider>#<external_id>`/`UNIQUE`, `user_id` = the new id) both exist.
  - *Evidence to collect:* run the Dynamo Local integration test that creates a user then `GetItem`s both keys — expect both present with the correct `user_id`.
  - *Checks:* resolve the guard-item builder to the new `schema.rs` helper; confirm both `Put`s carry `condition_expression("attribute_not_exists(pk)")`.
  - *Status:* ☐ unverified

- **O2 — A duplicate create returns one user and one Conflict.**
  - *Claim:* two `create_user` calls for the same `(provider, external_id)` yield one `Ok(User)` and one `Err(Conflict)`.
  - *Evidence to collect:* run the concurrent/sequential duplicate-create Dynamo Local test — expect exactly one `Conflict` from the cancelled transaction.
  - *Checks:* confirm the `TransactionCanceledException` with a `ConditionalCheckFailed` reason maps to `Error::Conflict`, resolved against the AWS SDK error type.
  - *Status:* ☐ unverified

- **O3 — Negative-space: a non-conditional transaction failure stays StoreError.**
  - *Claim:* a `TransactWriteItems` failure not caused by a conditional-check cancellation maps to `StoreError`.
  - *Evidence to collect:* run/read the test or mapping code covering a non-`ConditionalCheckFailed` cancellation or other SDK error — expect `StoreError`.
  - *Status:* ☐ unverified

- **O4 — The guard item is documented in the design sidecar and prose.**
  - *Claim:* `schemas/dynamodb/table-design.json` has a `UserUniquenessGuard` entry and `08-persistence.md` lists the guard row and transactional-create prose.
  - *Evidence to collect:* read `item_schemas.UserUniquenessGuard` (pk/sk/`user_id`) in the sidecar; read the `08-persistence.md` §DynamoDB item table + transactional-create paragraph.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean.
  - *Status:* ☐ unverified

- **O6 — Reviewable: concurrent create yields one user, one Conflict, and a persisted guard.**
  - *Claim:* a reviewer runs the concurrent-create Dynamo Local test and observes one user, one `Conflict`, and the guard item.
  - *Evidence to collect:* run the test; `GetItem` the guard key and confirm it holds the winning `user_id`.
  - *Status:* ☐ unverified

## Regression check

- `create_user` callers (`admin_create_user`, exchange JIT create) — trace a first create → expect an `Ok(User)` with `version == 1` and all fields, unchanged externally : ☐ (PRESERVED / REGRESSION)
- The existing `dynamo_repository_crud` test's create step → expect still green (or updated to assert the guard) : ☐ (PRESERVED / REGRESSION)

## Residue

- The guard backfill for pre-existing users (script vs lazy write) is an open question in the task and a precondition for task 07; not itself an obligation of Task 06 beyond providing the mechanism.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
