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
}
