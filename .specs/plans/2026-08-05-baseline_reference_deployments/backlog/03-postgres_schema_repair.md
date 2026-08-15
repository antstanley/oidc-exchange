# Task 03 — Postgres schema repair

**Plan:** [plan.md](../plan.md) · **Certificate:** omitted by requester

**Implements:** [change spec §Implementation notes — A.5 Postgres schema drift](../../../changes/2026-08-05-baseline_reference_deployments.md#implementation-notes), [change spec §Regression tests](../../../changes/2026-08-05-baseline_reference_deployments.md#regression-tests), [service persistence §PostgreSQL](../../../service/specs/08-persistence.md#postgresql)
**Depends on:** —
**Produces:** a Linux Postgres example and mirrored adapter migrations that converge drifted databases to the intended partial unique index.
**Pointers:** `examples/linux-postgres/init.sql:1-14`; `examples/linux-postgres/docker-compose.yml:12`; `crates/adapters/src/postgres/mod.rs:15-40,868-885`; `crates/adapters/src/sqlite/mod.rs:17-52`

## Steps

- [ ] Correct the example `users` DDL to include `version` and create the partial `(external_id, provider)` uniqueness index for live users.
- [ ] Add explicit migration repair for the old `idx_users_external_id` name before the existing index replacement in both Postgres and SQLite migration strings.
- [ ] Extend adapter integration coverage by bootstrapping the old example schema, applying migrations, and exercising deletion/re-creation plus the same external ID across different providers.
- [ ] Decide whether to retain or delete `init.sql` only after establishing whether pre-application DDL is required; preserve adapter self-repair in either outcome.

## Definition of done

- [ ] A database initialized from the old example schema converges to exactly the intended partial unique index after migrations.
- [ ] The regression proves same external IDs across providers coexist and a soft-deleted identity can be recreated.
- [ ] Migration changes are mirrored between Postgres and SQLite where the spec requires them.
- [ ] The `init.sql` retention/deletion decision is documented, with no untracked bootstrap behavior remaining.
- [ ] Meets the repo definition of done (Rust format, clippy, nextest and applicable compose/integration gates; negative-space tests; named-constant limits where introduced — see plan.md baseline).
- [ ] Reviewable: inspect catalog indexes after migration and execute the two-provider and re-registration test path.
