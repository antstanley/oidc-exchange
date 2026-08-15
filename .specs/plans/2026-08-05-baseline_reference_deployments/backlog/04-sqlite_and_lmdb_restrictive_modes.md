# Task 04 — SQLite and LMDB restrictive modes

**Plan:** [plan.md](../plan.md) · **Certificate:** omitted by requester

**Implements:** [change spec §Implementation notes — A.6 SQLite file mode and bootstrap transaction](../../../changes/2026-08-05-baseline_reference_deployments.md#implementation-notes), [change spec §Regression tests](../../../changes/2026-08-05-baseline_reference_deployments.md#regression-tests), [service persistence §SQLite](../../../service/specs/08-persistence.md#sqlite), [service persistence §Session-only stores](../../../service/specs/08-persistence.md#session-only-stores)
**Depends on:** —
**Produces:** owner-only SQLite and LMDB state under varied umasks, with SQLite migrations protected by a single bootstrap transaction and a restrictive local setup key.
**Pointers:** `crates/adapters/src/sqlite/mod.rs:17-110,789-...`; `crates/adapters/src/lmdb/mod.rs:24-...`; `examples/linux-sqlite/setup.sh:9-30`; `docs/deployment/linux-sqlite.md`

## Steps

- [ ] Add Unix-only SQLite path preparation before connection creation: create new files at `0600`, tighten existing files, skip `:memory:`, and map failures to typed store errors.
- [ ] Run SQLite migrations in a `BEGIN IMMEDIATE` transaction so index replacement never exposes an unconstrained bootstrap interval.
- [ ] Make the Linux SQLite setup script generate private key material with a restrictive umask and explicit mode, and correct backup instructions that lose source mode.
- [ ] Tighten LMDB directories/files on initial and existing open as required by the change spec.
- [ ] Add deterministic Unix tests across umasks `022`, `002`, and `077`, including pre-existing loose SQLite/LMDB paths and SQLite `-wal`/`-shm` siblings.

## Definition of done

- [ ] SQLite database, WAL, and SHM files are `0600` after create and after opening a pre-existing loose database on Unix.
- [ ] LMDB directory is `0700` and `data.mdb`/`lock.mdb` are `0600` across required umasks and existing loose paths are tightened.
- [ ] SQLite migration execution is atomic and preserves the intended unique index when competing bootstrap writers are possible.
- [ ] `:memory:` remains usable and filesystem failures return typed errors rather than panicking.
- [ ] Meets the repo definition of done (Rust and shell/docs checks applicable to the task; negative-space tests; named-constant limits where introduced — see plan.md baseline).
- [ ] Reviewable: run the mode matrix and inspect filesystem permissions plus the atomic migration test.
