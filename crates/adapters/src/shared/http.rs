//! Process-wide shared HTTP client for all outbound provider calls (JWKS, discovery, token
//! endpoint, revocation).
//!
//! Every outbound call goes through this one `reqwest::Client` so a hung or slow provider
//! fails the request instead of stalling `/token` indefinitely: a bounded connect timeout, a
//! bounded total request timeout, and redirects disabled (providers serve these endpoints
//! directly; a redirect is more likely misconfiguration or attack than legitimate behaviour).

use std::sync::OnceLock;
use std::time::Duration;

/// Maximum time allowed to establish the TCP/TLS connection to a provider.
const CONNECT_TIMEOUT_SECS: u64 = 5;

/// Maximum total time allowed for the whole outbound request (connect + send + receive).
const REQUEST_TIMEOUT_SECS: u64 = 10;

static SHARED_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Return the process-wide shared `reqwest::Client` used for every outbound provider call.
///
/// Built once, lazily, on first use: `connect_timeout` of [`CONNECT_TIMEOUT_SECS`], `timeout`
/// of [`REQUEST_TIMEOUT_SECS`], and redirects disabled via `redirect::Policy::none()`.
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

// ---------------------------------------------------------------------------
// Vendored prerequisite: HttpsUrl
//
// VENDORED PREREQUISITE — owned by sibling change
// `.specs/changes/2026-08-05-fail_closed_across_config_and_adapters.md`, which
// specifies the `HttpsUrl` scheme constraint on configured and discovered
// provider endpoints. That sibling change is not merged into this unstacked
// branch, so the outbound-boundary work pins the exact contract locally. The
// owning PR reconciles ownership: delete this copy or repoint imports at the
// sibling's type. Nothing here widens the sibling's contract.
// ---------------------------------------------------------------------------

/// An absolute `https://` URL, validated at construction.
///
/// The type exists so an endpoint that has not passed the scheme check cannot
/// be represented, let alone sent to: a plain `String` endpoint accepts
/// `http://` and typo'd schemes silently, and every call site re-deciding the
/// question is how the check drifts out of existence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsUrl {
    url: reqwest::Url,
}

/// Why a string failed [`HttpsUrl`] validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpsUrlError {
    /// The input did not parse as an acceptable absolute URL (unparseable,
    /// relative, or hostless — the `url` crate rejects hostless `https` URLs
    /// at parse time).
    NotAnAbsoluteUrl,
    /// The input parsed but its scheme was not `https`.
    SchemeNotHttps { actual_scheme: String },
}

impl std::fmt::Display for HttpsUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Messages describe the violation class, never echo the rejected input:
        // error strings end up in logs, and endpoints flow in from config and
        // from remote discovery documents.
        match self {
            Self::NotAnAbsoluteUrl => write!(f, "endpoint is not an absolute URL"),
            Self::SchemeNotHttps { actual_scheme } => {
                write!(f, "endpoint scheme must be https, found {actual_scheme:?}")
            }
        }
    }
}

impl std::error::Error for HttpsUrlError {}

impl HttpsUrl {
    /// Validate `input` as an absolute `https://` URL.
    ///
    /// Only the scheme constraint is enforced here; origin pinning of
    /// discovered endpoints belongs to the endpoint-origin work and is
    /// deliberately not folded in.
    pub fn parse(input: &str) -> Result<Self, HttpsUrlError> {
        let url = reqwest::Url::parse(input).map_err(|_| HttpsUrlError::NotAnAbsoluteUrl)?;

        assert!(
            !url.scheme().is_empty(),
            "a successfully parsed absolute URL always carries a scheme"
        );

        if url.scheme() != "https" {
            // The url crate lowercases schemes during parsing, so the comparison
            // above is the canonical form and `actual_scheme` is safe to report.
            return Err(HttpsUrlError::SchemeNotHttps {
                actual_scheme: url.scheme().to_string(),
            });
        }

        // The url crate rejects hostless special-scheme URLs (EmptyHost) at
        // parse time, so an https URL that parsed always names a host. The
        // assertion pins that invariant instead of silently trusting it: if the
        // parser's behaviour ever changes, this fails loudly at the boundary
        // rather than letting a hostless endpoint reach the network.
        assert!(
            url.host_str().is_some(),
            "the url crate guarantees a host for parsed https URLs"
        );

        Ok(Self { url })
    }

    /// The validated URL in its canonical string form.
    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    /// The validated URL in parsed form, ready to hand to an HTTP client.
    pub fn as_url(&self) -> &reqwest::Url {
        &self.url
    }
}

// ---------------------------------------------------------------------------
// Vendored prerequisite: bounded response-body accumulator
//
// VENDORED PREREQUISITE — owned by sibling change
// `.specs/changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md`,
// which specifies `http::read_bounded` / `MAX_UPSTREAM_BODY_BYTES` and routes
// the error-body call sites through them. That sibling change is not merged
// into this unstacked branch, so the success-body bounding work pins the exact
// contract locally under the source-specified name `read_bounded_bytes`. The
// owning PR reconciles ownership: delete this copy or repoint imports at the
// sibling's helper. Nothing here widens the sibling's contract.
// ---------------------------------------------------------------------------

