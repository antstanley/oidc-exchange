# Task 09 — Session reaper and internal cleanup

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Bootstrap, internal cleanup route, config `cleanup_interval`, and cleanup ownership.
**Depends on:** 01 · domain_config_port_contract; 03 · sql_session_adapters; 04 · lmdb_session_adapter; 05 · valkey_session_adapter; 06 · dynamodb_session_adapter
**Produces:** periodic long-lived-runtime cleanup, graceful shutdown cancellation, protected Lambda/scheduler cleanup endpoint, and E2E coverage.
**Pointers:** `crates/server/src/{bootstrap,main,routes/internal}.rs`; `crates/core/src/config.rs`; `config/default.toml`; server tests.

## Steps

- [x] Parse validated `session_repository.cleanup_interval`; use a named interval/task policy and `MissedTickBehavior::Skip`.
- [x] Spawn reaper for Hyper/persistent host modes, log every deleted count, retain/abort its handle through graceful shutdown, and do not spawn it in Lambda.
- [x] Add `POST /internal/sessions/cleanup`, protect it with existing internal auth, invoke the same port method, and return deleted count without leaking session/token data.
- [x] Test one tick cleans expired sessions and retired records but not live state; test authentication success/failure, endpoint count response, runtime selection, and shutdown cancellation.

## Definition of done

- [x] Reaping has an owned runtime lifecycle: no detached task survives server shutdown and Lambda does not rely on a frozen in-process interval.
- [x] The scheduler endpoint has equivalent cleanup semantics and rejects unauthenticated callers.
- [x] Native-expiry adapters still execute the cleanup as a safe backstop; SQL/LMDB cleanup includes retired records.
- [x] Timing tests avoid wall-clock flakiness through controllable Tokio time/explicit helper seams.
- [x] Done certificates remain intentionally absent.

## Completion notes

- **Reaper module (`crates/server/src/reaper.rs`).** The loop consumes `tokio::time::interval`'s built-in immediate first tick so the first sweep lands one full interval after spawn (startup never absorbs a whole-store sweep), sets [`REAPER_MISSED_TICK_POLICY`] = `MissedTickBehavior::Skip`, and selects each iteration between the tick and a pinned clone of the process `ShutdownSignal`. Every run logs its outcome — `deleted_rows` on success, a warning with the error on failure — because a silently dead reaper must be distinguishable from an empty one; only counts and error strings are logged. `reap_once(&AppService)` is both the tick body and the explicit test seam for one tick without waiting on real time; it returns the deleted count so tests assert exact sweep arithmetic.
- **Host-runtime selection lives in one tested place.** `reaper::spawn_session_reaper_for_runtime(.., HostRuntime)` returns `None` for `Lambda` without even parsing the interval, and `HostRuntime::detect()` classifies via `AWS_LAMBDA_RUNTIME_API` exactly as `main.rs` always detected serve mode — so the Node/Python Lambda bindings over `crates/ffi` classify as Lambda too. `main.rs` now computes the runtime once and feeds both the serve-mode branch and the reaper gate; its hyper arm aborts the retained handle on *every* path out of the drain (dropping a `JoinHandle` would detach rather than stop).
- **FFI embedders host the reaper too** (deviation from the task file's pointer list, which names only server files — taken on purpose so the canonical Bootstrap step 7 text folds verbatim). `OidcExchange` parks the loop future on its own runtime via `Runtime::spawn` (the reason `reaper_loop` is split from `tokio::spawn`), gets a never-firing signal — embedders have no OS-signal story, so `ShutdownSignal::never()` was added (its sender is deliberately leaked: dropping the only sender closes a watch channel, which resolves every waiter immediately) — and a new `Drop` impl aborts the handle before the runtime shuts down. Lambda-hosted instances spawn nothing.
- **Interval parsing** mirrors `bootstrap::request_timeout_duration`: `cleanup_interval_duration` trusts startup validation and panics loudly if reached with an invalid value instead of silently substituting the default, plus a named sanity bound (`REAPER_INTERVAL_MAX_SECS`, 30 days).
- **Internal route.** `POST /internal/sessions/cleanup` sits behind the existing internal-auth layer, calls the same port method through a shared core entry point (`AppService::cleanup_expired_sessions`, new `service/maintenance.rs`, also used by the reaper), and answers `{ "deleted": <count> }` — nothing else.
- **Adapter coverage note.** SQL/LMDB/mock sweeps of expired sessions *and* retirement records with a combined count already shipped in tasks 03–06 (`cleanup_counts_expired_sessions_and_retired_records_together`); Valkey prunes index sets and reconciles its counter; DynamoDB relies on TTL natively with the batch-delete backstop. This task adds no adapter changes.
- Tests: `reap_once_sweeps_expired_rows_and_spares_live_state` (expired session + expired retirement record swept, live generation spared, second tick zero), `reap_once_reports_zero_on_a_failing_sweep` (failing store never reads as "nothing to delete"), `spawned_reaper_sweeps_periodically` and `shutdown_signal_stops_the_reaper_before_its_next_tick` under paused Tokio time (`test-util` dev-dependency added), `only_persistent_runtimes_get_a_reaper_handle`, interval-parsing positive/negative space, `ShutdownSignal::never` staying unarmed over a day of advanced time, and four endpoint tests (401 missing secret, 401 wrong secret, `{"deleted":0}` empty store with no extra fields, combined-count sweep + idempotence through the full router).
- Gates at commit: nextest workspace 468 passed / 50 skipped (+14 vs the 454 baseline); fmt and clippy `-D warnings` clean.
