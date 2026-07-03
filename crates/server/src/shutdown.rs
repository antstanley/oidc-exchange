//! Graceful shutdown for the non-Lambda server (`04-http-api.md` Bootstrap step 6).
//!
//! On SIGTERM or ctrl-c, `axum::serve(..).with_graceful_shutdown(shutdown_signal())` stops
//! accepting new connections and drains in-flight ones — but that drain has no timeout of its
//! own and can hang forever on a straggling connection. [`run_with_drain_deadline`] races the
//! serve future against a watchdog that starts counting down
//! [`SHUTDOWN_DRAIN_DEADLINE_SECS`] *only once the shutdown signal fires* — with no signal the
//! server keeps serving indefinitely, exactly as if no deadline existed at all — so the
//! process always exits deterministically within the deadline of the *signal*, not of process
//! startup, instead of leaning on the deployment platform's SIGKILL timing.

use std::future::Future;
use std::time::Duration;

use tokio::sync::watch;

/// Hard deadline, in seconds, on how long the server drains in-flight requests after a
/// shutdown signal (SIGTERM or ctrl-c) before stragglers are aborted and the process exits.
/// Matches [`04-http-api.md`](../../../.specs/service/specs/04-http-api.md) Bootstrap step 6.
pub const SHUTDOWN_DRAIN_DEADLINE_SECS: u64 = 10;

/// Resolve when the process receives SIGTERM or ctrl-c, whichever comes first.
///
/// Intended as the underlying event behind [`ShutdownSignal`]: once this resolves, hyper
/// stops accepting new connections and starts draining in-flight ones.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl-c handler");
    };

    // SIGTERM is what ECS/K8s send on a rollout; `tokio::signal::unix` is Unix-only. The server
    // crate is nonetheless compiled for Windows (the napi/pyo3 bindings ship a win32 platform
    // package), so the terminate branch is `cfg`-gated: on Windows the closest rollout/stop
    // events are CTRL_CLOSE / CTRL_SHUTDOWN.
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = async {
        let mut ctrl_close =
            tokio::signal::windows::ctrl_close().expect("failed to install ctrl-close handler");
        let mut ctrl_shutdown = tokio::signal::windows::ctrl_shutdown()
            .expect("failed to install ctrl-shutdown handler");
        tokio::select! {
            _ = ctrl_close.recv() => {}
            _ = ctrl_shutdown.recv() => {}
        }
    };

    tokio::select! {
        () = ctrl_c => {
            tracing::info!("received ctrl-c, starting graceful shutdown");
        }
        () = terminate => {
            tracing::info!("received termination signal, starting graceful shutdown");
        }
    }
}

/// A shutdown signal that multiple independent observers can each `wait()` on.
///
/// Both the graceful-shutdown hook handed to `axum::serve(..).with_graceful_shutdown(..)` and
/// the drain-deadline watchdog in [`run_with_drain_deadline`] must observe the *same* signal
/// instant — otherwise the watchdog's deadline clock cannot be anchored to when the signal
/// actually fires. A plain `async fn` can only be awaited by one consumer (or requires
/// re-invoking the OS-signal installers per consumer, which is wasteful and racy); this type
/// fans a single underlying signal out to any number of clones via a `tokio::sync::watch`
/// channel, which is race-free regardless of when each clone starts waiting relative to the
/// signal firing.
#[derive(Clone)]
pub struct ShutdownSignal(watch::Receiver<bool>);

impl ShutdownSignal {
    /// Spawn a task that waits for [`shutdown_signal`] and then arms every clone of the
    /// returned [`ShutdownSignal`]. Cloning is cheap (an `Arc`-backed receiver handle).
    pub fn spawn() -> Self {
        let (tx, rx) = watch::channel(false);
        tokio::spawn(async move {
            shutdown_signal().await;
            // A send error means every receiver was already dropped, i.e. nothing is
            // listening any more — nothing left to notify.
            let _ = tx.send(true);
        });
        Self(rx)
    }

