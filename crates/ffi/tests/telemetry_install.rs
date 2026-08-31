//! Constructing an `OidcExchange` installs the process-wide telemetry
//! subscriber, and a second construction in the same process neither panics
//! nor fails — pinning `try_init` idempotency across instances.
//!
//! This is its own test binary: a global tracing dispatcher is process-wide,
//! so each install scenario needs its own process. The single test function
//! keeps the whole scenario (no dispatcher → first install → second retained)
//! ordered deterministically even under harnesses that share a process
//! between tests.

use oidc_exchange_ffi::OidcExchange;

/// The minimal admin-role SQLite config — the `embedder_tests` fixture shape.
fn minimal_admin_config(db_path: &std::path::Path) -> String {
    format!(
        "[server]\nissuer = \"https://auth.example.com\"\nrole = \"admin\"\n\n\
         [repository]\nadapter = \"sqlite\"\n\n\
         [repository.sqlite]\npath = \"{}\"\n",
        db_path.display()
    )
}

fn assert_serves_health(exchange: &OidcExchange, which: &str) {
    // The deprecated synchronous entry point is the simplest blocking route
    // to the router; it stays supported for one major cycle.
    #[allow(deprecated)]
    let response = exchange
        .handle_request("GET", "/health", Vec::new(), Vec::new())
        .expect("health request routes");
    assert_eq!(response.status, 200, "the {which} instance serves /health");
}

#[test]
fn construction_installs_the_subscriber_and_a_second_instance_still_serves() {
    let dir = tempfile::tempdir().expect("tempdir");

    assert!(
        !tracing::dispatcher::has_been_set(),
        "precondition: this test binary must start with no global dispatcher, \
         or the install below cannot be attributed to the constructor"
    );

    let first = OidcExchange::new(&minimal_admin_config(&dir.path().join("first.sqlite")))
        .expect("the first instance constructs");
    assert!(
        tracing::dispatcher::has_been_set(),
        "constructing an OidcExchange must install the process-wide telemetry subscriber"
    );
    assert_serves_health(&first, "first");

    // The second construction finds the global dispatcher already set;
    // `init_telemetry`'s `try_init` idempotency means it neither panics nor
    // fails, and the instance is fully servable.
    let second = OidcExchange::new(&minimal_admin_config(&dir.path().join("second.sqlite")))
        .expect("a second instance constructs without error once the subscriber is installed");
    assert_serves_health(&second, "second");
}
