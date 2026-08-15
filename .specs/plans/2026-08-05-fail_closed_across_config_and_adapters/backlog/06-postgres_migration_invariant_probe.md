# Task 06 — PostgreSQL migration invariant probe

**Plan:** [plan.md](../plan.md)  
**Status:** Backlog  
**Implements:** [source spec](../../../changes/2026-08-05-fail_closed_across_config_and_adapters.md) → Persistence PostgreSQL and Implementation note 6; [persistence canonical page](../../../service/specs/08-persistence.md) → PostgreSQL  
**Depends on:** —  
**Produces:** a DDL-denied (`42501`) migration fallback that returns a pool only after verifying tables, the unique partial external-id/provider index, and the `users.version` column; every missing or inconclusive probe returns the original migration error.  
**Pointers:** `crates/adapters/src/postgres/mod.rs`; Postgres integration-test setup/configuration; `schemas/datamodel.schema.json` (read-only contract reference).

## Steps

- [ ] Replace the table-presence-only probe with a query/queries that establish: `users` and
  `sessions` exist; `idx_users_external_id_provider` exists; its `pg_index.indisunique` is true;
  its predicate is non-null (`indpred`); and `users.version` exists.
- [ ] Preserve the existing control-flow contract: only a structured `42501` migration failure
  takes the fallback, every other migration error fails fast, and every failed/inconclusive probe
  returns the original DDL error rather than a probe-derived error.
- [ ] Make probe result decoding explicit and total (no production `unwrap`); add assertions that
  verify the named index/column semantics rather than merely counting rows.
- [ ] Add targeted tests against a restricted-role/preprovisioned database or an equivalent
  controlled harness for: fully compliant schema proceeds; missing partial index fails; full or
  non-unique/non-partial index fails; missing version fails; failed probe fails; and non-42501
  migration errors still fail fast. Ensure the observed returned error remains the original one.
- [ ] Document any local/Postgres-service requirement for these tests and keep it isolated from
  unrelated adapter baseline failures.

## Definition of done

- [ ] `42501` no longer treats table existence as proof of the migration invariant.
- [ ] The unique partial index and version-column checks are explicit, independently tested, and
  cannot pass on a full/non-unique/non-partial index.
- [ ] All failure or probe-error paths surface the original migration error; successful
  pre-provisioned schemas still boot under the restricted role.
- [ ] Relevant adapters tests plus format/lint results are reported; unrelated config/adapters
  baseline failures are not repaired as drive-by work.

## Sibling boundaries

- This task does not change migration ownership, schema design, or user lifecycle behavior beyond
  verifying the invariants the existing migration already promises.
