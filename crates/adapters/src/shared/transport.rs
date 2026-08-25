//! The single outbound provider HTTP boundary.
//!
//! Every request this service sends to a provider endpoint — discovery, JWKS,
//! token exchange, and both revocation call sites — is issued here and nowhere
//! else. Owning the boundary in one type is what lets the integrity properties
//! hold everywhere at once instead of being re-decided (and eventually dropped)
//! per call site:
//!
//! 1. **Status before body.** The response status is inspected before any body
//!    byte is read, so a non-success answer is classified by its status, never
//!    by attacker-controlled payload shape.
//! 2. **Bounded bodies.** Every body is read under the shared
//!    `MAX_UPSTREAM_BODY_BYTES` ceiling. A success body feeds a parser, so it
//!    goes through [`crate::shared::http::read_bounded_bytes`] and *fails* at
//!    the ceiling rather than after it; a failure body is only diagnostics, so
//!    it goes through the truncating [`crate::shared::http::read_bounded`]
//!    instead. Either way a provider cannot make this process buffer an
//!    unbounded response.
//! 3. **Safe error detail.** Non-success responses are described through
//!    [`crate::shared::upstream::error_detail`], so protocol error codes and a
//!    bounded, credential-redacted excerpt reach operators while raw response
//!    bodies never do.
//!
//! The transport uses the process-wide shared client from
//! [`crate::shared::http`] with its fixed 5s connect / 10s total timeouts and
//! redirects disabled; it issues no request through anything else.

use serde::de::DeserializeOwned;

use crate::shared::http::{client, read_bounded_bytes, BoundedBodyError};
use crate::shared::upstream;
use oidc_exchange_core::error::{Error, Result};

/// Issues every outbound provider HTTP request and applies the shared
/// integrity properties to each response.
///
/// Stateless by design: the process-wide client it delegates to carries the
/// timeout and redirect policy, so instances cost nothing and need no
/// configuration.
#[derive(Debug, Clone, Copy)]
pub struct ProviderTransport;

/// One upstream response whose body has already been read through the byte
/// ceiling, carrying the status that was inspected *before* that read.
pub struct UpstreamBody {
    /// Status captured before the body was read — the transport's ordering
    /// guarantee made data.
    status: reqwest::StatusCode,
    /// Body bytes as received, guaranteed no larger than
    /// `MAX_UPSTREAM_BODY_BYTES` by construction.
    bytes: Vec<u8>,
}

impl std::fmt::Debug for UpstreamBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Manual Debug on purpose: a derived one would print response bodies
        // (which may carry tokens or personal data) into any panic or log line
        // that formats an UpstreamBody. Only the shape is reported, not the
        // content.
        f.debug_struct("UpstreamBody")
            .field("status", &self.status.as_u16())
            .field("bytes", &format!("{} bytes (redacted)", self.bytes.len()))
            .finish()
    }
}

impl ProviderTransport {
    /// Issue a GET expecting a JSON body on success.
    pub async fn get_json(&self, provider: &str, url: &str) -> Result<UpstreamBody> {
        let response = client()
            .get(url)
            .send()
            .await
            .map_err(|e| Error::ProviderError {
                provider: provider.to_string(),
                detail: format!("request failed: {e}"),
            })?;
        self.collect(provider, url, response).await
    }

    /// Issue a form-encoded POST expecting a JSON body on success.
    pub async fn post_form(
        &self,
        provider: &str,
        url: &str,
        form: &[(String, String)],
    ) -> Result<UpstreamBody> {
        let response =
            client()
                .post(url)
                .form(form)
                .send()
                .await
                .map_err(|e| Error::ProviderError {
                    provider: provider.to_string(),
                    detail: format!("request failed: {e}"),
                })?;
        self.collect(provider, url, response).await
    }

