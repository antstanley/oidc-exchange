use async_trait::async_trait;
use hmac::{Hmac, Mac};
use oidc_exchange_core::config::HttpsUrl;
use oidc_exchange_core::domain::User;
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::UserSync;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Base delay (in milliseconds) for the first retry's exponential backoff.
const BASE_BACKOFF_MS: u64 = 100;

/// Maximum left-shift applied to the base delay. Bounding the shift keeps
/// `1u64 << shift` from overflowing (and keeps the pre-clamp value from
/// growing absurdly large) even when `retries` is set very high.
const MAX_BACKOFF_SHIFT: u32 = 6;

/// Hard ceiling on the per-attempt retry delay, regardless of attempt count.
const MAX_BACKOFF_MS: u64 = 5_000;

/// Compute the exponential backoff delay for a given retry `attempt`
/// (1-indexed: the delay before the first retry, second retry, ...).
///
/// The delay doubles per attempt starting from `BASE_BACKOFF_MS`, with the
/// shift clamped to `MAX_BACKOFF_SHIFT` so it can never overflow, and the
/// resulting delay clamped to `MAX_BACKOFF_MS` so a large `retries` count
/// cannot accumulate hours of sleep inside a request.
fn backoff_delay(attempt: u32) -> std::time::Duration {
    let shift = (attempt - 1).min(MAX_BACKOFF_SHIFT);
    let delay_ms = BASE_BACKOFF_MS.saturating_mul(1u64 << shift);
    std::time::Duration::from_millis(delay_ms.min(MAX_BACKOFF_MS))
}

/// Sends user lifecycle events as webhook HTTP POST requests with HMAC-SHA256 signatures.
pub struct WebhookUserSync {
    url: HttpsUrl,
    secret: String,
    retries: u32,
    client: reqwest::Client,
}

impl WebhookUserSync {
    pub fn new(url: HttpsUrl, secret: String, timeout: std::time::Duration, retries: u32) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build reqwest client");

        Self {
            url,
            secret,
            retries,
            client,
        }
    }

    /// Build payload JSON, sign it, and POST with retries.
    async fn send_webhook(&self, event_name: &str, data: serde_json::Value) -> Result<()> {
        let payload = serde_json::json!({
            "event": event_name,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "data": data,
        });

        let body = serde_json::to_vec(&payload).map_err(|e| Error::SyncError {
            detail: format!("failed to serialize webhook payload: {e}"),
        })?;

        let signature = compute_hmac_hex(&self.secret, &body);

        let mut last_err = None;
        for attempt in 0..=self.retries {
            if attempt > 0 {
                // Exponential backoff: 100ms, 200ms, 400ms, ... capped at MAX_BACKOFF_MS.
                tokio::time::sleep(backoff_delay(attempt)).await;
            }

            match self
                .client
                .post(self.url.as_str())
                .header("Content-Type", "application/json")
                .header("X-Signature-256", &signature)
                .body(body.clone())
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(());
                    }
                    if status.is_server_error() {
                        last_err = Some(format!("server error: HTTP {status}"));
                        continue; // retry on 5xx
                    }
                    // 4xx — don't retry
                    return Err(Error::SyncError {
                        detail: format!("webhook rejected: HTTP {status}"),
                    });
                }
                Err(e) if e.is_timeout() || e.is_connect() => {
                    last_err = Some(format!("request error: {e}"));
                    continue; // retry on timeout/connection errors
                }
                Err(e) => {
                    return Err(Error::SyncError {
                        detail: format!("webhook request failed: {e}"),
                    });
                }
            }
        }

        Err(Error::SyncError {
            detail: format!(
                "webhook delivery failed after {} attempts: {}",
                self.retries + 1,
                last_err.unwrap_or_else(|| "unknown".to_string())
            ),
        })
    }
}

fn compute_hmac_hex(secret: &str, body: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(body);
    let result = mac.finalize();
    hex::encode(result.into_bytes())
}

#[async_trait]
impl UserSync for WebhookUserSync {
    async fn notify_user_created(&self, user: &User) -> Result<()> {
        let data = serde_json::to_value(user).map_err(|e| Error::SyncError {
            detail: format!("failed to serialize user: {e}"),
        })?;
        self.send_webhook("user.created", data).await
    }

    async fn notify_user_updated(&self, user: &User, changed_fields: &[&str]) -> Result<()> {
        let user_value = serde_json::to_value(user).map_err(|e| Error::SyncError {
            detail: format!("failed to serialize user: {e}"),
        })?;
        let data = serde_json::json!({
            "user": user_value,
            "changed_fields": changed_fields,
        });
        self.send_webhook("user.updated", data).await
    }

