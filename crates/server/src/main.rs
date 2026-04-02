use oidc_exchange::bootstrap;
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
        // Lambda mode — not yet implemented
        tracing::info!("Lambda runtime detected, but not yet implemented");
        // TODO: lambda_http::run(app)
    } else {
        let addr = format!("{}:{}", config.server.host, config.server.port);
        tracing::info!(addr = %addr, "starting server");
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app).await?;
    }

    Ok(())
}
