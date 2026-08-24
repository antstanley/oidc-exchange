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

    // 3. Build service and router. The service is shared: the router's
    // `AppState` holds one `Arc` clone and — under a long-lived runtime — the
    // session reaper holds another, so both observe one store/audit/provider
    // set (`04-http-api.md` → Bootstrap steps 4–5 and 7).
    let service = Arc::new(bootstrap::build_service(&config).await?);
    let app = bootstrap::build_router_shared(&config, Arc::clone(&service));

    // 4. Run. One runtime detection feeds both the serve-mode branch and the
    // reaper's host gate, so "Lambda spawns no in-process interval" cannot
    // drift from "Lambda serves via lambda_http".
    let host_runtime = HostRuntime::detect();
    if host_runtime == HostRuntime::Lambda {
        // Lambda mode: the same router (middleware, state, and base-path layer) is served
        // through `lambda_http`, which speaks the Lambda Runtime API directly and translates
        // API Gateway REST/HTTP-API, Function URL, and ALB events into tower `Service` calls
        // against `app` (`04-http-api.md` → Bootstrap, step 6). `lambda::run_lambda` wraps
        // `app` in the per-invocation flush hook (`FlushOnResponse`) so telemetry (and any
        // future buffered audit writes) force-flush synchronously after each invocation's
        // response future resolves and before the response is returned — the execution
        // environment may freeze immediately after the response. `lambda_http::run` fails with
        // `lambda_http::Error` (`Box<dyn std::error::Error + Send + Sync>`), which the
        // standard library does not blanket-convert into `main`'s `Box<dyn std::error::Error>`
        // (the `Send + Sync` marker traits make the two boxed trait objects distinct types to
        // `?`'s `From` resolution); the error message is preserved and reboxed so the failure
        // still propagates via `?` with no `unwrap`/`expect` on this path.
        //
        // The session reaper is deliberately not spawned here: there is no long-lived process
        // to host it, and the same sweep stays reachable as `POST /internal/sessions/cleanup`
        // for an external scheduler to drive on the deployment's own cadence
        // (`04-http-api.md` → Bootstrap step 7).
        tracing::info!("Lambda runtime detected; serving via lambda_http");
        lambda::run_lambda(app, Arc::new(telemetry::flush_telemetry))
            .await
            .map_err(|err| -> Box<dyn std::error::Error> { err.to_string().into() })?;
    } else {
        let addr = format!("{}:{}", config.server.host, config.server.port);
        assert!(
            !addr.is_empty(),
            "bind address must not be empty before serving"
        );
        tracing::info!(addr = %addr, "starting server");
        let listener = tokio::net::TcpListener::bind(&addr).await?;

        // `signal` is spawned once, up front, and cloned so the graceful-shutdown hook below,
        // `run_with_drain_deadline`'s watchdog, and the session reaper all observe the *same*
        // SIGTERM/ctrl-c instant — the drain deadline must be anchored to when the signal
        // fires, not to process startup (see `shutdown` module docs), and the reaper must stop
        // on that same instant rather than ticking past shutdown.
        let signal = ShutdownSignal::spawn();

        // Bootstrap step 7: this hyper runtime is long-lived, so it hosts the session reaper —
        // one sweep of expired sessions and retirement records per
        // `session_repository.cleanup_interval`, each run logged with its deleted count. The
        // handle is retained and aborted once the drain finishes below; nothing detached
        // outlives the server.
        let reaper_handle = reaper::spawn_session_reaper_for_runtime(
            &config,
            &service,
            signal.clone(),
            host_runtime,
        );

        let graceful_signal = signal.clone();
        let serve_future = async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(graceful_signal.wait())
            .await
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
