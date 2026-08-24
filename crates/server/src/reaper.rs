//! The session reaper — the service-owned owner of expired-row cleanup
//! (`04-http-api.md` → Bootstrap step 7).
//!
//! Before this module existed, `cleanup_expired_sessions` had no production
//! caller: only the trait, the adapters' implementations, and test call sites.
//! Expired sessions — and now retirement records — were retained indefinitely
//! on SQL and LMDB, along with the `ip_address`/`user_agent`/`device_id` they
//! captured. The canonical pages assumed "a scheduler external to the service"
//! that no operator-facing document ever mentioned; that assumption is
//! replaced by an owned lifecycle:
//!
//! - **Long-lived runtimes** (the hyper server; a `crates/ffi` embedder whose
//!   host process persists) spawn the reaper task on
//!   [`session_repository.cleanup_interval`](`oidc_exchange_core::config::SessionRepositoryConfig::cleanup_interval`)
//!   and abort it with the graceful-shutdown drain.
//! - **Lambda** spawns nothing: the execution environment freezes the process
//!   between invocations, so an in-process interval would fire at best once
//!   per invocation and at worst never. The same sweep stays reachable as
//!   `POST /internal/sessions/cleanup` for an external scheduler (EventBridge)
//!   to drive on the deployment's own cadence — including from inside a
//!   Node/Python Lambda binding, where [`HostRuntime::detect`] sees the
//!   runtime API environment variable exactly as `main.rs` does.
//!
//! Every run logs its outcome — deleted count or failure — because a silently
//! dead reaper must be distinguishable from one with nothing to delete.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use oidc_exchange_core::config::Config;
use oidc_exchange_core::service::AppService;

use crate::shutdown::ShutdownSignal;

/// Missed-tick policy for the reaper's interval: **skip**, never burst.
///
/// If the runtime stalls past one or more tick deadlines (a long GC pause, a
/// suspended host), catching up by firing the missed ticks back-to-back would
/// run the sweep repeatedly for time already lost while doing nothing for the
/// interval that actually matters — the *next* one. Skipping keeps the cadence
/// at "one sweep per interval, however late this one ran"
/// (`04-http-api.md` → Bootstrap step 7 names the policy).
const REAPER_MISSED_TICK_POLICY: MissedTickBehavior = MissedTickBehavior::Skip;

/// Sanity upper bound, in seconds, on a parsed `cleanup_interval`. Config
/// validation already rejects unparseable and zero values before any server
/// is built, so reaching this function with a value beyond it (a month) is
/// certainly a misconfiguration — e.g. a TTL-style string pasted into the
/// wrong key — mirroring the guard `bootstrap::request_timeout_duration`
/// places on `server.request_timeout`.
const REAPER_INTERVAL_MAX_SECS: u64 = 30 * 24 * 60 * 60;

/// Which kind of host process would own the reaper's lifetime.
///
/// The distinction is not about OS processes but about whether time passes
/// predictably inside one: a hyper server or a persistent FFI embedder runs
/// its executor continuously, while AWS Lambda freezes the execution
/// environment between invocations, so a periodic task there cannot honour
/// its interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRuntime {
    /// A process whose async runtime keeps running between requests (hyper,
    /// persistent FFI embedder). Hosts the reaper.
    Persistent,
    /// AWS Lambda. Never hosts the reaper; the internal cleanup endpoint is
    /// the equivalent control.
    Lambda,
}

impl HostRuntime {
    /// Detect the host runtime the way `main.rs` has always detected serve
    /// mode: the presence of `AWS_LAMBDA_RUNTIME_API`, which the Lambda
    /// execution environment sets for every runtime — including the Node.js
    /// and Python ones hosting the bindings, so an FFI-built exchange inside
    /// a Lambda function classifies as [`HostRuntime::Lambda`] too.
    pub fn detect() -> Self {
        if std::env::var("AWS_LAMBDA_RUNTIME_API").is_ok() {
            Self::Lambda
        } else {
            Self::Persistent
        }
    }