    /// Inspect the status, read the body through the ceiling, and package both.
    ///
    /// This is the one place the status-before-body ordering exists: callers
    /// receive an [`UpstreamBody`] whose status was recorded before a single
    /// body byte was buffered, so they cannot accidentally re-decide the order.
    async fn collect(
        &self,
        provider: &str,
        url: &str,
        response: reqwest::Response,
    ) -> Result<UpstreamBody> {
        // Order is load-bearing: status first, body second — and the status
        // decides which bounded read applies. A success body feeds a parser,
        // so a truncated one is garbage and oversize must FAIL at the ceiling.
        // A failure body is only ever diagnostics: it is truncated at the same
        // ceiling instead, so an oversized error page still yields its
        // status-led, redacted detail rather than collapsing into a cap error.
        let status = response.status();

        let bytes = if status.is_success() {
            read_bounded_bytes(response).await.map_err(|e| match e {
                BoundedBodyError::OverLimit { limit_bytes } => Error::ProviderError {
                    provider: provider.to_string(),
                    // The cap error names both the endpoint and the limit so it
                    // is alertable as a provider fault and actionable without
                    // correlation: "which upstream, and over what bound".
                    detail: format!(
                        "response from {url} exceeded the {limit_bytes}-byte upstream limit"
                    ),
                },
                BoundedBodyError::Network(e) => Error::ProviderError {
                    provider: provider.to_string(),
                    detail: format!("reading response body failed: {e}"),
                },
            })?
        } else {
            crate::shared::http::read_bounded(provider, response)
                .await?
                .into_inner()
                .into_bytes()
        };

        assert!(
            bytes.len() <= crate::shared::http::MAX_UPSTREAM_BODY_BYTES,
            "bounded body must respect MAX_UPSTREAM_BODY_BYTES"
        );

        Ok(UpstreamBody { status, bytes })
    }
}

impl UpstreamBody {
    /// The response status, captured before the body was read.
    pub fn status(&self) -> reqwest::StatusCode {
        self.status
    }

    /// Whether the response was a 2xx.
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Parse the bounded body as the expected success shape.
    ///
    /// A non-success status is itself an error here, described through the
    /// safe error-detail path — callers who use `parsed` get status handling
    /// for free rather than deciding per call site what a failure means.
    pub fn parsed<T: DeserializeOwned>(&self, provider: &str) -> Result<T> {
        if !self.status.is_success() {
            return Err(self.error_into(provider));
        }

        serde_json::from_slice(&self.bytes).map_err(|e| Error::ProviderError {
            provider: provider.to_string(),
            detail: format!("invalid JSON in success response: {e}"),
        })
    }

    /// The safe, bounded description of a non-success response, as an error.
    ///
    /// Surfaces OAuth `error`/`error_description` tokens when the body carries
    /// them; never echoes the raw body.
    pub fn error_into(&self, provider: &str) -> Error {
        Error::ProviderError {
            provider: provider.to_string(),
            // The body crosses into the audited redaction boundary as a secret:
            // `error_detail` consumes it and surfaces only bounded protocol
            // tokens, never raw upstream text.
            detail: upstream::error_detail(
                self.status,
                oidc_exchange_core::Secret::new(
                    String::from_utf8_lossy(&self.bytes).into_owned(),
                ),
            ),
        }
    }

    /// The already-bounded raw body bytes, for callers whose success shape is
    /// not plain JSON (or who must parse once and branch twice). The single
    /// bounded read happened inside the transport; borrowing these bytes does
    /// not re-read anything.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ENDPOINT_PATH: &str = "/provider";

