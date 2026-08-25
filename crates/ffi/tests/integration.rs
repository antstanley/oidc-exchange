#![allow(deprecated)]

use std::process::Command;

use oidc_exchange_ffi::OidcExchange;
#[cfg(feature = "conformance")]
use oidc_exchange_ffi::{TransportHints, WireRequest};

#[cfg(feature = "conformance")]
fn wire(method: &str, raw_path: &[u8]) -> WireRequest {
    WireRequest {
        method: method.to_string(),
        raw_path: raw_path.to_vec(),
        query: None,
        headers: vec![],
        body: vec![],
        hints: TransportHints { path_is_raw: true },
    }
}

/// Generate an Ed25519 PEM key file at the given path using `openssl`.
fn setup_test_key(path: &std::path::Path) {
    let status = Command::new("openssl")
        .args(["genpkey", "-algorithm", "Ed25519", "-out"])
        .arg(path)
        .status()
        .expect("failed to run openssl");
    assert!(status.success(), "openssl genpkey failed");
}

/// Return a minimal TOML config string that uses sqlite, local key manager,
/// and noop audit.
fn minimal_config(key_path: &str, db_path: &str) -> String {
    format!(
        r#"
[server]
issuer = "https://auth.test.com"
role = "exchange"

[registration]
mode = "open"

[repository]
adapter = "sqlite"

[repository.sqlite]
path = "{db_path}"

[key_manager]
adapter = "local"

[key_manager.local]
private_key_path = "{key_path}"
algorithm = "EdDSA"
kid = "test-key-1"

[audit]
adapter = "noop"

[telemetry]
enabled = false
"#
    )
}

/// Per-test helper that creates a temp dir with the key and db, returning the
/// `OidcExchange` instance and the temp dir (so it stays alive for the test).
fn setup() -> (OidcExchange, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let key_path = tmp.path().join("test-key.pem");
    let db_path = tmp.path().join("test.db");

    setup_test_key(&key_path);

    let config = minimal_config(key_path.to_str().unwrap(), db_path.to_str().unwrap());

    let exchange = OidcExchange::new(&config).expect("failed to create OidcExchange");
    (exchange, tmp)
}

#[test]
fn test_health_endpoint() {
    let (exchange, _tmp) = setup();

    let resp = exchange
        .handle_request("GET", "/health", vec![], vec![])
        .expect("handle_request failed");

    assert_eq!(resp.status, 200);
}

#[test]
fn test_jwks_endpoint() {
    let (exchange, _tmp) = setup();

    let resp = exchange
        .handle_request("GET", "/keys", vec![], vec![])
        .expect("handle_request failed");

    assert_eq!(resp.status, 200);

    let body: serde_json::Value =
        serde_json::from_slice(&resp.body).expect("response body is not valid JSON");

    let keys = body.get("keys").expect("missing 'keys' field");
    let keys_arr = keys.as_array().expect("'keys' is not an array");
    assert!(!keys_arr.is_empty(), "keys array should not be empty");
}

#[test]
fn test_openid_discovery() {
    let (exchange, _tmp) = setup();

    #[allow(deprecated)]
    let resp = exchange
        .handle_request("GET", "/.well-known/openid-configuration", vec![], vec![])
        .expect("handle_request failed");

    assert_eq!(resp.status, 200);

    let body: serde_json::Value =
        serde_json::from_slice(&resp.body).expect("response body is not valid JSON");

    let issuer = body
        .get("issuer")
        .expect("missing 'issuer' field")
        .as_str()
        .expect("issuer is not a string");

    assert_eq!(issuer, "https://auth.test.com");
}

#[test]
fn test_invalid_config() {
    match OidcExchange::new("this is not valid toml {{{") {
        Err(err) => assert_eq!(err.code, "CONFIG_ERROR"),
        Ok(_) => panic!("expected error for invalid TOML"),
    }
}

/// Well-formed TOML with a semantically invalid field (an unknown
/// `server.role`) must be rejected by load-time validation at construction,
/// not merely at parse time, and never reach request handling.
#[test]
fn test_invalid_role_rejected_at_construction() {
    let config = r#"
[server]
issuer = "https://auth.test.com"
role = "exchang"

[registration]
mode = "open"
"#;

    match OidcExchange::new(config) {
        Err(err) => {
            assert_eq!(err.code, "CONFIG_ERROR");
            assert!(
                err.message.contains("role"),
                "error message should name the offending field, got: {}",
                err.message
            );
        }
        Ok(_) => panic!("expected CONFIG_ERROR for an invalid server.role"),
    }
}

/// The same validation applies through `from_file`: an invalid config on
/// disk is rejected before `OidcExchange` is constructed.
#[test]
fn test_invalid_config_rejected_via_from_file() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let config_path = tmp.path().join("bad-config.toml");
    std::fs::write(
        &config_path,
        r#"
[server]
role = "not-a-real-role"
"#,
    )
    .expect("failed to write config file");

    match OidcExchange::from_file(config_path.to_str().unwrap()) {
        Err(err) => assert_eq!(err.code, "CONFIG_ERROR"),
        Ok(_) => panic!("expected CONFIG_ERROR for an invalid server.role via from_file"),
    }
}

