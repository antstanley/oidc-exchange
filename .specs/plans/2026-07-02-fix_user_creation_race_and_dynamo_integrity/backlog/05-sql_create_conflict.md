# Task 05 — SQL create_user conflict mapping

**Plan:** [plan.md](../plan.md) · **Certificate:** [05-sql_create_conflict-certificate.md](05-sql_create_conflict-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §PostgreSQL/SQLite (a unique violation on insert maps to `Error::Conflict`)
**Depends on:** 01
**Produces:** A `create_user` insert that violates the `(external_id, provider)` unique index on Postgres or SQLite returns `Error::Conflict`, not `Error::StoreError`, so the exchange flow and callers can distinguish "already registered" from an infrastructure failure.
**Pointers:** `crates/adapters/src/postgres/mod.rs:192-221` (`create_user`; unique-violation code `23505`) · `crates/adapters/src/sqlite/mod.rs:211-251` (`create_user`; unique-violation code `2067`) · the adapters' `store_err` helpers

## Steps

- [ ] In the Postgres `create_user`, inspect the `sqlx::Error` for a database error whose code is `23505` (unique violation) and return `Error::Conflict { detail }`; otherwise map as `StoreError` as before.
- [ ] In the SQLite `create_user`, inspect the `sqlx::Error` for the extended result code `2067` (`SQLITE_CONSTRAINT_UNIQUE`) and return `Error::Conflict { detail }`; otherwise `StoreError`.
- [ ] Factor the unique-violation detection into a small helper per adapter (or a shared classifier) so the two `create_user` paths and any future insert path use the same rule.
- [ ] Update the `08-persistence.md` §PostgreSQL/SQLite sentence noting the unique-violation → `Conflict` mapping.

## Definition of done

- [ ] An integration test inserts a user, then inserts a second user with the same `(external_id, provider)`, and asserts the second returns `Error::Conflict` — on Postgres and on SQLite.
- [ ] Negative-space test: a non-unique-violation insert failure (e.g. a NOT NULL / type error) still maps to `Error::StoreError`, not `Conflict`.
- [ ] The unique-violation classifier is exercised by name from `create_user` and asserts on the driver's structured code, not a substring of the message.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the duplicate-insert tests on both SQL backends and observes `Conflict` on the second insert.

## Open questions

- Whether the in-memory SQLite test uses the same `create_pool` migration path as production so the unique index is present — confirm the test harness runs `MIGRATIONS` before the duplicate insert.