    #[tokio::test]
    async fn get_json_parses_a_success_shape() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(ENDPOINT_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "issuer": "https://example.test",
                "token_endpoint": "https://example.test/token",
                "jwks_uri": "https://example.test/jwks",
            })))
            .expect(1)
            .mount(&server)
            .await;

        #[derive(serde::Deserialize)]
        struct Doc {
            issuer: String,
            token_endpoint: String,
        }

        let body = ProviderTransport
            .get_json("test-provider", &format!("{}{ENDPOINT_PATH}", server.uri()))
            .await
            .expect("a small success body must come back intact");
        assert!(body.is_success());
        assert_eq!(body.status(), 200);

        let doc: Doc = body.parsed("test-provider").expect("shape should parse");
        assert_eq!(doc.issuer, "https://example.test");
        assert_eq!(doc.token_endpoint, "https://example.test/token");
    }

    #[tokio::test]
    async fn status_is_evaluated_before_any_body_is_read() {
        // A 500 carrying a non-JSON body must be classified by its STATUS (the
        // safe error-detail path), never by parsing whatever the body held —
        // proving the status was inspected before the body was consumed.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(ENDPOINT_PATH))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_string("<html>this body would fail JSON parsing</html>"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let body = ProviderTransport
            .get_json("test-provider", &format!("{}{ENDPOINT_PATH}", server.uri()))
            .await
            .expect("the response arrives; its status decides what it means");
        assert_eq!(body.status(), 500);
        assert!(!body.is_success());

        let err = body
            .parsed::<serde_json::Value>("test-provider")
            .expect_err("a 500 must not parse as a success shape");

        let message = err.to_string();
        assert!(
            message.contains("500"),
            "the status must lead the failure description: {message}"
        );
        assert!(
            message.contains("excerpt:"),
            "the body reaches the detail only through the audited redaction \
             pipeline (status, length, bounded excerpt): {message}"
        );
    }

    #[tokio::test]
    async fn non_success_response_surfaces_oauth_error_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(ENDPOINT_PATH))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "code expired",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let body = ProviderTransport
            .post_form(
                "test-provider",
                &format!("{}{ENDPOINT_PATH}", server.uri()),
                &[("code".to_string(), "expired".to_string())],
            )
            .await
            .expect("the exchange itself completes; the status is the failure");
        assert!(!body.is_success());

        let err = body.error_into("test-provider");
        let message = err.to_string();
        assert!(
            message.contains("invalid_grant") && message.contains("code expired"),
            "protocol tokens must survive into the error: {message}"
        );
    }

    #[tokio::test]
    async fn oversized_success_body_is_a_distinct_cap_error_naming_limit_and_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(ENDPOINT_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_string("z".repeat(70 * 1024)))
            .expect(1)
            .mount(&server)
            .await;

        let endpoint = format!("{}{ENDPOINT_PATH}", server.uri());
        let err = ProviderTransport
            .get_json("test-provider", &endpoint)
            .await
            .expect_err("an over-ceiling success body must fail");

        let message = err.to_string();
        assert!(
            message.contains("70") || message.contains("exceeded"),
            "must read as a cap error, got: {message}"
        );
        assert!(
            message.contains(&endpoint),
            "the cap error must name the endpoint: {message}"
        );
        assert!(
            message.contains(format!("{}", crate::shared::http::MAX_UPSTREAM_BODY_BYTES).as_str()),
            "the cap error must name the limit: {message}"
        );
        assert!(
            !message.contains("invalid JSON"),
            "a cap error must stay distinct from a parse failure: {message}"
        );
    }

    #[tokio::test]
    async fn oversized_chunked_success_body_hits_the_same_cap_error() {
        // Chunked transfer defeats any Content-Length short-circuit; the
        // streaming running-total bound inside the transport must still fire.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ephemeral bind works");
        let addr = listener.local_addr().expect("local_addr resolves");
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut socket, _) = listener.accept().await.expect("one connection");
            let mut scratch = [0u8; 4096];
            loop {
                let read = socket.read(&mut scratch).await.expect("readable");
                if read == 0 || scratch[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .expect("head writable");
            for _ in 0..8 {
                socket
                    .write_all(format!("{:x}\r\n", 16 * 1024).as_bytes())
                    .await
                    .expect("chunk header writable");
                socket
                    .write_all(&vec![b'q'; 16 * 1024])
                    .await
                    .expect("chunk writable");
                socket.write_all(b"\r\n").await.expect("CRLF writable");
            }
            socket
                .write_all(b"0\r\n\r\n")
                .await
                .expect("terminal chunk writable");
        });

        let endpoint = format!("http://{addr}/chunked-provider");
        let err = ProviderTransport
            .get_json("test-provider", &endpoint)
            .await
            .expect_err("128 KiB chunked against a 64 KiB ceiling must fail mid-stream");

        let message = err.to_string();
        assert!(
            message.contains("exceeded") && message.contains(&endpoint),
            "chunked over-limit must produce the same distinctive cap error naming \
             limit and endpoint, got: {message}"
        );
    }

    #[tokio::test]
    async fn post_form_sends_form_encoded_pairs_and_reads_one_bounded_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(ENDPOINT_PATH))
            .and(wiremock::matchers::body_string_contains(
                "grant_type=authorization_code",
            ))
            .and(wiremock::matchers::body_string_contains("code=abc"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let body = ProviderTransport
            .post_form(
                "test-provider",
                &format!("{}{ENDPOINT_PATH}", server.uri()),
                &[
                    ("grant_type".to_string(), "authorization_code".to_string()),
                    ("code".to_string(), "abc".to_string()),
                ],
            )
            .await
            .expect("small success POST must complete");

        let value: serde_json::Value = body.parsed("test-provider").expect("parses");
        assert_eq!(value["ok"], serde_json::json!(true));
        // The single bounded read: bytes() returns the same buffer, not a new fetch.
        assert_eq!(body.bytes(), br#"{"ok":true}"#);
    }

    #[tokio::test]
    async fn unreachable_provider_is_a_provider_error_not_a_panic() {
        let err = ProviderTransport
            .get_json("test-provider", "http://127.0.0.1:1/provider")
            .await
            .expect_err("connecting to a dead port must error");

        assert!(
            matches!(err, Error::ProviderError { .. }),
            "transport failures are provider faults, got: {err:?}"
        );
    }
}