/// A valid config, by contrast, must still construct successfully — the
/// negative-space tests above only prove half the contract without this
/// counterpart.
#[test]
fn test_valid_config_constructs_successfully() {
    let (_exchange, _tmp) = setup();
}

/// File-backed construction uses the same resolver as string-backed
/// construction, so the same valid fixture is accepted by both entry points.
#[test]
fn test_valid_config_constructs_via_from_file() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let key_path = tmp.path().join("test-key.pem");
    let db_path = tmp.path().join("test.db");
    let config_path = tmp.path().join("valid-config.toml");
    setup_test_key(&key_path);
    std::fs::write(
        &config_path,
        minimal_config(
            key_path.to_str().expect("UTF-8 key path"),
            db_path.to_str().expect("UTF-8 database path"),
        ),
    )
    .expect("failed to write config file");

    let exchange = OidcExchange::from_file(config_path.to_str().expect("UTF-8 config path"));

    assert!(exchange.is_ok(), "valid file config must construct");
}

#[test]
fn test_invalid_method() {
    let (exchange, _tmp) = setup();

    #[allow(deprecated)]
    match exchange.handle_request("NOT A METHOD", "/health", vec![], vec![]) {
        Ok(resp) => assert_eq!(resp.status, 400),
        Err(err) => panic!("invalid method has HTTP meaning, got boundary error: {err}"),
    }
}

#[test]
#[cfg(feature = "conformance")]
fn async_wire_normalises_empty_path_and_separate_query() {
    let (exchange, _tmp) = setup();
    let mut request = wire("GET", b"");
    request.query = Some(b"check=1".to_vec());
    let response = exchange
        .runtime_handle_for_conformance(request)
        .expect("wire request must produce a response");
    assert_eq!(response.status, 404);
}

#[test]
#[cfg(feature = "conformance")]
fn async_wire_preserves_encoded_delimiters_as_path_data() {
    let (exchange, _tmp) = setup();
    for path in [b"/a%2Fb".as_slice(), b"/a%3Fb", b"/a%23b", b"/a/%2E%2E/b"] {
        let response = exchange
            .runtime_handle_for_conformance(wire("GET", path))
            .expect("encoded path must produce a response");
        assert_eq!(response.status, 404, "path={path:?}");
    }
}

#[test]
#[cfg(feature = "conformance")]
fn async_wire_rejects_non_origin_form_and_invalid_method_as_400() {
    let (exchange, _tmp) = setup();
    for mut request in [
        wire("GET", b"relative"),
        wire("GET", b"//authority/path"),
        wire("NOT A METHOD", b"/health"),
    ] {
        let response = exchange
            .runtime_handle_for_conformance(request.clone())
            .expect("shaping failure must be an HTTP response");
        assert_eq!(response.status, 400);
        request.raw_path = b"/health".to_vec();
    }
}

#[test]
#[cfg(feature = "conformance")]
fn async_wire_drops_invalid_headers_and_keeps_valid_duplicates() {
    let (exchange, _tmp) = setup();
    let mut request = wire("GET", b"/health");
    request.headers = vec![
        ("x-forwarded-for".into(), "192.0.2.1".into()),
        ("bad header".into(), "ignored".into()),
        ("x-forwarded-for".into(), "198.51.100.2".into()),
    ];
    let response = exchange
        .runtime_handle_for_conformance(request)
        .expect("invalid header must be dropped, not fatal");
    assert_eq!(response.status, 200);
}

#[test]
#[cfg(feature = "conformance")]
fn async_wire_enforces_published_body_limit_at_boundary() {
    let (exchange, _tmp) = setup();
    let limit = exchange.limits().max_body_bytes;
    assert_eq!(limit, 2 * 1024 * 1024);

    let mut at_limit = wire("POST", b"/token");
    at_limit.body = vec![b'x'; limit as usize];
    assert_ne!(
        exchange
            .runtime_handle_for_conformance(at_limit)
            .expect("at-limit body reaches router")
            .status,
        413
    );

    let mut over_limit = wire("POST", b"/token");
    over_limit.body = vec![b'x'; limit as usize + 1];
    assert_eq!(
        exchange
            .runtime_handle_for_conformance(over_limit)
            .expect("over-limit body yields response")
            .status,
        413
    );
}

#[test]
fn deprecated_shim_splits_only_first_query_delimiter() {
    let (exchange, _tmp) = setup();
    #[allow(deprecated)]
    let response = exchange
        .handle_request("GET", "/health?first=1?second=2", vec![], vec![])
        .expect("legacy shim must remain compatible");
    assert_eq!(response.status, 200);
}
