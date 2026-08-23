use std::io::{self, BufRead};
use std::process::Command;

use oidc_exchange_ffi::{OidcExchange, TransportHints, WireRequest};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    id: String,
    method: String,
    #[serde(rename = "rawPath")]
    raw_path: String,
    query: Option<String>,
    headers: Vec<Header>,
    body_length: usize,
    path_is_raw: bool,
}

#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}

fn main() {
    let shape = std::env::args().nth(1).expect("shape argument");
    let config = test_config();
    let exchange = OidcExchange::new(&config).expect("construct production service");
    for line in io::stdin().lock().lines() {
        let input: Input = serde_json::from_str(&line.expect("read input")).expect("parse input");
        let output = match shape.as_str() {
            "ffi" => run_ffi(&exchange, input),
            "native" => run_native(&config, input),
            _ => panic!("unknown shape {shape}"),
        };
        println!("{}", serde_json::to_string(&output).unwrap());
    }
}

fn run_ffi(exchange: &OidcExchange, input: Input) -> Value {
    let response = exchange
        .runtime_handle_for_conformance(WireRequest {
            method: input.method,
            raw_path: input.raw_path.as_bytes().to_vec(),
            query: input.query.map(String::into_bytes),
            headers: tagged_headers(input.headers),
            body: vec![b'x'; input.body_length],
            hints: TransportHints {
                path_is_raw: input.path_is_raw,
            },
        })
        .expect("production FFI request failed");
    output(input.id, response.status, &response.body)
}

fn run_native(config: &str, input: Input) -> Value {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async move {
        let config = oidc_exchange::bootstrap::parse_config(config).unwrap();
        let service = oidc_exchange::bootstrap::build_service(&config)
            .await
            .unwrap();
        let router = oidc_exchange::bootstrap::build_router(&config, service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let mut request = format!(
            "{} {}{} HTTP/1.1\r\nHost: localhost\r\n",
            input.method,
            if input.raw_path.is_empty() {
                "/"
            } else {
                &input.raw_path
            },
            input
                .query
                .as_ref()
                .map(|q| format!("?{q}"))
                .unwrap_or_default()
        );
        for header in tagged_headers(input.headers) {
            request.push_str(&format!("{}: {}\r\n", header.0, header.1));
        }
        request.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            input.body_length
        ));
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream.write_all(request.as_bytes()).await.unwrap();
        if input.body_length > 0 {
            if let Err(error) = stream.write_all(&vec![b'x'; input.body_length]).await {
                if error.kind() != std::io::ErrorKind::ConnectionReset
                    && error.kind() != std::io::ErrorKind::BrokenPipe
                {
                    panic!("write native body: {error}");
                }
            }
        }
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 16 * 1024];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) => break,
                Ok(count) => bytes.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
                Err(error) => panic!("read native response: {error}"),
            }
        }
        server.abort();
        let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&bytes[..split]);
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        output(input.id, status, &bytes[split + 4..])
    })
}

fn tagged_headers(mut headers: Vec<Header>) -> Vec<(String, String)> {
    headers.push(Header {
        name: "x-oidc-conformance-observe".into(),
        value: "1".into(),
    });
    headers
        .into_iter()
        .map(|header| {
            let value = if header.name.eq_ignore_ascii_case("x-conformance-marker") {
                format!("boundary:{}", header.value)
            } else {
                header.value
            };
            (header.name, value)
        })
        .collect()
}

fn output(id: String, status: u16, body: &[u8]) -> Value {
    if status == 200 {
        let mut observed: Value = serde_json::from_slice(body).expect("observation JSON response");
        observed["id"] = json!(id);
        observed["executed"] = json!(true);
        observed
    } else {
        json!({"id": id, "status": status, "executed": true})
    }
}

fn test_config() -> String {
    let temp = tempfile::tempdir().unwrap().keep();
    let key = temp.join("key.pem");
    assert!(Command::new("openssl")
        .args(["genpkey", "-algorithm", "Ed25519", "-out"])
        .arg(&key)
        .status()
        .unwrap()
        .success());
    format!(
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
        temp.join("db.sqlite").display(),
        key.display()
    )
}