/// Hard ceiling on any single upstream response body, in bytes (64 KiB).
///
/// Applies to success and failure bodies alike: a JWKS, a discovery document,
/// and an OAuth error document are all orders of magnitude smaller than this,
/// so anything larger is a provider fault, not data worth buffering.
pub const MAX_UPSTREAM_BODY_BYTES: u64 = 64 * 1024;

/// Why reading an upstream body through the [`MAX_UPSTREAM_BODY_BYTES`]
/// ceiling failed.
#[derive(Debug)]
pub enum BoundedBodyError {
    /// The body reached the ceiling. Reported *at* the limit, not after it,
    /// so an oversized response costs bounded memory, and the error names the
    /// limit so it is alertable as a provider fault rather than looking like
    /// a parse failure.
    OverLimit { limit_bytes: u64 },
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
/// ceiling, failing at the limit rather than after it.
///
/// Two bounds apply before any byte is buffered: an honest `Content-Length`
/// above the ceiling aborts immediately without reading the body, and a
/// streamed (or lying-header) body aborts mid-stream the moment the running
/// total would exceed the ceiling. Callers must have inspected the response
/// status *before* invoking this — the status-before-body ordering lives in
/// the transport, not here.
pub async fn read_bounded_bytes(
    mut response: reqwest::Response,
) -> Result<Vec<u8>, BoundedBodyError> {
    // An honest Content-Length lets us refuse without reading a byte.
    if let Some(declared) = response.content_length() {
        if declared > MAX_UPSTREAM_BODY_BYTES {
            return Err(BoundedBodyError::OverLimit {
                limit_bytes: MAX_UPSTREAM_BODY_BYTES,
            });
        }
    }

    let mut buffer = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(BoundedBodyError::Network)? {
        let new_len = buffer.len() as u64 + chunk.len() as u64;
        if new_len > MAX_UPSTREAM_BODY_BYTES {
            return Err(BoundedBodyError::OverLimit {
                limit_bytes: MAX_UPSTREAM_BODY_BYTES,
            });
        }
        buffer.extend_from_slice(&chunk);
    }

    assert!(
        buffer.len() as u64 <= MAX_UPSTREAM_BODY_BYTES,
        "bounded read returned more than MAX_UPSTREAM_BODY_BYTES bytes"
    );

    Ok(buffer)
}

#[cfg(test)]
mod tests {
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

    // -------------------------------------------------------------------------
    // HttpsUrl (vendored prerequisite — see the module comment above the type)
    // -------------------------------------------------------------------------

    #[test]
    fn https_url_accepts_a_valid_https_url() {
        let parsed = HttpsUrl::parse("https://accounts.google.com/o/oauth2/v2/auth")
            .expect("a plain https URL must parse");

        assert_eq!(
            parsed.as_str(),
            "https://accounts.google.com/o/oauth2/v2/auth"
        );
        assert_eq!(parsed.as_url().host_str(), Some("accounts.google.com"));
    }

    #[test]
    fn https_url_rejects_plain_http_with_the_actual_scheme_named() {
        let err = HttpsUrl::parse("http://accounts.google.com")
            .expect_err("plain http must be rejected by the scheme constraint");

        assert_eq!(
            err,
            HttpsUrlError::SchemeNotHttps {
                actual_scheme: "http".to_string()
            }
        );
        assert!(
            err.to_string().contains("https"),
            "the message must state the required scheme: {err}"
        );
    }

    #[test]
    fn https_url_rejects_relative_and_non_http_schemes() {
        assert_eq!(
            HttpsUrl::parse("not a url at all"),
            Err(HttpsUrlError::NotAnAbsoluteUrl)
        );
        assert_eq!(
            // A relative reference must not pass, even one that looks like a path
            // on some implied host.
            HttpsUrl::parse("/.well-known/openid-configuration"),
            Err(HttpsUrlError::NotAnAbsoluteUrl)
        );
        assert_eq!(
            HttpsUrl::parse("ftp://files.example.com/pub"),
            Err(HttpsUrlError::SchemeNotHttps {
                actual_scheme: "ftp".to_string()
            })
        );
        // A hostless https URL is refused by the URL parser itself rather than by
        // the scheme check; both paths must end in an error, never a value.
        assert!(HttpsUrl::parse("https://").is_err());
    }

    // -------------------------------------------------------------------------
    // read_bounded_bytes (vendored prerequisite — see the module comment above it)
    // -------------------------------------------------------------------------

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
        let body = "x".repeat(MAX_UPSTREAM_BODY_BYTES as usize);
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
            Some(MAX_UPSTREAM_BODY_BYTES),
            "fixture must carry an honest Content-Length at the limit"
        );

        let bytes = read_bounded_bytes(response)
            .await
            .expect("a body at exactly the ceiling fits");
        assert_eq!(bytes.len() as u64, MAX_UPSTREAM_BODY_BYTES);
    }

    #[tokio::test]
    async fn bounded_read_rejects_oversized_body_with_honest_content_length_without_reading_it() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/toobig"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("y".repeat(MAX_UPSTREAM_BODY_BYTES as usize + 1)),
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
        let oversized = vec![b'a'; MAX_UPSTREAM_BODY_BYTES as usize + 1024];
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
