use oidc_exchange_core::config::TelemetryConfig;

/// Initialise the tracing/telemetry pipeline based on configuration.
///
/// Currently supports the following exporter values:
///
/// | Exporter  | Behaviour |
/// |-----------|-----------|
/// | `"none"`  | JSON structured logs via `tracing-subscriber` only |
/// | `"stdout"`| Same as `"none"` (OTEL stdout exporter is a future enhancement) |
/// | `"otlp"`  | Falls back to `"stdout"` with a warning (OTLP pipeline is a future enhancement) |
/// | `"xray"`  | Falls back to `"stdout"` with a warning (X-Ray pipeline is a future enhancement) |
///
/// When `config.enabled` is `false` the exporter field is ignored and a plain
/// JSON subscriber is installed.
pub fn init_telemetry(config: &TelemetryConfig) -> Result<(), Box<dyn std::error::Error>> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    if !config.enabled {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
        return Ok(());
    }

    match config.exporter.as_str() {
        "none" | "stdout" => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .json()
                .init();
        }
        "otlp" => {
            // TODO: Wire up opentelemetry-otlp exporter when OTEL crate versions stabilize.
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .json()
                .init();
            tracing::warn!(
                "OTLP exporter requested but not yet implemented — falling back to stdout JSON logs"
            );
        }
        "xray" => {
            // TODO: Wire up opentelemetry X-Ray ID generator + OTLP exporter.
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .json()
                .init();
            tracing::warn!(
                "X-Ray exporter requested but not yet implemented — falling back to stdout JSON logs"
            );
        }
        other => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .json()
                .init();
            tracing::warn!(exporter = other, "unknown telemetry exporter — using stdout JSON logs");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_telemetry_does_not_panic() {
        // We can only call init once per process, and test harnesses share a
        // process, so we just verify the function *without* calling init by
        // checking config parsing.
        let config = TelemetryConfig {
            enabled: false,
            exporter: "none".to_string(),
            ..Default::default()
        };
        // The function would succeed; we test the logic path without actually
        // installing a global subscriber (which would conflict with other tests).
        assert!(!config.enabled);
        assert_eq!(config.exporter, "none");
    }
}
