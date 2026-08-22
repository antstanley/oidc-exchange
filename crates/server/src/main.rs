use std::sync::Arc;

use oidc_exchange::bootstrap;
use oidc_exchange::lambda;
use oidc_exchange::shutdown::{self, DrainOutcome, ShutdownSignal, SHUTDOWN_DRAIN_DEADLINE_SECS};
use oidc_exchange::telemetry;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("oidc-exchange {VERSION}");
        return Ok(());
    }

    // 1. Load config
    let config = bootstrap::load_config()?;

    // 2. Init telemetry
    telemetry::init_telemetry(&config.telemetry)?;

    tracing::info!("configuration loaded");

    let role = config.server.role.as_str();
    tracing::info!(role = %role, "server role");

    // 3. Build service and the per-plane routers the role requires.
    let service = bootstrap::build_service(&config).await?;
    let routers = bootstrap::build_routers(&config, service);

    if config.server.role == "all" {
        // Native runtime: both planes bind, each on its own socket. The
        // warning is informational here (unlike the single-plane runtimes,
        // where the same role collapses), but an operator selecting `all`
        // should know the admin listener exists and needs network-policy
        // protection of its own.
        tracing::warn!(
            public = %format!("{}:{}", config.server.host, config.server.port),
            admin = %config.internal_api.bind_address(),
            "role = \"all\" binds two distinct listeners; the admin plane must be \
             firewalled separately from the exchange plane"
        );
    }

    // 4. Run
    if std::env::var("AWS_LAMBDA_RUNTIME_API").is_ok() {
        // Lambda mode: there is one request surface, so exactly one plane can
        // be served (`04-http-api.md` → Bootstrap, step 6). The single-plane
        // rule lives in `Routers::single_plane`: `exchange` and `admin` serve
        // their own plane; `all` serves the public plane and warns that
        // `/internal/*` is unmounted. `lambda::run_lambda` wraps the chosen
        // router in the per-invocation flush hook (`FlushOnResponse`) so
        // telemetry force-flushes synchronously after each invocation's
        // response future resolves. `lambda_http::run` fails with
        // `lambda_http::Error` (`Box<dyn std::error::Error + Send + Sync>`),
        // which the standard library does not blanket-convert into `main`'s
        // `Box<dyn std::error::Error>` (the `Send + Sync` marker traits make
        // the two boxed trait objects distinct types to `?`'s `From`
        // resolution); the error message is preserved and reboxed so the
        // failure still propagates via `?` with no `unwrap`/`expect`.
        let app = routers.single_plane().ok_or_else(|| {
            std::io::Error::other("configured role produces no servable router plane")
        })?;
        tracing::info!("Lambda runtime detected; serving via lambda_http");
        lambda::run_lambda(app, Arc::new(telemetry::flush_telemetry))
            .await
            .map_err(|err| -> Box<dyn std::error::Error> { err.to_string().into() })?;
    } else {
        // Hyper runtime: one socket per router the role produced, served
        // concurrently under one graceful-shutdown signal. Both sockets are
        // bound *before* either server starts, so a bind failure on either
        // fails startup rather than silently serving half the configured
        // surface — a process that cannot bind its admin listener must not
        // keep serving `/token` as if the admin plane did not exist.
        let mut planes: Vec<(String, axum::Router)> = Vec::new();
        if let Some(public) = routers.public {
            let addr = format!("{}:{}", config.server.host, config.server.port);
            assert!(
                !addr.is_empty(),
                "public bind address must not be empty before serving"
            );
            tracing::info!(addr = %addr, plane = "public", "binding listener");
            planes.push((addr, public));
        }
        if let Some(admin) = routers.admin {
            let addr = config.internal_api.bind_address();
            assert!(
                !addr.is_empty(),
                "admin bind address must not be empty before serving"
            );
            tracing::info!(addr = %addr, plane = "admin", "binding listener");
            planes.push((addr, admin));
        }
        assert!(
            !planes.is_empty(),
            "a validated role binds at least one listener; build_routers guarantees a router"
        );

        let mut bound: Vec<(tokio::net::TcpListener, axum::Router)> =
            Vec::with_capacity(planes.len());
        for (addr, router) in planes {
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            bound.push((listener, router));
        }

        // One signal anchors every listener's drain and the deadline watchdog:
        // each server gets its own clone so all of them observe the same
        // SIGTERM/ctrl-c instant (see the `shutdown` module docs).
        let signal = ShutdownSignal::spawn();
        let serve_future = {
            let mut handles = Vec::with_capacity(bound.len());
            for (listener, router) in bound {
                let graceful = signal.clone();
                handles.push(tokio::spawn(async move {
                    axum::serve(listener, router)
                        .with_graceful_shutdown(graceful.wait())
                        .await
                }));
            }
            async move {
                // Resolve only when every listener has finished draining, so
                // the drain-deadline watchdog bounds the whole process, not
                // just whichever plane happens to drain first.
                for handle in handles {
                    let served = handle
                        .await
                        .map_err(|err| std::io::Error::other(err.to_string()))?;
                    served?;
                }
                Ok::<(), std::io::Error>(())
            }
        };
        let deadline = std::time::Duration::from_secs(SHUTDOWN_DRAIN_DEADLINE_SECS);

        match shutdown::run_with_drain_deadline(serve_future, signal, deadline).await {
            DrainOutcome::Completed(Ok(())) => tracing::info!("server exited cleanly after drain"),
            DrainOutcome::Completed(Err(err)) => return Err(err.into()),
            DrainOutcome::DeadlineExceeded => {
                tracing::warn!(
                    deadline_secs = SHUTDOWN_DRAIN_DEADLINE_SECS,
                    "shutdown drain deadline exceeded; aborting stragglers and exiting"
                );
            }
        }
    }

    Ok(())
}