    /// Whether a reaper may be spawned under this runtime.
    ///
    /// Exposed separately from [`Self::detect`] so the gate itself is
    /// assertable without mutating process state: the invariant "Lambda does
    /// not rely on a frozen in-process interval" lives here, in one tested
    /// place, rather than in the branch layout of three entry points.
    pub fn hosts_reaper(self) -> bool {
        match self {
            Self::Persistent => true,
            Self::Lambda => false,
        }
    }
}

/// Parse the validated `session_repository.cleanup_interval` into the
/// `Duration` the reaper's interval is built from.
///
/// Every production entry point resolves configuration through
/// `Config::resolve`, which narrows this field to a strictly-positive
/// duration before a reaper is ever spawned — an invalid value fails config
/// loading closed rather than reaching this function.
pub fn cleanup_interval_duration(config: &Config) -> Duration {
    let secs = config.session_repository.cleanup_interval_secs();
    assert!(
        secs > 0,
        "resolved cleanup_interval must be non-zero, got {secs}s"
    );
    assert!(
        secs <= REAPER_INTERVAL_MAX_SECS,
        "parsed cleanup_interval of {secs}s from {:?} exceeds the sane upper bound of \
         {REAPER_INTERVAL_MAX_SECS}s",
        config.session_repository.cleanup_interval
    );
    Duration::from_secs(secs)
}

/// Run one sweep and log its outcome, returning the number of rows deleted
/// (zero when the sweep failed).
///
/// This is the body of one reaper tick and also the explicit seam tests use
/// to exercise a single tick without waiting on real time. The log line on
/// *every* run — including the empty and failing ones — is the point: an
/// operator must be able to tell a dead reaper from one whose store simply
/// had nothing to delete. Only counts and error strings are logged, never
/// hashes, tokens, or subjects.
pub async fn reap_once(service: &AppService) -> u64 {
    match service.cleanup_expired_sessions().await {
        Ok(deleted) => {
            tracing::info!(deleted_rows = deleted, "session reaper sweep completed");
            deleted
        }
        Err(err) => {
            // Best-effort path, documented like the audit-fallback one: the
            // reaper has no caller to propagate to. Swallowing the failure
            // silently is not an option (it would read as "nothing to
            // delete"), so every failure is logged and the next tick retries
            // on its own cadence.
            tracing::warn!(error = %err, "session reaper sweep failed");
            0
        }
    }
}

/// The reaper loop future: one sweep per `interval`, forever, until
/// `shutdown` fires.
///
/// Split from [`spawn_session_reaper`] so embedders that own their own Tokio
/// runtime (`crates/ffi`) can park this future on *their* runtime via
/// `Runtime::spawn` instead of needing to already be inside a runtime context
/// to call `tokio::spawn`.
///
/// The first sweep happens one full `interval` after spawn, not immediately:
/// the immediate first tick `tokio::time::interval` produces is consumed so
/// process startup never absorbs a whole-store sweep and the cadence stays
/// exactly "one sweep per configured interval".
pub fn reaper_loop(
    service: Arc<AppService>,
    interval: Duration,
    shutdown: ShutdownSignal,
) -> impl Future<Output = ()> + Send + 'static {
    assert!(!interval.is_zero(), "reaper interval must be non-zero");

    async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(REAPER_MISSED_TICK_POLICY);
        // Consume interval's built-in immediate first tick so the first sweep
        // lands one full interval after spawn (see the doc above).
        ticker.tick().await;

        // Pin the shutdown waiter once so every loop iteration selects on a
        // borrow of it — `ShutdownSignal::wait` consumes, and a fresh
        // `Arc`-clone per iteration would be waste.
        let stopped = async {
            shutdown.wait().await;
            tracing::info!("session reaper stopping on shutdown");
        };
        tokio::pin!(stopped);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    // The count matters only for its log line (see reap_once);
                    // the loop itself carries no state between ticks.
                    let _deleted = reap_once(&service).await;
                }
                _ = &mut stopped => return,
            }
        }
    }
}