    /// Wrap an already-fired signal source (test helper) so the same racing logic used in
    /// production can be exercised against a controllable trigger instead of real OS signals.
    #[cfg(test)]
    fn from_receiver(rx: watch::Receiver<bool>) -> Self {
        Self(rx)
    }

    /// Resolve once the underlying signal has fired — immediately if it already had by the
    /// time `wait` was called.
    pub async fn wait(mut self) {
        if *self.0.borrow() {
            return;
        }
        let _ = self.0.changed().await;
    }
}

/// Outcome of [`run_with_drain_deadline`].
#[derive(Debug, PartialEq, Eq)]
pub enum DrainOutcome<T> {
    /// `serve_future` ran to completion (the drain finished, before the deadline elapsed —
    /// or before any signal fired at all).
    Completed(T),
    /// The signal fired and the drain had not finished [`SHUTDOWN_DRAIN_DEADLINE_SECS`] later;
    /// stragglers were aborted by dropping `serve_future`.
    DeadlineExceeded,
}

/// Run `serve_future` — the `axum::serve(...).with_graceful_shutdown(...)` future — racing it
/// against a watchdog that only starts counting `deadline` once `signal` resolves.
///
/// This is the crux of bounding the *post-signal* drain without bounding the server's overall
/// lifetime: with no signal, `signal` never resolves, the watchdog never starts its sleep, and
/// `serve_future` is free to run forever (i.e. until the process receives SIGTERM/ctrl-c, same
/// as if no deadline existed). Once `signal` does resolve, the watchdog sleeps for `deadline`
/// and — if `serve_future` (which itself should be wired to stop accepting connections and
/// start draining on the same signal, via `with_graceful_shutdown`) has not completed by
/// then — this function returns [`DrainOutcome::DeadlineExceeded`], dropping `serve_future`
/// and aborting whatever stragglers remain.
///
/// `signal` must be a *clone* (or otherwise-independent observer) of the same underlying event
/// passed to `with_graceful_shutdown` when constructing `serve_future` — see [`ShutdownSignal`].
///
/// `deadline` is injected (rather than hard-coding [`SHUTDOWN_DRAIN_DEADLINE_SECS`] here) so
/// tests can exercise the timeout branch with a short duration; production code always passes
/// `Duration::from_secs(SHUTDOWN_DRAIN_DEADLINE_SECS)`.
pub async fn run_with_drain_deadline<F>(
    serve_future: F,
    signal: ShutdownSignal,
    deadline: Duration,
) -> DrainOutcome<F::Output>
where
    F: Future,
{
    assert!(
        !deadline.is_zero(),
        "shutdown drain deadline must be non-zero, got {deadline:?}"
    );
    assert!(
        deadline <= Duration::from_secs(SHUTDOWN_DRAIN_DEADLINE_SECS * 60),
        "shutdown drain deadline {deadline:?} is implausibly large relative to the documented \
         {SHUTDOWN_DRAIN_DEADLINE_SECS}s production deadline"
    );

    let watchdog = async move {
        signal.wait().await;
        tokio::time::sleep(deadline).await;
    };

    tokio::select! {
        output = serve_future => DrainOutcome::Completed(output),
        () = watchdog => DrainOutcome::DeadlineExceeded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::pending;
    use std::time::Instant;

    /// Short deadline used in place of [`SHUTDOWN_DRAIN_DEADLINE_SECS`] so the timeout tests
    /// run fast; production code always uses the named constant instead.
    const TEST_SHORT_DEADLINE: Duration = Duration::from_millis(50);

    /// Build a [`ShutdownSignal`] plus a trigger closure the test can call to fire it later,
    /// standing in for the real OS-signal-backed [`ShutdownSignal::spawn`].
    fn controllable_signal() -> (ShutdownSignal, watch::Sender<bool>) {
        let (tx, rx) = watch::channel(false);
        (ShutdownSignal::from_receiver(rx), tx)
    }

    /// Core invariant this task exists to fix: with **no signal**, the deadline clock must
    /// never start, so a server that is otherwise happily serving (modeled here by a
    /// never-completing `serve_future`, standing in for `axum::serve` accepting connections
    /// forever) must keep running *past* the deadline rather than self-exiting at
    /// startup+deadline.
    #[tokio::test]
    async fn no_signal_keeps_running_past_the_deadline() {
        let (signal, _tx) = controllable_signal(); // never fires — `_tx` kept alive so the
                                                   // channel isn't dropped out from under `signal`.

        // The whole point: race `run_with_drain_deadline` itself against an *outer* timeout
        // several multiples of the drain deadline. If the bug (deadline anchored at startup)
        // were still present, `run_with_drain_deadline` would resolve at ~1x the deadline and
        // this outer timeout would see `Ok(..)`, not `Err(Elapsed)`.
        let outer_timeout = TEST_SHORT_DEADLINE * 10;
        let result = tokio::time::timeout(
            outer_timeout,
            run_with_drain_deadline(pending::<()>(), signal, TEST_SHORT_DEADLINE),
        )
        .await;

        assert!(
            result.is_err(),
            "with no shutdown signal, run_with_drain_deadline must keep running past the \
             deadline instead of self-exiting at startup+deadline, but it resolved to \
             {result:?} within {outer_timeout:?}"
        );
    }

    /// Positive counterpart: once the signal fires, a drain that never completes on its own
    /// (a hung in-flight request) must not be allowed to hang the process — the watchdog must
    /// abort it and return `DeadlineExceeded` within (roughly) `deadline` *of the signal*, not
    /// of process/test startup.
    #[tokio::test]
    async fn signal_then_non_completing_drain_hits_deadline() {
        let (signal, tx) = controllable_signal();

        // Fire the signal after a short, arbitrary delay so the deadline clock's start is
        // observably later than `start`, proving the watchdog is anchored to the signal.
        let signal_delay = Duration::from_millis(10);
        tokio::spawn(async move {
            tokio::time::sleep(signal_delay).await;
            let _ = tx.send(true);
        });

        let start = Instant::now();
        let result = run_with_drain_deadline(pending::<()>(), signal, TEST_SHORT_DEADLINE).await;
        let elapsed = start.elapsed();

        assert_eq!(
            result,
            DrainOutcome::DeadlineExceeded,
            "a drain that never completes after the signal fires must hit the deadline"
        );
        assert!(
            elapsed >= signal_delay + TEST_SHORT_DEADLINE,
            "elapsed {elapsed:?} was shorter than signal_delay + deadline \
             ({signal_delay:?} + {TEST_SHORT_DEADLINE:?}) — the deadline clock started before \
             the signal fired"
        );
        assert!(
            elapsed < signal_delay + TEST_SHORT_DEADLINE * 10,
            "run_with_drain_deadline took {elapsed:?}, far longer than expected — the process \
             would hang instead of exiting deterministically"
        );
    }

    /// Negative space is covered above; this is the positive counterpart — a drain that
    /// completes on its own (no signal involved at all) returns the inner future's output
    /// rather than waiting on the signal or the deadline.
    #[tokio::test]
    async fn drain_returns_inner_output_when_it_completes_first() {
        let (signal, _tx) = controllable_signal(); // never fires

        let result = run_with_drain_deadline(async { 42 }, signal, Duration::from_secs(5)).await;

        assert_eq!(
            result,
            DrainOutcome::Completed(42),
            "a drain that finishes on its own must succeed without needing a signal"
        );
    }

    /// Negative-space test for the deadline-validation assertion: a zero deadline is a
    /// programmer error (it would make every drain time out instantly once signalled,
    /// indistinguishable from "no graceful shutdown at all"), so `run_with_drain_deadline`
    /// must panic rather than silently accept it.
    #[tokio::test]
    #[should_panic(expected = "must be non-zero")]
    async fn rejects_zero_deadline() {
        let (signal, _tx) = controllable_signal();
        let _ = run_with_drain_deadline(pending::<()>(), signal, Duration::ZERO).await;
    }
}
