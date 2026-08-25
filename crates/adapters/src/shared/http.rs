//! Process-wide shared HTTP client for all outbound provider calls (JWKS, discovery, token
//! endpoint, revocation).
//!
//! Every outbound call goes through this one `reqwest::Client` so a hung or slow provider
//! fails the request instead of stalling `/token` indefinitely: a bounded connect timeout, a
//! bounded total request timeout, and redirects disabled (providers serve these endpoints
//! directly; a redirect is more likely misconfiguration or attack than legitimate behaviour).

use std::sync::OnceLock;
use std::time::Duration;

use futures::StreamExt;
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::Secret;

/// Maximum time allowed to establish the TCP/TLS connection to a provider.
const CONNECT_TIMEOUT_SECS: u64 = 5;

/// Maximum total time allowed for the whole outbound request (connect + send + receive).
const REQUEST_TIMEOUT_SECS: u64 = 10;

/// Upper bound, in bytes, on how much of an upstream response body this process will
/// buffer (`64 KiB`). The cap exists because an upstream chooses its own body size:
/// reading unbounded text lets a hostile provider decide how much memory the service
/// retains and how much diagnostic text can later reach a log line.
pub const MAX_UPSTREAM_BODY_BYTES: usize = 65_536;

static SHARED_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Return the process-wide shared `reqwest::Client` used for every outbound provider call.
///
/// Built once, lazily, on first use: `connect_timeout` of [`CONNECT_TIMEOUT_SECS`], `timeout`
/// of [`REQUEST_TIMEOUT_SECS`], and redirects disabled via `redirect::Policy::none()`.
///
/// Production adapters never call this directly: every provider request goes
/// through [`crate::shared::transport::ProviderTransport`], which owns the
/// status-before-body ordering and the bounded reads. The webhook adapter
/// builds its own operator-timeout client and deliberately does not use this
/// one.
pub fn client() -> &'static reqwest::Client {
    SHARED_CLIENT.get_or_init(build_client)
}

fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build shared reqwest client")
}

/// Read an upstream response body under the [`MAX_UPSTREAM_BODY_BYTES`] ceiling and
/// return it wrapped as a [`Secret<String>`], so whatever a provider sent can be
/// consumed by the audited redaction path but never formatted directly.
///
/// Explicit behavior at the edges:
///
/// - *Oversize body* — streaming stops at the ceiling; only the first
///   [`MAX_UPSTREAM_BODY_BYTES`] bytes are retained (so an upstream cannot choose how
///   many bytes the process buffers), and reaching the limit is observable: a
///   structured `warn!` naming the provider and the limit is emitted. The retained,
///   possibly cut mid-character bytes are converted lossily to UTF-8 — never a panic.
/// - *Read failure* — the stream errors partway (connection reset, truncated body): the
///   partial bytes are dropped, not published, and a `ProviderError` with a generic
///   detail plus the transport error string (which never carries body content) is
///   returned.
///
/// `provider` labels the observability events and any error; it is never derived from
/// body content.
pub async fn read_bounded(provider: &str, response: reqwest::Response) -> Result<Secret<String>> {
    assert!(
        !provider.is_empty(),
        "provider label must not be empty for bounded reads"
    );

    // Precondition: callers read bodies only after inspecting the status; nothing here
    // depends on success or failure, so no status assert beyond the label above.

    let mut buffered: Vec<u8> = Vec::with_capacity(1024.min(MAX_UPSTREAM_BODY_BYTES));
    let mut truncated = false;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| Error::ProviderError {
            provider: provider.to_string(),
            detail: format!("failed while reading upstream response body: {e}"),
        })?;
        if buffered.len() + chunk.len() > MAX_UPSTREAM_BODY_BYTES {
            // Take only what fits under the ceiling, then stop consuming: the rest of
            // the stream is dropped along with the response, releasing the connection.
            let remaining = MAX_UPSTREAM_BODY_BYTES - buffered.len();
            buffered.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        buffered.extend_from_slice(&chunk);
    }
    // Postcondition: the buffer can never exceed the named ceiling, truncated or not.
    assert!(
        buffered.len() <= MAX_UPSTREAM_BODY_BYTES,
        "bounded read exceeded MAX_UPSTREAM_BODY_BYTES"
    );

    if truncated {
        // Reaching a limit is an observable event (development guidelines §Limits):
        // log it structured, without any body content, and keep serving the truncated
        // remainder to the caller — for diagnostics the excerpt path bounds it further.
        tracing::warn!(
            provider = %provider,
            limit_bytes = MAX_UPSTREAM_BODY_BYTES,
            "upstream response body exceeded the read ceiling and was truncated"
        );
    }

    Ok(Secret::new(String::from_utf8_lossy(&buffered).into_owned()))
}


