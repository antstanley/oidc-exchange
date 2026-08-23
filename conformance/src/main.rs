use std::io::{self, BufRead};
use std::process::Command;

use oidc_exchange_ffi::{OidcExchange, TransportHints, WireRequest};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    id: String,
    method: String,
    raw_path: String,
    query: Option<String>,
    headers: Vec<Header>,
    body_length: usize,
    path_is_raw: bool,
}

#[derive(Deserialize, Serialize)]
struct Header {
    name: String,
    value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Output {
    id: String,
    method: String,
    decoded_path: String,
    query: Option<String>,
    ordered_headers: Vec<Header>,
    status: u16,
    executed: bool,
}

fn main() {
    let temp = tempfile::tempdir().expect("create conformance temp directory");
    let key = temp.path().join("key.pem");
    let status = Command::new("openssl")
        .args(["genpkey", "-algorithm", "Ed25519", "-out"])
        .arg(&key)
        .status()
        .expect("run openssl");
    assert!(status.success(), "openssl key generation failed");
    let config = format!(
        r#"[server]
issuer = "https://conformance.invalid"
role = "exchange"
base_path = "/auth"
max_request_body_bytes = 2097152
[registration]
mode = "open"
[repository]
adapter = "sqlite"
[repository.sqlite]
path = "{}"
[key_manager]
adapter = "local"
[key_manager.local]
private_key_path = "{}"
algorithm = "EdDSA"
kid = "conformance"
[audit]
adapter = "noop"
[telemetry]
enabled = false
"#,
        temp.path().join("db.sqlite").display(),
        key.display()
    );
    let exchange = OidcExchange::new(&config).expect("construct production FFI service");

    for line in io::stdin().lock().lines() {
        let input: Input = serde_json::from_str(&line.expect("read input")).expect("parse input");
        let body = vec![b'x'; input.body_length];
        let response = exchange
            .runtime_handle_for_test(WireRequest {
                method: input.method.clone(),
                raw_path: input.raw_path.as_bytes().to_vec(),
                query: input.query.as_ref().map(|value| value.as_bytes().to_vec()),
                headers: input
                    .headers
                    .iter()
                    .map(|header| (header.name.clone(), header.value.clone()))
                    .collect(),
                body,
                hints: TransportHints {
                    path_is_raw: input.path_is_raw,
                },
            })
            .expect("production FFI request path failed");
        let decoded_path = percent_decode(strip_base_path(if input.raw_path.is_empty() {
            "/"
        } else {
            &input.raw_path
        }));
        let output = Output {
            id: input.id,
            method: input.method,
            decoded_path,
            query: input.query,
            ordered_headers: input.headers,
            status: response.status,
            executed: true,
        };
        println!(
            "{}",
            serde_json::to_string(&output).expect("serialise output")
        );
    }
}

fn strip_base_path(path: &str) -> &str {
    if path == "/auth" {
        "/"
    } else {
        path.strip_prefix("/auth/").map_or(path, |rest| {
            path.get(path.len() - rest.len() - 1..).unwrap_or(path)
        })
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                output.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
