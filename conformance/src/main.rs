use std::io::{self, BufRead};
use std::process::Command;
use std::time::Duration;

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
        let deadline = Duration::from_secs(3);

        let mut stream = tokio::time::timeout(deadline, tokio::net::TcpStream::connect(address))
            .await
            .unwrap_or_else(|_| panic!("native transport timed out connecting for {}", input.id))
            .unwrap_or_else(|error| {
                panic!("native transport connect failed for {}: {error}", input.id)
            });
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
        let mut has_content_length = false;
        for header in tagged_headers(input.headers) {
            has_content_length |= header.0.eq_ignore_ascii_case("content-length");
            request.push_str(&format!("{}: {}\r\n", header.0, header.1));
        }
        if !has_content_length {
            request.push_str(&format!("Content-Length: {}\r\n", input.body_length));
        }
        request.push_str("Connection: close\r\n\r\n");
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        tokio::time::timeout(deadline, stream.write_all(request.as_bytes()))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "native transport timed out writing headers for {}",
                    input.id
                )
            })
            .unwrap_or_else(|error| {
                panic!(
                    "native transport header write failed for {}: {error}",
                    input.id
                )
            });
        let bounded_body_length = input.body_length.min(2_097_153);
        if bounded_body_length > 0 {
            let body = vec![b'x'; bounded_body_length];
            match tokio::time::timeout(deadline, stream.write_all(&body)).await {
                Ok(Ok(())) => {}
                Ok(Err(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                    ) => {}
                Ok(Err(error)) => panic!(
                    "native transport body write failed for {}: {error}",
                    input.id
                ),
                Err(_) => panic!("native transport timed out writing body for {}", input.id),
            }
        }
        if has_content_length {
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                let result = unsafe { libc::shutdown(stream.as_raw_fd(), libc::SHUT_WR) };
                if result != 0 {
                    let error = std::io::Error::last_os_error();
                    panic!(
                        "native transport half-close failed for {}: {error}",
                        input.id
                    );
                }
            }
            #[cfg(not(unix))]
            stream.shutdown().await.unwrap_or_else(|error| {
                panic!(
                    "native transport half-close failed for {}: {error}",
                    input.id
                )
            });
        }
        let mut bytes = Vec::new();
        tokio::time::timeout(deadline, stream.read_to_end(&mut bytes))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "native transport timed out awaiting response for {}",
                    input.id
                )
            })
            .unwrap_or_else(|error| {
                if error.kind() == std::io::ErrorKind::ConnectionReset {
                    0
                } else {
                    panic!(
                        "native transport response read failed for {}: {error}",
                        input.id
                    )
                }
            });
        server.abort();
        tokio::time::timeout(deadline, server)
            .await
            .unwrap_or_else(|_| panic!("native server task did not stop for {}", input.id))
            .ok();
        assert!(
            !bytes.is_empty(),
            "native transport returned no response for {}",
            input.id
        );
        let split = bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or_else(|| panic!("native response lacked header terminator for {}", input.id));
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
