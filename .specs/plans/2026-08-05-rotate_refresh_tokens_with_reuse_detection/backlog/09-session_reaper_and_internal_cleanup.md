# Task 09 — Session reaper and internal cleanup

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Bootstrap, internal cleanup route, config `cleanup_interval`, and cleanup ownership.
**Depends on:** 01 · domain_config_port_contract; 03 · sql_session_adapters; 04 · lmdb_session_adapter; 05 · valkey_session_adapter; 06 · dynamodb_session_adapter
**Produces:** periodic long-lived-runtime cleanup, graceful shutdown cancellation, protected Lambda/scheduler cleanup endpoint, and E2E coverage.
**Pointers:** `crates/server/src/{bootstrap,main,routes/internal}.rs`; `crates/core/src/config.rs`; `config/default.toml`; server tests.

## Steps

- [ ] Parse validated `session_repository.cleanup_interval`; use a named interval/task policy and `MissedTickBehavior::Skip`.
- [ ] Spawn reaper for Hyper/persistent host modes, log every deleted count, retain/abort its handle through graceful shutdown, and do not spawn it in Lambda.
- [ ] Add `POST /internal/sessions/cleanup`, protect it with existing internal auth, invoke the same port method, and return deleted count without leaking session/token data.
- [ ] Test one tick cleans expired sessions and retired records but not live state; test authentication success/failure, endpoint count response, runtime selection, and shutdown cancellation.

## Definition of done

- [ ] Reaping has an owned runtime lifecycle: no detached task survives server shutdown and Lambda does not rely on a frozen in-process interval.
- [ ] The scheduler endpoint has equivalent cleanup semantics and rejects unauthenticated callers.
- [ ] Native-expiry adapters still execute the cleanup as a safe backstop; SQL/LMDB cleanup includes retired records.
- [ ] Timing tests avoid wall-clock flakiness through controllable Tokio time/explicit helper seams.
- [ ] Done certificates remain intentionally absent.
