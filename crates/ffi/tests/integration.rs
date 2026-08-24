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

    // HTTP method tokens cannot contain spaces; this triggers INVALID_METHOD.
    match exchange.handle_request("NOT A METHOD", "/health", vec![], vec![]) {
        Err(err) => assert_eq!(err.code, "INVALID_METHOD"),
        Ok(_) => panic!("expected error for invalid method"),
    }
}