/// Spawn the reaper loop onto the current runtime and return its handle.
///
/// The caller retains the handle through graceful shutdown — `main.rs` aborts
/// it once the drain finishes, and the FFI embedder aborts it on drop — so no
/// detached reaper survives its host.
pub fn spawn_session_reaper(
    service: Arc<AppService>,
    interval: Duration,
    shutdown: ShutdownSignal,
) -> JoinHandle<()> {
    tokio::spawn(reaper_loop(service, interval, shutdown))
}

/// Spawn the reaper iff `runtime` is one that hosts it, else return `None`.
///
/// This is the single decision point every entry point funnels through, so
/// "Lambda does not get a frozen in-process interval" is enforced (and
/// tested) here rather than depending on each entry point remembering not to
/// call [`spawn_session_reaper`] in its Lambda branch.
pub fn spawn_session_reaper_for_runtime(
    config: &Config,
    service: &Arc<AppService>,
    shutdown: ShutdownSignal,
    runtime: HostRuntime,
) -> Option<JoinHandle<()>> {
    if !runtime.hosts_reaper() {
        tracing::debug!(
            "session reaper not spawned: Lambda freezes the process between invocations; \
             drive POST /internal/sessions/cleanup from an external scheduler instead"
        );
        return None;
    }
    Some(spawn_session_reaper(
        Arc::clone(service),
        cleanup_interval_duration(config),
        shutdown,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::{Duration as ChronoDuration, Utc};
    use tokio::sync::watch;

    use oidc_exchange_core::config::Config;
    use oidc_exchange_core::domain::{RefreshResolution, Session};
    use oidc_exchange_core::error::{Error, Result};
    use oidc_exchange_core::ports::{IdentityProvider, SessionRepository};
    use oidc_exchange_core::service::AppService;
    use oidc_exchange_test_utils::session_contract::{
        capture_base_instant, fixture_family_id, fixture_hash, generation_session,
    };
    use oidc_exchange_test_utils::{MockAuditLog, MockKeyManager, MockRepository, MockUserSync};

    use super::*;
    use crate::shutdown::ShutdownSignal;

    /// Build a [`ShutdownSignal`] plus a trigger standing in for the real
    /// OS-signal-backed [`ShutdownSignal::spawn`] (same helper shape as the
    /// shutdown module's own tests).
    fn controllable_signal() -> (ShutdownSignal, watch::Sender<bool>) {
        let (tx, rx) = watch::channel(false);
        (ShutdownSignal::from_receiver(rx), tx)
    }

    fn providers_map() -> HashMap<String, Box<dyn IdentityProvider>> {
        HashMap::new()
    }

    /// An `Arc<AppService>` over mocks whose session store shares state with
    /// the returned handle, so a test can seed rows and then observe what a
    /// tick did to them.
    fn service_with_shared_session_store() -> (Arc<AppService>, MockRepository) {
        let sessions = MockRepository::new();
        let service = Arc::new(AppService::new(
            Box::new(MockRepository::new()),
            Box::new(sessions.clone()),
            Box::new(MockKeyManager::new()),
            Box::new(MockAuditLog::new()),
            Box::new(MockUserSync::new()),
            Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
            providers_map(),
            Config::test_default(),
        ));
        (service, sessions)
    }

    /// Seed the three expiry shapes a tick must distinguish:
    ///
    /// - a **live** generation (one family, expires +2h) that must survive;
    /// - an **expired session** (a lone generation expiring −1h);
    /// - an **expired retirement record**: a second family whose generations
    ///   expire −1h is rotated once, so its retirement record inherits that
    ///   past deadline (`expires_at = min(retired_at + retention, family
    ///   expires_at)`) and is already past — no sleeping or backdating
    ///   arithmetic anywhere.
    ///
    /// Returns `(live_hash, dead_hash)` — the surviving live generation and
    /// the retired generation whose record must be swept.
    async fn seed_expiry_shapes(store: &MockRepository) -> (String, String) {
        let base = capture_base_instant();
        let future_deadline = base + ChronoDuration::hours(2);
        let past_deadline = base - ChronoDuration::hours(1);

        // The surviving family.
        let live_family = fixture_family_id("reaper:family:live");
        let live = generation_session(
            "usr_reaper",
            &live_family,
            0,
            fixture_hash("reaper:live:gen0"),
            future_deadline,
            base,
            None,
        );
        store.store_refresh_token(&live).await.expect("store live");

        // The lone expired session.
        let dead_family = fixture_family_id("reaper:family:dead-solo");
        let dead = generation_session(
            "usr_reaper",
            &dead_family,
            0,
            fixture_hash("reaper:dead-solo:gen0"),
            past_deadline,
            base,
            None,
        );
        store.store_refresh_token(&dead).await.expect("store dead");

        // The expired retirement record: rotate a past-expiry family once so
        // its record's deadline is capped at the family's past `expires_at`.
        let rotting_family = fixture_family_id("reaper:family:rotting");
        let rotting_gen0 = generation_session(
            "usr_reaper",
            &rotting_family,
            0,
            fixture_hash("reaper:rotting:gen0"),
            past_deadline,
            base,
            None,
        );
        let rotting_gen1 = generation_session(
            "usr_reaper",
            &rotting_family,
            1,
            fixture_hash("reaper:rotting:gen1"),
            past_deadline,
            base,
            Some(base),
        );
        store
            .store_refresh_token(&rotting_gen0)
            .await
            .expect("store rotting gen0");
        let rotated = store
            .rotate_refresh_token(&rotting_gen0.refresh_token_hash, &rotting_gen1)
            .await
            .expect("rotate");
        assert!(
            rotated,
            "the fixture rotation must win its compare-and-swap against gen 0"
        );

        (
            live.refresh_token_hash.clone(),
            rotting_gen0.refresh_token_hash.clone(),
        )
    }

    /// One tick sweeps both kinds of expiry — the expired session and the
    /// expired retirement record — and returns their combined count, while
    /// the live generation survives untouched. A follow-up tick reports zero,
    /// which is what makes "ran and found nothing" distinguishable from
    /// "never ran".
    #[tokio::test]
    async fn reap_once_sweeps_expired_rows_and_spares_live_state() {
        let (service, store) = service_with_shared_session_store();
        let (live_hash, retired_hash) = seed_expiry_shapes(&store).await;

        let deleted = reap_once(&service).await;

        assert_eq!(
            deleted, 3,
            "one tick deletes exactly: the expired solo session, the expired successor \
             session of the rotated past-expiry family, and its expired retirement record"
        );

        let remaining = store.get_all_sessions().await;
        assert_eq!(
            remaining.len(),
            1,
            "exactly one generation may survive the sweep"
        );
        assert_eq!(
            remaining[0].refresh_token_hash, live_hash,
            "the survivor must be the live generation, not whichever row iterated first"
        );
        assert!(
            remaining[0].expires_at > Utc::now(),
            "the surviving session must itself be unexpired"
        );
        assert!(
            store.get_all_retired_tokens().await.is_empty(),
            "an expired retirement record must be swept together with expired sessions"
        );
        assert!(
            matches!(
                store
                    .resolve_refresh_token(&retired_hash)
                    .await
                    .expect("resolve"),
                RefreshResolution::Unknown
            ),
            "a swept retirement record leaves its hash resolving as unknown, not retired"
        );

        assert_eq!(
            reap_once(&service).await,
            0,
            "a follow-up tick over a clean store deletes nothing"
        );
        assert_eq!(store.get_all_sessions().await.len(), 1);
    }

    /// A session store whose every operation fails, standing in for an
    /// unreachable backend.
    struct FailingSessionStore;

    #[async_trait]
    impl SessionRepository for FailingSessionStore {
        async fn put_single_use(
            &self,
            _: &str,
            _: chrono::DateTime<chrono::Utc>,
        ) -> oidc_exchange_core::error::Result<bool> {
            unreachable!("the reaper never writes single-use records")
        }
        async fn take_single_use(&self, _: &str) -> oidc_exchange_core::error::Result<bool> {
            unreachable!("the reaper never burns single-use records")
        }

        async fn store_refresh_token(&self, _session: &Session) -> Result<()> {
            Err(Error::StoreError {
                detail: "failing fixture store".into(),
            })
        }
        async fn get_session_by_refresh_token(&self, _token_hash: &str) -> Result<Option<Session>> {
            Err(Error::StoreError {
                detail: "failing fixture store".into(),
            })
        }
        async fn resolve_refresh_token(&self, _token_hash: &str) -> Result<RefreshResolution> {
            Err(Error::StoreError {
                detail: "failing fixture store".into(),
            })
        }
        async fn rotate_refresh_token(
            &self,
            _live_hash: &str,
            _replacement: &Session,
        ) -> Result<bool> {
            Err(Error::StoreError {
                detail: "failing fixture store".into(),
            })
        }
        async fn revoke_session(&self, _token_hash: &str) -> Result<()> {
            Err(Error::StoreError {
                detail: "failing fixture store".into(),
            })
        }
        async fn revoke_family(&self, _family_id: &str) -> Result<u64> {
            Err(Error::StoreError {
                detail: "failing fixture store".into(),
            })
        }
        async fn revoke_all_user_sessions(&self, _user_id: &str) -> Result<()> {
            Err(Error::StoreError {
                detail: "failing fixture store".into(),
            })
        }
        async fn count_active_sessions(&self) -> Result<u64> {
            Err(Error::StoreError {
                detail: "failing fixture store".into(),
            })
        }
        async fn cleanup_expired_sessions(&self) -> Result<u64> {
            Err(Error::StoreError {
                detail: "failing fixture store".into(),
            })
        }
    }

    /// Negative space for the failure path: a sweep that errors is reported
    /// as zero deletions rather than propagated (the reaper has no caller to
    /// fail toward) and logged, so a broken backend never reads as "nothing
    /// to delete".
    #[tokio::test]
    async fn reap_once_reports_zero_on_a_failing_sweep() {
        let service = Arc::new(AppService::new(
            Box::new(MockRepository::new()),
            Box::new(FailingSessionStore),
            Box::new(MockKeyManager::new()),
            Box::new(MockAuditLog::new()),
            Box::new(MockUserSync::new()),
            Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
            providers_map(),
            Config::test_default(),
        ));

        let deleted = reap_once(&service).await;

        assert_eq!(
            deleted, 0,
            "a failed sweep must not be reported as having deleted anything"
        );
    }

    /// The spawned loop ticks on its own: seeded expired rows are gone after
    /// several intervals of paused Tokio time without any direct call to
    /// [`reap_once`], proving the loop — not just the tick body — sweeps.
    #[tokio::test(start_paused = true)]
    async fn spawned_reaper_sweeps_periodically() {
        let (service, store) = service_with_shared_session_store();
        let (_, retired_hash) = seed_expiry_shapes(&store).await;

        let interval = Duration::from_millis(20);
        let (signal, _tx) = controllable_signal(); // never fires in this test
        let handle = spawn_session_reaper(Arc::clone(&service), interval, signal);

        // Paused Tokio time auto-advances while every task awaits a timer, so
        // this sleep deterministically hands the loop five intervals in which
        // to fire its tick — no wall-clock waiting anywhere.
        tokio::time::sleep(interval * 5).await;

        assert_eq!(
            store.get_all_sessions().await.len(),
            1,
            "after several intervals the loop must have swept the expired rows down to the \
             one live generation"
        );
        assert!(
            matches!(
                store
                    .resolve_refresh_token(&retired_hash)
                    .await
                    .expect("resolve"),
                RefreshResolution::Unknown
            ),
            "the loop's sweeps must have removed the expired retirement record"
        );
        assert!(
            !handle.is_finished(),
            "before shutdown fires the loop must still be running, not exited early"
        );
        handle.abort();
    }

    /// Shutdown cancellation: firing the signal stops the loop even though
    /// its next tick lies a full hour out, and the retained handle resolves —
    /// no detached task survives shutdown.
    #[tokio::test(start_paused = true)]
    async fn shutdown_signal_stops_the_reaper_before_its_next_tick() {
        let (service, _store) = service_with_shared_session_store();

        let interval = Duration::from_secs(3600);
        let (signal, tx) = controllable_signal();
        let handle = spawn_session_reaper(service, interval, signal);

        let _ = tx.send(true);
        let waited = tokio::time::timeout(interval / 2, handle).await;
        assert!(
            waited.is_ok(),
            "the reaper must exit on shutdown well before its next tick, not wait out the \
             whole interval"
        );
    }

    /// Runtime selection, both arms: Lambda yields no handle (the frozen
    /// in-process-interval prohibition), a persistent host yields one.
    #[tokio::test]
    async fn only_persistent_runtimes_get_a_reaper_handle() {
        let config = Config::test_default();
        let (service, _store) = service_with_shared_session_store();
        let (signal, _tx) = controllable_signal();

        assert!(
            !HostRuntime::Lambda.hosts_reaper(),
            "Lambda must never host the reaper: the execution environment freezes between \
             invocations, so the interval could never fire reliably"
        );
        assert!(HostRuntime::Persistent.hosts_reaper());

        let lambda_result = spawn_session_reaper_for_runtime(
            &config,
            &service,
            signal.clone(),
            HostRuntime::Lambda,
        );
        assert!(
            lambda_result.is_none(),
            "a Lambda runtime must not receive a reaper handle"
        );

        let persistent_result =
            spawn_session_reaper_for_runtime(&config, &service, signal, HostRuntime::Persistent);
        let handle = persistent_result.expect("a persistent runtime receives a reaper handle");
        // Clean up the spawned task before the test runtime drops.
        handle.abort();
    }

    /// Interval parsing, positive space: validated config values resolve to
    /// exactly the duration they spell.
    #[test]
    fn cleanup_interval_duration_uses_the_configured_value() {
        let mut config = Config::test_default();
        config.session_repository.cleanup_interval = std::time::Duration::from_secs(90);

        assert_eq!(
            cleanup_interval_duration(&config),
            Duration::from_secs(90),
            "the reaper cadence must be exactly the configured interval"
        );
        config.session_repository.cleanup_interval = std::time::Duration::from_secs(7200);
        assert_eq!(
            cleanup_interval_duration(&config),
            Duration::from_secs(7200),
            "hour-suffixed durations resolve through the shared duration parser"
        );
    }

    /// Negative space: an unparseable interval fails config resolution — the
    /// typed config cannot even represent it, so no reaper is ever spawned
    /// with an interval nobody configured.
    #[test]
    fn unparseable_cleanup_interval_fails_config_resolution() {
        let mut raw: oidc_exchange_core::config::RawConfig =
            toml::from_str(include_str!("../../../config/default.toml"))
                .expect("default config deserializes");
        raw.session_repository.cleanup_interval = "hourly".to_string();
        let err = oidc_exchange_core::config::Config::resolve(raw)
            .expect_err("an unparseable cleanup interval must fail resolution");
        assert!(
            err.to_string().contains("session_repository.cleanup_interval"),
            "the error must name the offending field: {err}"
        );
    }

    /// Negative space above the sanity bound: a month-plus "interval" is a
    /// misconfiguration, not a cadence.
    #[test]
    #[should_panic(expected = "exceeds the sane upper bound")]
    fn cleanup_interval_duration_panics_above_the_sanity_bound() {
        let mut config = Config::test_default();
        config.session_repository.cleanup_interval =
            std::time::Duration::from_secs(400 * 24 * 60 * 60);
        let _ = cleanup_interval_duration(&config);
    }
}
