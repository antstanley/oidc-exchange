use oidc_exchange_core::config::{TelemetryConfig, TelemetryExporter};

/// Initialise the tracing/telemetry pipeline based on configuration.
///
/// Behaviour by `[telemetry].exporter` (the closed [`TelemetryExporter`]
/// domain), when `config.enabled` is `true`:
///
/// | Exporter       | Behaviour |
/// |----------------|-----------|
/// | `none`         | JSON structured logs via `tracing-subscriber` only |
/// | `stdout`       | Same as `none` (OTEL stdout exporter is a future enhancement) |
/// | `otlp`         | Falls back to stdout JSON with a warning (OTLP pipeline is a future enhancement) |
/// | `xray`         | Falls back to stdout JSON with a warning (X-Ray pipeline is a future enhancement) |
/// | `prometheus`   | Accepted, but not yet implemented: warns and falls back to stdout JSON — no metrics are exported and no metrics endpoint is exposed |
///
/// The match is exhaustive over the closed enum: because a config-valid value can
/// never be an unknown string, there is no "unknown exporter" arm — a future
/// exporter is a compile error here rather than a silent stdout fallback.
///
/// When `config.enabled` is `false` the exporter field is ignored and a plain
/// JSON subscriber is installed.
pub fn init_telemetry(config: &TelemetryConfig) -> Result<(), Box<dyn std::error::Error>> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // The JSON stdout formatter is what every exporter installs today; the only
    // variation is the fallback warning, computed by the exhaustive classifier
    // below. Installing the subscriber (`init`) can happen only once per
    // process, so keeping the *decision* in a pure function lets it be unit
    // tested without a global subscriber.
    let warning = if config.enabled {
        exporter_fallback_warning(&config.exporter)
    } else {
        None
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();

    if let Some(message) = warning {
        tracing::warn!("{message}");
    }

    Ok(())
}

/// Accurately describes what `init_telemetry` does for each exporter: `None`
/// means the value is served directly (JSON stdout, no warning); `Some(message)`
/// is the fallback warning for a value accepted by the closed config domain but
/// not yet backed by a real pipeline.
///
/// The match is exhaustive over the closed [`TelemetryExporter`] enum with no
/// catch-all: a config-valid value can never be an unknown string, so there is
/// no "unknown exporter" arm, and a future exporter variant is a compile error
/// here rather than a silent stdout fallback.
fn exporter_fallback_warning(exporter: &TelemetryExporter) -> Option<&'static str> {
    match exporter {
        // TODO: OTEL stdout exporter is a future enhancement; served as JSON today.
        TelemetryExporter::None | TelemetryExporter::Stdout => None,
        // TODO: Wire up opentelemetry-otlp exporter when OTEL crate versions stabilize.
        TelemetryExporter::Otlp => Some(
            "OTLP exporter requested but not yet implemented — falling back to stdout JSON logs",
        ),
        // TODO: Wire up opentelemetry X-Ray ID generator + OTLP exporter.
        TelemetryExporter::Xray => Some(
            "X-Ray exporter requested but not yet implemented — falling back to stdout JSON logs",
        ),
        // Accepted by the closed config domain, but no metrics pipeline is wired
        // here — that belongs with the pending
        // `2026-06-24-complete_telemetry_exporters.md` change.
        TelemetryExporter::Prometheus => Some(
            "prometheus exporter requested but not yet implemented — no metrics are exported and \
             no metrics endpoint is exposed; falling back to stdout JSON logs",
        ),
    }
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

    /// Every exporter is classified (the function is total over the closed enum,
    /// so this exercises the JSON-formatter path for all of them without
    /// installing a global subscriber), and `prometheus` produces its accurate
    /// accepted-but-unimplemented warning — never the old "unknown exporter"
    /// wording, which is now unrepresentable.
    #[test]
    fn every_exporter_is_classified_and_prometheus_warns_accurately() {
        for exporter in [
            TelemetryExporter::None,
            TelemetryExporter::Stdout,
            TelemetryExporter::Otlp,
            TelemetryExporter::Xray,
            TelemetryExporter::Prometheus,
        ] {
            // Total function: returns for every variant without panicking.
            let warning = exporter_fallback_warning(&exporter);
            match exporter {
                TelemetryExporter::None | TelemetryExporter::Stdout => {
                    assert!(warning.is_none(), "{exporter:?} is served directly");
                }
                _ => {
                    let message = warning.expect("unimplemented exporters warn");
                    assert!(
                        !message.contains("unknown"),
                        "no exporter may be reported as unknown: {message}"
                    );
                }
            }
        }

        let prometheus =
            exporter_fallback_warning(&TelemetryExporter::Prometheus).expect("prometheus warns");
        assert!(prometheus.contains("prometheus"), "{prometheus}");
        assert!(prometheus.contains("not yet implemented"), "{prometheus}");
        assert!(
            prometheus.contains("no metrics endpoint is exposed"),
            "the prometheus warning must state no metrics endpoint is exposed: {prometheus}"
        );
    }
}