/// Why reading an upstream body through the [`MAX_UPSTREAM_BODY_BYTES`]
/// ceiling *fail-closed* failed. See [`read_bounded_bytes`].
#[derive(Debug)]
pub enum BoundedBodyError {
    /// The body reached the ceiling. Reported *at* the limit, not after it,
    /// so an oversized response costs bounded memory, and the error names the
    /// limit so it is alertable as a provider fault rather than looking like
    /// a parse failure.
    OverLimit { limit_bytes: usize },
    /// The connection broke or timed out mid-body.
    Network(reqwest::Error),
}

impl std::fmt::Display for BoundedBodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverLimit { limit_bytes } => {
                write!(f, "response body reached the {limit_bytes}-byte ceiling")
            }
            Self::Network(e) => write!(f, "reading response body failed: {e}"),
        }
    }
}

impl std::error::Error for BoundedBodyError {}

/// Read an upstream response body through the [`MAX_UPSTREAM_BODY_BYTES`]
/// ceiling, **failing at the limit** rather than truncating.
///
/// This is the success-body counterpart to [`read_bounded`]: a diagnostic
/// error body can be truncated and still be useful, but a truncated JWKS or
/// discovery document is garbage that must not be parsed, so oversize here is
/// an error, not a prefix. Two bounds apply before any byte is buffered: an
/// honest `Content-Length` above the ceiling aborts immediately without
/// reading the body, and a streamed (or lying-header) body aborts mid-stream
/// the moment the running total would exceed the ceiling. Callers must have
/// inspected the response status *before* invoking this — the
/// status-before-body ordering lives in the transport, not here.
pub async fn read_bounded_bytes(
    mut response: reqwest::Response,
) -> std::result::Result<Vec<u8>, BoundedBodyError> {
    // An honest Content-Length lets us refuse without reading a byte.
    if let Some(declared) = response.content_length() {
        if declared > MAX_UPSTREAM_BODY_BYTES as u64 {
            return Err(BoundedBodyError::OverLimit {
                limit_bytes: MAX_UPSTREAM_BODY_BYTES,
            });
        }
    }

    let mut buffer = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(BoundedBodyError::Network)? {
        let new_len = buffer.len() + chunk.len();
        if new_len > MAX_UPSTREAM_BODY_BYTES {
            return Err(BoundedBodyError::OverLimit {
                limit_bytes: MAX_UPSTREAM_BODY_BYTES,
            });
        }
        buffer.extend_from_slice(&chunk);
    }

    assert!(
        buffer.len() <= MAX_UPSTREAM_BODY_BYTES,
        "bounded read returned more than MAX_UPSTREAM_BODY_BYTES bytes"
    );

    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn client_returns_same_instance_across_calls() {
        let a = client() as *const reqwest::Client;
        let b = client() as *const reqwest::Client;
        assert_eq!(a, b, "client() must return the same process-wide instance");
    }

    #[tokio::test]
    async fn delayed_response_past_total_timeout_fails_the_call() {
        let server = MockServer::start().await;

        // Delay well past REQUEST_TIMEOUT_SECS so the shared client's total timeout fires.
        let delay = Duration::from_secs(REQUEST_TIMEOUT_SECS + 5);

        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(delay))
            .mount(&server)
            .await;

        let result = client().get(format!("{}/slow", server.uri())).send().await;

        let err = result.expect_err("a delayed response must fail, not hang or succeed");
        assert!(err.is_timeout(), "expected a timeout error, got: {err:?}");
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/redirect-me"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/target"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/target"))
            .respond_with(ResponseTemplate::new(200).set_body_string("should not get here"))
            .mount(&server)
            .await;

        let response = client()
            .get(format!("{}/redirect-me", server.uri()))
            .send()
            .await
            .expect("the request itself should succeed, just not follow the redirect");

        assert_eq!(response.status(), 302);
        assert_eq!(
            response
                .headers()
                .get("location")
                .map(|v| v.to_str().unwrap()),
            Some("/target")
        );
    }

    // -------------------------------------------------------------------
    // read_bounded
    // -------------------------------------------------------------------

    /// Fetch the given body from a mock server and run [`read_bounded`] over it.
    async fn read_body_from_server(body: Vec<u8>) -> Result<Secret<String>> {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/body"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(body.clone())
                    .insert_header("content-type", "application/octet-stream"),
            )
            .mount(&server)
            .await;

        let response = client()
            .get(format!("{}/body", server.uri()))
            .send()
            .await
            .expect("the request itself must succeed");

        read_bounded("test-provider", response).await
    }

    #[tokio::test]
    async fn small_body_round_trips_through_the_secret_wrap() {
        let body = b"small diagnostic body".to_vec();
        let secret = read_body_from_server(body.clone())
            .await
            .expect("a small body must read successfully");
        assert_eq!(
            secret.expose(),
            &String::from_utf8(body).unwrap(),
            "read_bounded must preserve the exact bytes of an in-ceiling body"
        );
    }

    #[tokio::test]
    async fn body_exactly_at_the_ceiling_is_not_truncated() {
        let body = vec![b'a'; MAX_UPSTREAM_BODY_BYTES];
        let secret = read_body_from_server(body.clone())
            .await
            .expect("an exactly-at-ceiling body must read successfully");
        assert_eq!(
            secret.expose().len(),
            MAX_UPSTREAM_BODY_BYTES,
            "a body at the ceiling fits entirely under the bound"
        );
        assert!(
            secret.expose().ends_with('a'),
            "the tail byte must survive when nothing is truncated"
        );
    }

    #[tokio::test]
    async fn body_one_byte_past_the_ceiling_is_truncated_to_the_ceiling() {
        let body = vec![b'b'; MAX_UPSTREAM_BODY_BYTES + 1];
        let secret = read_body_from_server(body)
            .await
            .expect("an oversize body still yields the truncated prefix, not an error");
        assert_eq!(
            secret.expose().len(),
            MAX_UPSTREAM_BODY_BYTES,
            "retention must stop exactly at MAX_UPSTREAM_BODY_BYTES"
        );
    }

    #[tokio::test]
    async fn non_utf8_body_is_converted_lossily_without_panicking() {
        // Invalid UTF-8: a lone continuation byte and a cut multi-byte sequence.
        let body = vec![0xff, 0xfe, b'o', b'k', 0xE2, 0x82];
        let secret = read_body_from_server(body)
            .await
            .expect("non-UTF-8 bodies must convert lossily, never fail or panic");
        assert!(
            secret.expose().contains("ok"),
            "the valid ASCII middle must survive lossy conversion, got {:?}",
            secret.expose()
        );
        assert!(
            secret.expose().contains('\u{fffd}'),
            "invalid sequences become replacement characters, got {:?}",
            secret.expose()
        );
    }

    #[tokio::test]
    async fn oversized_truncation_emits_a_structured_warn_event() {
        use tracing_subscriber::layer::{Context, Layer};
        use tracing_subscriber::prelude::*;

        struct WarnCapture(Arc<std::sync::Mutex<Vec<String>>>);

        impl<S> Layer<S> for WarnCapture
        where
            S: tracing::Subscriber,
        {
            fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
                if *event.metadata().level() == tracing::Level::WARN {
                    struct V(Vec<String>);
                    impl tracing::field::Visit for V {
                        fn record_debug(
                            &mut self,
                            field: &tracing::field::Field,
                            value: &dyn std::fmt::Debug,
                        ) {
                            self.0.push(format!("{}={:?}", field.name(), value));
                        }
                    }
                    let mut fields = V(Vec::new());
                    event.record(&mut fields);
                    self.0.lock().unwrap().push(fields.0.join(", "));
                }
            }
        }

        let captured: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let subscriber = tracing_subscriber::registry().with(WarnCapture(captured.clone()));

        let server = MockServer::start().await;
        let body = vec![b'c'; MAX_UPSTREAM_BODY_BYTES + 42];
        Mock::given(method("GET"))
            .and(path("/body"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;

        let response = client()
            .get(format!("{}/body", server.uri()))
            .send()
            .await
            .expect("the request itself must succeed");

        // Stays active for the whole async body below (single-threaded `#[tokio::test]`
        // runtime keeps every poll on this OS thread), so the truncation warn is
        // guaranteed to hit the capturing layer.
        let _gate = oidc_exchange_test_utils::telemetry::CAPTURE_GATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _guard = tracing::subscriber::set_default(subscriber);
        tracing::callsite::rebuild_interest_cache();

        let secret = read_bounded("warn-test-provider", response)
            .await
            .expect("oversize body must truncate, not fail");
        assert_eq!(secret.expose().len(), MAX_UPSTREAM_BODY_BYTES);

        let events = captured.lock().unwrap().clone();
        // Positive control: the truncation warn fired with the provider label and the
        // named limit — and negative space: no body bytes appear in any field value.
        assert_eq!(
            events.len(),
            1,
            "exactly one truncation warning must be emitted, got {events:?}"
        );
        let event = &events[0];
        assert!(
            event.contains("provider=") && event.contains("warn-test-provider"),
            "the warn must name the provider, got {event:?}"
        );
        assert!(
            event.contains(&format!("limit_bytes={MAX_UPSTREAM_BODY_BYTES}")),
            "the warn must carry the named ceiling, got {event:?}"
        );
    }

    // -------------------------------------------------------------------
    // read_bounded_bytes (fail-closed success-body reads)
    // -------------------------------------------------------------------

    /// Serve one HTTP/1.1 response with `Transfer-Encoding: chunked` from a raw
    /// TCP listener, writing `body` in `chunk_size` chunks.
    ///
    /// wiremock cannot force chunked transfer encoding, and the chunked path is
    /// exactly where an honest-Content-Length short-circuit does not apply, so
    /// the streaming bound needs a real chunked origin to be tested.
    async fn spawn_chunked_server(body: Vec<u8>, chunk_size: usize) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding an ephemeral port should work");
        let addr = listener.local_addr().expect("local_addr should resolve");

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("the test connects once");
            // Drain the request head; its contents are irrelevant to the response.
            let mut scratch = [0u8; 4096];
            loop {
                let read = socket.read(&mut scratch).await.expect("request readable");
                if read == 0 || scratch[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }

            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                      Transfer-Encoding: chunked\r\n\r\n",
                )
                .await
                .expect("response head writable");

            for chunk in body.chunks(chunk_size.max(1)) {
                socket
                    .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                    .await
                    .expect("chunk header writable");
                socket.write_all(chunk).await.expect("chunk writable");
                socket
                    .write_all(b"\r\n")
                    .await
                    .expect("chunk CRLF writable");
            }
            socket
                .write_all(b"0\r\n\r\n")
                .await
                .expect("terminal chunk writable");
        });

        format!("http://{addr}/chunked")
    }

    #[tokio::test]
    async fn bounded_read_accepts_body_at_exactly_the_limit() {
        let server = MockServer::start().await;

        // Boundary: exactly MAX bytes with an honest Content-Length must succeed,
        // so the ceiling is inclusive and the failure is strictly above it.
        let body = "x".repeat(MAX_UPSTREAM_BODY_BYTES);
        Mock::given(method("GET"))
            .and(path("/bounded"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;

        let response = client()
            .get(format!("{}/bounded", server.uri()))
            .send()
            .await
            .expect("request should succeed");
        assert_eq!(
            response.content_length(),
            Some(MAX_UPSTREAM_BODY_BYTES as u64),
            "fixture must carry an honest Content-Length at the limit"
        );

        let bytes = read_bounded_bytes(response)
            .await
            .expect("a body at exactly the ceiling fits");
        assert_eq!(bytes.len(), MAX_UPSTREAM_BODY_BYTES);
    }

    #[tokio::test]
    async fn bounded_read_rejects_oversized_body_with_honest_content_length_without_reading_it() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/toobig"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("y".repeat(MAX_UPSTREAM_BODY_BYTES + 1)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let response = client()
            .get(format!("{}/toobig", server.uri()))
            .send()
            .await
            .expect("request should succeed");

        let err = read_bounded_bytes(response)
            .await
            .expect_err("one byte over the honest Content-Length ceiling must fail");
        assert!(
            matches!(err, BoundedBodyError::OverLimit { limit_bytes } if limit_bytes == MAX_UPSTREAM_BODY_BYTES),
            "expected OverLimit naming the limit, got: {err:?}"
        );
        assert!(
            err.to_string()
                .contains(&MAX_UPSTREAM_BODY_BYTES.to_string()),
            "the error must name the limit: {err}"
        );
    }

    #[tokio::test]
    async fn bounded_read_rejects_oversized_chunked_body_midstream() {
        // Chunked transfer carries no Content-Length, so only the streaming
        // running-total check can bound it; the abort happens mid-stream, before
        // the sender finishes writing, which is what keeps memory bounded against
        // a lying or unbounded origin.
        let oversized = vec![b'a'; MAX_UPSTREAM_BODY_BYTES + 1024];
        let url = spawn_chunked_server(oversized, 4096).await;

        let response = client()
            .get(url)
            .send()
            .await
            .expect("request should reach the raw chunked server");
        assert!(
            response.content_length().is_none(),
            "a chunked response must arrive without a Content-Length"
        );

        let err = read_bounded_bytes(response)
            .await
            .expect_err("a chunked body over the ceiling must fail mid-stream");
        assert!(
            matches!(err, BoundedBodyError::OverLimit { .. }),
            "expected OverLimit on the streamed body, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn bounded_read_parses_a_small_chunked_body_successfully() {
        // Positive space: the chunked path must also *succeed* under the ceiling,
        // proving the mid-stream bound is about size, not about chunked encoding.
        let payload = br#"{"keys":[]}"#;
        let url = spawn_chunked_server(payload.to_vec(), 4).await;

        let response = client().get(url).send().await.expect("request succeeds");
        let bytes = read_bounded_bytes(response)
            .await
            .expect("a small chunked body fits well under the ceiling");

        let value: serde_json::Value =
            serde_json::from_slice(&bytes).expect("body should be the JSON we sent");
        assert_eq!(value["keys"], serde_json::json!([]));
    }
}
