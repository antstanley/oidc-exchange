//! `init_telemetry` is idempotent: the double-init panic is unrepresentable.
//!
//! A global tracing dispatcher is process-wide state, so this scenario lives in
//! its own integration binary rather than a unit test in `telemetry.rs` — and
//! it is deliberately the *sole* test in this binary. Under `cargo nextest`
//! every test gets its own process, but under plain `cargo test` all tests in
//! one binary share a process; a sibling test racing for the global dispatcher
//! would make the suite order-dependent.

use oidc_exchange::telemetry::init_telemetry;
use oidc_exchange_core::config::{TelemetryConfig, TelemetryExporter};

/// A minimal disabled config: the exporter field is ignored when `enabled` is
/// `false`, so the first call installs the plain JSON subscriber and warns
/// about nothing.
fn minimal_config() -> TelemetryConfig {
    TelemetryConfig {
        enabled: false,
        exporter: TelemetryExporter::None,
        endpoint: None,
        service_name: None,
        sample_rate: None,
        protocol: None,
    }
}

/// Calling `init_telemetry` twice in one process returns `Ok` both times: the
/// first call installs the global dispatcher, the second finds it already set
/// and retains it instead of panicking (the old `.init()` aborted the process
/// here).
#[test]
fn init_telemetry_twice_returns_ok_both_times() {
    let config = minimal_config();

    let first = init_telemetry(&config);
    assert!(
        first.is_ok(),
        "first init_telemetry call must install the subscriber and succeed: {:?}",
        first.err()
    );
    assert!(
        tracing::dispatcher::has_been_set(),
        "the first successful call must have installed the global dispatcher"
    );

    let second = init_telemetry(&config);
    assert!(
        second.is_ok(),
        "second init_telemetry call must retain the installed subscriber and succeed \
         instead of panicking: {:?}",
        second.err()
    );
    assert!(
        tracing::dispatcher::has_been_set(),
        "the retained path must leave the global dispatcher installed"
    );
}
