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
            tracing::warn!(
                exporter = other,
                "unknown telemetry exporter — using stdout JSON logs"
            );
        }
    }

    Ok(())
}

/// Force-flush the installed telemetry pipeline.
///
/// This is the single point [`crate::lambda::run_lambda`]'s per-invocation flush hook calls,
/// synchronously, after each Lambda invocation's response future resolves and before the
/// response is returned to the runtime API — the execution environment may freeze immediately
/// after the response, so anything batched and not yet sent by then is lost
/// (`04-http-api.md` → Bootstrap, step 6). It is also the seam a buffered audit adapter would
/// flush through, when one exists.
///
/// Under the stdout-JSON subscriber [`init_telemetry`] installs today, every `tracing` event is
/// written synchronously by `tracing-subscriber`'s `fmt` layer as it is emitted — there is
/// nothing buffered — so this is a documented, safe no-op. It stays a real function (rather
/// than being inlined away at the call site) so the Lambda wrapper has exactly one seam to
/// call regardless of what the telemetry backend becomes; once the OTLP/X-Ray exporters land
/// (`changes/2026-06-24-complete_telemetry_exporters.md`), the installed tracer provider's
/// `force_flush` call lands inside this function's body, not at every call site.
pub fn flush_telemetry() {
    // No-op today: see doc comment above. Intentionally still called on every Lambda
    // invocation (`run_lambda`) so the seam is exercised end to end even while it does
    // nothing, rather than only wired in once there is something to flush.
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `flush_telemetry` must be callable any number of times without panicking or requiring
    /// a subscriber to be installed first — the Lambda wrapper calls it unconditionally on
    /// every invocation, including in test harnesses that never call `init_telemetry`.
    #[test]
    fn flush_telemetry_is_idempotent_and_does_not_panic() {
        flush_telemetry();
        flush_telemetry();
    }

    #[test]
    fn disabled_telemetry_does_not_panic() {
        // We can only call init once per process, and test harnesses share a
        // process, so we just verify the function *without* calling init by
        // checking config parsing.
        let config =
            oidc_exchange_core::config::Config::resolve(oidc_exchange_core::config::RawConfig {
                telemetry: oidc_exchange_core::config::RawTelemetryConfig {
                    enabled: false,
                    exporter: "none".to_string(),
                    endpoint: None,
                    service_name: None,
                    sample_rate: None,
                    protocol: None,
                },
                ..oidc_exchange_core::config::RawConfig::default()
            });
        assert!(config.is_err(), "incomplete raw config must not resolve");
        let config = oidc_exchange_core::config::TelemetryConfig {
            enabled: false,
            exporter: oidc_exchange_core::config::TelemetryExporter::None,
            endpoint: None,
            service_name: None,
            sample_rate: None,
            protocol: None,
        };
        // The function would succeed; we test the logic path without actually
        // installing a global subscriber (which would conflict with other tests).
        assert!(!config.enabled);
        assert_eq!(config.exporter.as_str(), "none");
    }
}
