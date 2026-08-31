//! `init_telemetry` is host-respecting: a global dispatcher installed by a
//! host application before the call is retained, the call still returns `Ok`,
//! and the exporter fallback warning — which describes a subscriber *this
//! call* installed — is not emitted through the host's subscriber.
//!
//! A global tracing dispatcher is process-wide state, so this scenario lives
//! in its own integration binary and is deliberately its *sole* test. Under
//! `cargo nextest` every test gets its own process, but under plain
//! `cargo test` all tests in one binary share a process; a sibling test racing
//! for the global dispatcher would make the suite order-dependent.

use std::io::Write;
use std::sync::{Arc, Mutex};

use oidc_exchange::telemetry::init_telemetry;
use oidc_exchange_core::config::{TelemetryConfig, TelemetryExporter};

/// A clonable in-memory writer the host's fmt subscriber renders into, so the
/// test asserts on exactly what the host subscriber emitted.
#[derive(Clone, Default)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl SharedBuffer {
    fn rendered(&self) -> String {
        let bytes = self
            .0
            .lock()
            .expect("capture mutex must not be poisoned")
            .clone();
        String::from_utf8(bytes).expect("captured telemetry is utf-8")
    }
}

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("capture mutex must not be poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// With a host-owned global subscriber already installed, `init_telemetry`
/// returns `Ok`, emits the debug retention note through the *host's*
/// subscriber, and — even for an `otlp` config that would warn on the
/// installed path — emits no exporter fallback warning (negative space for the
/// warning's move inside the installed-path branch).
#[test]
fn init_telemetry_retains_host_subscriber_and_skips_fallback_warning() {
    // The host application's own subscriber: captures everything at DEBUG and
    // above into an in-memory buffer. DEBUG must be enabled so the retention
    // note is observable rather than filtered out.
    let buffer = SharedBuffer::default();
    let writer = buffer.clone();
    let host = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_global_default(host)
        .expect("the host subscriber must be the first global dispatcher in this process");
    assert!(
        tracing::dispatcher::has_been_set(),
        "precondition: the host's global dispatcher must be installed before init_telemetry"
    );

    // An enabled `otlp` config is the strongest probe: on the installed path
    // it *would* emit the fallback warning, so its absence below proves the
    // warning is skipped on the retained path rather than merely unclassified.
    let config = TelemetryConfig {
        enabled: true,
        exporter: TelemetryExporter::Otlp,
        endpoint: None,
        service_name: None,
        sample_rate: None,
        protocol: None,
    };

    let result = init_telemetry(&config);
    assert!(
        result.is_ok(),
        "init_telemetry must respect the host's dispatcher and succeed: {:?}",
        result.err()
    );

    let rendered = buffer.rendered();
    assert!(
        rendered.contains("retaining the existing subscriber"),
        "the debug retention note must flow through the host's subscriber, got: {rendered:?}"
    );
    assert!(
        !rendered.contains("falling back to stdout JSON"),
        "the exporter fallback warning describes a subscriber this call installed; on the \
         retained path nothing was installed, so it must not be emitted, got: {rendered:?}"
    );
}
