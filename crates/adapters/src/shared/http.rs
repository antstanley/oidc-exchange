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
}
