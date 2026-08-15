# Task 01 — Immediate session span redactions

**Status:** Backlog · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §Implementation notes step 1](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`08-persistence.md` Session-only stores](../../../service/specs/08-persistence.md)
**Depends on:** —
**Produces:** LMDB and Valkey session methods explicitly skip sensitive arguments and record only the permitted schema fields before the broad `Secret<T>` migration.
**Pointers:** `crates/adapters/src/lmdb/mod.rs:55,102,140`; `crates/adapters/src/valkey/mod.rs:52,141,211`; sibling patterns in Dynamo/Postgres/SQLite.

## Steps

- [ ] Change LMDB store instrumentation to skip `session` on writes and record only `%session.user_id`; skip `token_hash` on lookup/revoke while retaining an empty `token_hash` schema field.
- [ ] Make the equivalent explicit `skip(...)` changes in Valkey; do not record `Session`, refresh-token hash, IP address, user agent, or device ID.
- [ ] Audit all three methods on both backends for accidental default argument capture.
- [ ] Add focused tracing-capture regression tests with span-close events enabled that place sentinel hash/provenance values in sessions and prove none occurs in output while `user_id`/field schema remains observable.

## Task-specific definition of done

- [ ] LMDB and Valkey write, lookup, and revoke spans cannot render sentinels from the hash or provenance fields.
- [ ] Tests use `FmtSpan::CLOSE` (or equivalent explicit close-event capture), avoiding a vacuous no-span assertion.
- [ ] Existing session behavior and permitted `user_id` observability remain covered.
- [ ] No certificate file is created; test output is the completion evidence.
