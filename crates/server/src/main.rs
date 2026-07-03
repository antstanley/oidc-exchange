use oidc_exchange::bootstrap;
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

    // 3. Build service and router
    let service = bootstrap::build_service(&config).await?;
    let app = bootstrap::build_router(&config, service);

    // 4. Run
    if std::env::var("AWS_LAMBDA_RUNTIME_API").is_ok() {
        // Lambda mode: the same router (middleware, state, and base-path layer) is served
        // through `lambda_http`, which speaks the Lambda Runtime API directly and translates
        // API Gateway REST/HTTP-API, Function URL, and ALB events into tower `Service` calls
        // against `app` (`04-http-api.md` → Bootstrap, step 6). `lambda_http::run` fails with
        // `lambda_http::Error` (`Box<dyn std::error::Error + Send + Sync>`), which the
        // standard library does not blanket-convert into `main`'s `Box<dyn std::error::Error>`
        // (the `Send + Sync` marker traits make the two boxed trait objects distinct types to
        // `?`'s `From` resolution); the error message is preserved and reboxed so the failure
        // still propagates via `?` with no `unwrap`/`expect` on this path.
        tracing::info!("Lambda runtime detected; serving via lambda_http");
        lambda_http::run(app)
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

        // `signal` is spawned once, up front, and cloned so the graceful-shutdown hook below
        // and `run_with_drain_deadline`'s watchdog observe the *same* SIGTERM/ctrl-c instant —
        // the drain deadline must be anchored to when the signal fires, not to process
        // startup (see `shutdown` module docs).
        let signal = ShutdownSignal::spawn();
        let graceful_signal = signal.clone();
        let serve_future = async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(graceful_signal.wait())
                .await
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
