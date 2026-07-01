# Done Certificate — Task 09: Deletion frees the external id for re-registration

**Task:** [09-deletion_frees_id.md](09-deletion_frees_id.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 09. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 09) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** After a delete, `(provider, external_id)` is free: `get_user_by_external_id` returns nothing for it and a later first login re-registers as a brand-new user — on DynamoDB (guard removed) and SQL (partial unique index excludes deleted rows; lookup filters them).
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not weaken uniqueness among live users; must not carry claims/sessions from the deleted user into the re-registered one; must not break the existing soft-delete (row/record retained) behaviour.

## Obligations

- **O1 — Delete then re-register yields a fresh user on all three backends.**
  - *Claim:* create → delete → `create_user` for the same identity succeeds as a new user (fresh `id`, empty claims), and `get_user_by_external_id` returns nothing for the deleted identity.
  - *Evidence to collect:* run the delete-then-re-register integration test on DynamoDB (Local), Postgres, and SQLite — expect a new `id`, empty claims, and `None` from the external-id lookup for the deleted identity.
  - *Checks:* resolve the SQL lookup filter to `status != 'deleted'` at `postgres/mod.rs:178` and `sqlite/mod.rs:195`; resolve the DynamoDB delete to a `TransactWriteItems` including a `Delete` of the guard key.
  - *Status:* ☐ unverified

- **O2 — Negative-space: a live duplicate still conflicts.**
  - *Claim:* uniqueness still holds among non-deleted users — a duplicate live `(provider, external_id)` conflicts.
  - *Evidence to collect:* run the test creating two live users with the same identity — expect the second `Conflict` (SQL partial index / DynamoDB guard) on all backends.
  - *Status:* ☐ unverified

- **O3 — The DynamoDB delete is a single transaction (status write + guard delete).**
  - *Claim:* setting `status = Deleted` writes the versioned status and deletes the guard atomically (both or neither).
  - *Evidence to collect:* run the Dynamo Local test asserting, after delete, the profile item shows `Deleted` and the guard item is absent; confirm the code path uses one `TransactWriteItems`.
  - *Checks:* confirm the non-delete `update_user` path remains a plain versioned write (no spurious guard delete).
  - *Status:* ☐ unverified

- **O4 — The SQL partial-index migration is idempotent for fresh and existing DBs.**
  - *Claim:* fresh databases get the partial index via inline DDL; existing ones via an explicit `DROP INDEX idx_users_external_id_provider` + partial recreate.
  - *Evidence to collect:* read the Postgres/SQLite `MIGRATIONS` DDL for the partial `WHERE status != 'deleted'` index and the explicit drop/recreate step; run the migration twice (fresh + re-run) — expect no error.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; `08-persistence.md` and `01-domain-model.md` prose reflect the freed identity.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean; confirm the §DynamoDB guard-delete and §PostgreSQL/SQLite partial-index + deleted-exclusion sentences are present.
  - *Status:* ☐ unverified

- **O6 — Reviewable: fresh user on re-register, no lookup hit for the deleted identity.**
  - *Claim:* a reviewer runs the delete-then-re-register test on all three backends and observes a fresh user with no carried-over claims/sessions and no lookup hit for the deleted identity.
  - *Evidence to collect:* run the tests; inspect the re-registered user (new `id`, empty claims) and the external-id lookup for the deleted identity (`None`).
  - *Status:* ☐ unverified

## Regression check

- `admin_delete_user` (patch `status=Deleted`, revoke-all, notify) — trace a delete → expect the record still retained with `status == Deleted` and sessions revoked, plus the guard/partial-index freeing now applied : ☐ (PRESERVED / REGRESSION)
- `get_user_by_id` on a deleted user — trace → expect it STILL returns the deleted record (only `get_user_by_external_id` excludes deleted) : ☐ (PRESERVED / REGRESSION)

## Residue

- Dedup of any pre-existing duplicate `(provider, external_id)` rows is a manual migration outside this task (per the change spec's assumptions); a validator need not verify it here.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
