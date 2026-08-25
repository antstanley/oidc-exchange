use std::sync::Arc;

use oidc_exchange::bootstrap;
use oidc_exchange::lambda;
use oidc_exchange::reaper::{self, HostRuntime};
use oidc_exchange::shutdown::{self, DrainOutcome, ShutdownSignal, SHUTDOWN_DRAIN_DEADLINE_SECS};
use oidc_exchange::telemetry;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("oidc-exchange {VERSION}");
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("config") {
        return run_config_command(&args[1..]);
    }
    if !args.is_empty() {
        return Err(format!("unrecognized arguments: {}", args.join(" ")).into());
    }

    // 1. Load config
    let config = bootstrap::load_config()?;

    // 2. Init telemetry
    telemetry::init_telemetry(&config.telemetry)?;

    tracing::info!("configuration loaded");

    let role = config.server.role.as_str();
    tracing::info!(role = %role, "server role");

    // 3. Build service and the per-plane routers the role requires. The
    // service is shared: each router's `AppState` holds one `Arc` clone and —
    // under a long-lived runtime — the session reaper holds another, so all
    // observe one store/audit/provider set (`04-http-api.md` → Bootstrap
    // steps 4–5 and 7).
    let service = Arc::new(bootstrap::build_service(&config).await?);
    let routers = bootstrap::build_routers_shared(&config, Arc::clone(&service))?;

    if config.server.role == oidc_exchange_core::config::ServerRole::All {
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

    // 4. Run. One runtime detection feeds both the serve-mode branch and the
    // reaper's host gate, so "Lambda spawns no in-process interval" cannot
    // drift from "Lambda serves via lambda_http".
    let host_runtime = HostRuntime::detect();
    if host_runtime == HostRuntime::Lambda {
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
        //
        // The session reaper is deliberately not spawned here: there is no
        // long-lived process to host it, and the same sweep stays reachable as
        // `POST /internal/sessions/cleanup` for an external scheduler to drive
        // on the deployment's own cadence (`04-http-api.md` → Bootstrap step 7).
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

        // One signal anchors every listener's drain, the deadline watchdog,
        // and the session reaper: each gets its own clone so all of them
        // observe the same SIGTERM/ctrl-c instant (see the `shutdown` module
        // docs) — and the reaper stops on that same instant rather than
        // ticking past shutdown.
        //
        // Both planes serve through the connect-info make-service so the
        // client-address middleware — and the admin plane's operator-auth
        // throttle key — keep a real socket peer on every request.
        let signal = ShutdownSignal::spawn();
        // Bootstrap step 7: this hyper runtime is long-lived, so it hosts the
        // session reaper — one sweep of expired sessions and retirement
        // records per `session_repository.cleanup_interval`, each run logged
        // with its deleted count. The handle is retained and aborted once the
        // drain finishes below; nothing detached outlives the server.
        let reaper_handle = reaper::spawn_session_reaper_for_runtime(
            &config,
            &service,
            signal.clone(),
            host_runtime,
        );

        let serve_future = {
            let mut handles = Vec::with_capacity(bound.len());
            for (listener, router) in bound {
                let graceful = signal.clone();
                handles.push(tokio::spawn(async move {
                    axum::serve(
                        listener,
                        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                    )
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

        let outcome = shutdown::run_with_drain_deadline(serve_future, signal, deadline).await;

        // Backstop behind the loop's own shutdown exit, run on *every* path
        // out of the drain (including the error arm's early return below):
        // abort whatever sweep the reaper was mid-flight on when the signal
        // fired. Dropping a `JoinHandle` would detach its task instead, so
        // this explicit abort — no-op on an already-finished task — is what
        // guarantees no reaper survives server shutdown.
        if let Some(handle) = reaper_handle {
            handle.abort();
        }

        match outcome {
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

/// Run preflight-only configuration commands without initializing telemetry,
/// building adapters, binding sockets, or writing state.
fn run_config_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.first().map(String::as_str) != Some("check") {
        return Err(
            "usage: oidc-exchange config check [<path>] [--dir <config-dir>] [--file <path>]"
                .into(),
        );
    }

    // `config check <path>` (no flag) checks that single fully-materialized
    // file against the committed defaults without consulting the environment;
    // `--dir`/`--file` run the environment-aware server and FFI layerings.
    if let [path] = &args[1..] {
        if !path.starts_with("--") {
            let config = bootstrap::check_config_file(path)?;
            println!("{}", bootstrap::render_checked_config(&config));
            return Ok(());
        }
    }

    let mut dir: Option<&str> = None;
    let mut file: Option<&str> = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--dir" => {
                index += 1;
                dir = Some(args.get(index).ok_or("--dir requires a path")?);
            }
            "--file" => {
                index += 1;
                file = Some(args.get(index).ok_or("--file requires a path")?);
            }
            value => return Err(format!("unknown config check argument: {value}").into()),
        }
        index += 1;
    }
    if dir.is_some() && file.is_some() {
        return Err("--dir and --file are mutually exclusive".into());
    }

    let config = match (dir, file) {
        (Some(path), None) => bootstrap::load_config_from_dir(path)?,
        (None, Some(path)) => bootstrap::load_config_from_file(path)?,
        (None, None) => bootstrap::load_config()?,
        (Some(_), Some(_)) => unreachable!("validated mutually exclusive options"),
    };
    println!("{}", bootstrap::render_checked_config(&config));
    Ok(())
}
