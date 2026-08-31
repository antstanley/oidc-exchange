//! A host-owned global subscriber installed *before* construction is
//! retained: construction succeeds against it, and a real request carrying an
//! invalid header name is answered while the host's subscriber captures the
//! deterministic `invalid request headers dropped at FFI boundary` warning —
//! pinning both host-respect and the end-to-end operator signal.
//!
//! This is its own test binary: a global tracing dispatcher is process-wide,
//! so this host-first scenario cannot share a process with the scenario where
//! the constructor performs the install.

use std::fmt;
use std::sync::{Arc, Mutex};

use oidc_exchange_ffi::{OidcExchange, TransportHints, WireRequest};
use tracing_subscriber::layer::{Context as LayerContext, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

#[derive(Default)]
struct EventVisitor(String);

impl tracing::field::Visit for EventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        use std::fmt::Write;
        let _ = write!(self.0, "{}={value:?};", field.name());
    }
}

/// Records every event's fields into a shared string — the "host application
/// owns its own subscriber" stand-in.
#[derive(Clone)]
struct EventCapture(Arc<Mutex<String>>);

impl<S> Layer<S> for EventCapture
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: LayerContext<'_, S>) {
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .expect("capture mutex must not be poisoned")
            .push_str(&visitor.0);
    }
}

/// The minimal admin-role SQLite config — the `embedder_tests` fixture shape.
fn minimal_admin_config(db_path: &std::path::Path) -> String {
    format!(
        "[server]\nissuer = \"https://auth.example.com\"\nrole = \"admin\"\n\n\
         [repository]\nadapter = \"sqlite\"\n\n\
         [repository.sqlite]\npath = \"{}\"\n",
        db_path.display()
    )
}

#[test]
fn host_subscriber_survives_construction_and_captures_the_boundary_warning() {
    let captured = Arc::new(Mutex::new(String::new()));
    let subscriber = tracing_subscriber::registry().with(EventCapture(captured.clone()));
    tracing::subscriber::set_global_default(subscriber)
        .expect("the host installs its global subscriber before construction");

    let dir = tempfile::tempdir().expect("tempdir");
    let exchange = OidcExchange::new(&minimal_admin_config(&dir.path().join("host.sqlite")))
        .expect("construction succeeds against a host-owned global subscriber");

    // A request carrying an invalid header name (a space is illegal in an
    // HTTP header name) is still answered: the header is dropped at the
    // boundary and the deterministic warning is emitted.
    let response = exchange
        .handle_blocking(WireRequest {
            method: "GET".to_string(),
            raw_path: b"/health".to_vec(),
            query: None,
            headers: vec![("invalid header name".to_string(), "value".to_string())],
            body: Vec::new(),
            hints: TransportHints { path_is_raw: true },
        })
        .expect("the request with a dropped header is still answered");
    assert_eq!(
        response.status, 200,
        "/health answers despite the dropped invalid header"
    );

    let logs = captured.lock().expect("capture mutex must not be poisoned");
    assert!(
        logs.contains("invalid request headers dropped at FFI boundary"),
        "the host's subscriber must capture the FFI boundary warning; captured: {logs}"
    );
}