    async fn notify_user_deleted(&self, user_id: &str) -> Result<()> {
        let data = serde_json::json!({ "user_id": user_id });
        self.send_webhook("user.deleted", data).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oidc_exchange_core::config::HttpsUrl;
    use std::collections::HashMap;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_user() -> User {
        User {
            id: "usr_test123".to_string(),
            external_id: "google|abc".to_string(),
            provider: "google".to_string(),
            email: Some("alice@example.com".to_string()),
            display_name: Some("Alice".to_string()),
            metadata: HashMap::new(),
            claims: HashMap::new(),
            status: oidc_exchange_core::domain::user::UserStatus::Active,
            version: oidc_exchange_core::domain::user::INITIAL_USER_VERSION,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_backoff_delay_is_capped_and_never_overflows() {
        // retries = 20: far beyond MAX_BACKOFF_SHIFT, exercising both the
        // pre-clamp overflow guard and the post-clamp cap.
        for attempt in 1..=20u32 {
            let delay = backoff_delay(attempt);
            assert!(
                delay <= std::time::Duration::from_millis(MAX_BACKOFF_MS),
                "attempt {attempt} produced delay {delay:?} exceeding the {MAX_BACKOFF_MS}ms cap"
            );
        }

        // The delay must actually grow (exponentially) before it saturates,
        // not just be clamped to the cap from the first attempt.
        assert!(backoff_delay(1) < backoff_delay(2));
        assert!(backoff_delay(2) < backoff_delay(3));

        // Once the shift bound is reached, further attempts stay at the cap.
        assert_eq!(
            backoff_delay(MAX_BACKOFF_SHIFT + 1),
            std::time::Duration::from_millis(MAX_BACKOFF_MS)
        );
        assert_eq!(
            backoff_delay(20),
            std::time::Duration::from_millis(MAX_BACKOFF_MS)
        );
    }

    #[tokio::test]
    async fn test_successful_delivery_with_correct_hmac() {
        let server = MockServer::start().await;
        let secret = "test-secret-key";

        // We need a custom matcher to verify HMAC
        // For simplicity, we set up the mock to accept any valid request and then
        // verify the signature on the captured request.
        Mock::given(method("POST"))
            .and(path("/"))
            .and(header("Content-Type", "application/json"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sync = WebhookUserSync::new(
            HttpsUrl::parse_for_test(format!("{}/", server.uri())).expect("wiremock URL"),
            secret.to_string(),
            std::time::Duration::from_secs(5),
            2,
        );

        let user = test_user();
        sync.notify_user_created(&user)
            .await
            .expect("webhook should succeed");

        // Verify the request was received
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);

        let req = &requests[0];

        // Verify the signature header is present and correct
        let sig_header = req
            .headers
            .get("X-Signature-256")
            .expect("X-Signature-256 header should be present")
            .to_str()
            .unwrap()
            .to_string();

        // Recompute HMAC over the request body
        let expected_sig = compute_hmac_hex(secret, &req.body);
        assert_eq!(sig_header, expected_sig, "HMAC signature should match");

        // Verify payload structure
        let payload: serde_json::Value =
            serde_json::from_slice(&req.body).expect("body should be valid JSON");
        assert_eq!(payload["event"], "user.created");
        assert!(payload["timestamp"].is_string());
        assert_eq!(payload["data"]["id"], "usr_test123");
        assert_eq!(payload["data"]["email"], "alice@example.com");
    }

    #[tokio::test]
    async fn test_retry_on_5xx() {
        let server = MockServer::start().await;

        // Serve 500 for the first two requests, then 200
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(2)
            .expect(2)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sync = WebhookUserSync::new(
            HttpsUrl::parse_for_test(format!("{}/", server.uri())).expect("wiremock URL"),
            "secret".to_string(),
            std::time::Duration::from_secs(5),
            2, // 1 initial + 2 retries = 3 attempts total
        );

        let user = test_user();
        sync.notify_user_created(&user)
            .await
            .expect("should succeed after retries");

        // Verify total request count: 2 failures + 1 success = 3
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3, "should have made 3 requests total");
    }

    #[tokio::test]
    async fn test_4xx_no_retry() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(400))
            .expect(1)
            .mount(&server)
            .await;

        let sync = WebhookUserSync::new(
            HttpsUrl::parse_for_test(format!("{}/", server.uri())).expect("wiremock URL"),
            "secret".to_string(),
            std::time::Duration::from_secs(5),
            2,
        );

        let user = test_user();
        let result = sync.notify_user_created(&user).await;
        assert!(result.is_err());

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "should not retry on 4xx");
    }

    #[tokio::test]
    async fn test_redirect_is_not_followed_and_is_not_retried() {
        // A second, unmounted server: it never registers a `Mock`, so any request
        // that reached it would be answered with wiremock's default 404 and would
        // register as a received request. It stands in for a redirect target the
        // operator never configured.
        let redirect_target = MockServer::start().await;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/", redirect_target.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;

        let sync = WebhookUserSync::new(
            HttpsUrl::parse_for_test(format!("{}/", server.uri())).expect("wiremock URL"),
            "secret".to_string(),
            std::time::Duration::from_secs(5),
            2,
        );

        let user = test_user();
        let result = sync.notify_user_created(&user).await;
        assert!(result.is_err(), "a 3xx must not count as delivery success");

        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            1,
            "a 3xx is not retried, so the configured host sees exactly one request"
        );

        let redirect_requests = redirect_target.received_requests().await.unwrap();
        assert_eq!(
            redirect_requests.len(),
            0,
            "the client must not follow redirects: the signed body is never re-sent \
             to a location the operator did not configure"
        );
    }
}
