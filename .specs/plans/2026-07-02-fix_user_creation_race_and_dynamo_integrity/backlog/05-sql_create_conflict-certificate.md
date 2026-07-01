# Done Certificate — Task 05: SQL create_user conflict mapping

**Task:** [05-sql_create_conflict.md](05-sql_create_conflict.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 05. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 05) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** A `create_user` insert that violates the `(external_id, provider)` unique index on Postgres (`23505`) or SQLite (`2067`) returns `Error::Conflict`, not `Error::StoreError`.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not change the success path of `create_user` (id minting, returned `User`), nor reclassify non-unique insert failures.

## Obligations

- **O1 — A duplicate insert returns Conflict on both SQL backends.**
  - *Claim:* inserting a second user with the same `(external_id, provider)` returns `Error::Conflict` on Postgres and SQLite.
  - *Evidence to collect:* run the duplicate-insert integration test for each backend — expect the second `create_user` yields `Err(Error::Conflict { .. })`.
  - *Checks:* resolve the error classification to the per-adapter unique-violation helper, and confirm it reads the driver's structured code (`23505` / `2067`), not a message substring.
  - *Status:* ☐ unverified

- **O2 — Negative-space: a non-unique failure stays StoreError.**
  - *Claim:* a NOT NULL / type / other insert failure maps to `Error::StoreError`, not `Conflict`.
  - *Evidence to collect:* run the test injecting a non-unique-violation insert error — expect `StoreError`.
  - *Status:* ☐ unverified

- **O3 — The classifier asserts on the structured code.**
  - *Claim:* the unique-violation detection uses the driver's error code, not string matching.
  - *Evidence to collect:* read the per-adapter classifier; confirm it inspects the `sqlx` DB error code/kind (`.code()` / constraint kind), and is called from `create_user`.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; `08-persistence.md` prose updated.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean; confirm the PostgreSQL/SQLite unique-violation → `Conflict` sentence is present.
  - *Status:* ☐ unverified

- **O5 — Reviewable: duplicate insert yields Conflict on both backends.**
  - *Claim:* a reviewer runs the duplicate-insert tests on Postgres and SQLite and observes `Conflict` on the second insert.
  - *Evidence to collect:* run both tests; confirm the SQLite in-memory harness ran `MIGRATIONS` (so the unique index exists) before the duplicate insert.
  - *Status:* ☐ unverified

## Regression check

- `create_user` success path callers (`admin_create_user`, exchange JIT create) — trace a first insert → expect the returned `User` unchanged (id, fields) : ☐ (PRESERVED / REGRESSION)
- Existing `sqlite_user_crud` test — expect still green after the error-mapping change : ☐ (PRESERVED / REGRESSION)

## Residue

- Whether the SQLite test harness path (`:memory:`, split-statement migrations) creates the unique index is called out as an open question in the task; a validator should confirm it before trusting the duplicate-insert test.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
