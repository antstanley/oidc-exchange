use std::process::Command;

use oidc_exchange_ffi::OidcExchange;

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

    let config = minimal_config(
        key_path.to_str().unwrap(),
        db_path.to_str().unwrap(),
    );

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

#[test]
fn test_invalid_method() {
    let (exchange, _tmp) = setup();

    match exchange.handle_request("NOTAMETHOD", "/health", vec![], vec![]) {
        Err(err) => assert_eq!(err.code, "INVALID_METHOD"),
        Ok(_) => panic!("expected error for invalid method"),
    }
}
