use async_trait::async_trait;
use hmac::{Hmac, Mac};
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
    url: String,
    secret: String,
    retries: u32,
    client: reqwest::Client,
}

impl WebhookUserSync {
    pub fn new(url: String, secret: String, timeout: std::time::Duration, retries: u32) -> Self {
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

    /// Build payload JSON, sign the delivery, and POST with retries.
    ///
    /// All delivery identity material — the RFC3339 timestamp, the ULID
    /// delivery id, and the signature over all three inputs — is minted ONCE,
    /// before the retry loop. Every attempt in a retry burst is byte-identical
    /// in body and delivery-authentication headers, so a receiver that saw one
    /// attempt has seen everything that defines the delivery; a repeated id is
    /// unambiguously a retry of the same occasion, never a new one.
    async fn send_webhook(&self, event_name: &str, data: serde_json::Value) -> Result<()> {
        let sent_at = chrono::Utc::now().to_rfc3339();
        let delivery_id = ulid::Ulid::new().to_string();

        assert!(
            !sent_at.is_empty() && !delivery_id.is_empty(),
            "delivery identity material must be minted before signing"
        );

        let payload = serde_json::json!({
            "event": event_name,
            // The in-body timestamp remains for receivers written against the
            // previous contract; it records the same single delivery occasion.
            "timestamp": sent_at,
            "data": data,
        });

        let body = serde_json::to_vec(&payload).map_err(|e| Error::SyncError {
            detail: format!("failed to serialize webhook payload: {e}"),
        })?;

        let signature = compute_delivery_signature(&self.secret, &sent_at, &delivery_id, &body);

        let mut last_err = None;
        for attempt in 0..=self.retries {
            if attempt > 0 {
                // Exponential backoff: 100ms, 200ms, 400ms, ... capped at MAX_BACKOFF_MS.
                tokio::time::sleep(backoff_delay(attempt)).await;
            }

            match self
                .client
                .post(&self.url)
                .header("Content-Type", "application/json")
                // The three delivery headers travel on every attempt, with the
                // values minted once above.
                .header("X-Webhook-Timestamp", &sent_at)
                .header("X-Webhook-Delivery-Id", &delivery_id)
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

/// Compute the `X-Signature-256` value for one webhook delivery.
///
/// The MAC covers `<timestamp> "." <delivery-id> "." <raw body>` under the
/// configured secret, hex-encoded and prefixed with `sha256=`. The dot
/// separators make the signed input unambiguous (a body cannot splice itself
/// across field boundaries), and binding the timestamp plus delivery id means a
/// captured `(body, signature)` pair from one delivery is worthless for another:
/// origin authenticity alone was replayable forever.
fn compute_delivery_signature(
    secret: &str,
    timestamp: &str,
    delivery_id: &str,
    body: &[u8],
) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(delivery_id.as_bytes());
    mac.update(b".");
    mac.update(body);
    let result = mac.finalize();
    format!("sha256={}", hex::encode(result.into_bytes()))
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

        Mock::given(method("POST"))
            .and(path("/"))
            .and(header("Content-Type", "application/json"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sync = WebhookUserSync::new(
            format!("{}/", server.uri()),
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

        // All three delivery headers must travel together.
        let sig_header = req
            .headers
            .get("X-Signature-256")
            .expect("X-Signature-256 header should be present")
            .to_str()
            .unwrap()
            .to_string();
        let ts_header = req
            .headers
            .get("X-Webhook-Timestamp")
            .expect("X-Webhook-Timestamp header should be present")
            .to_str()
            .unwrap()
            .to_string();
        let id_header = req
            .headers
            .get("X-Webhook-Delivery-Id")
            .expect("X-Webhook-Delivery-Id header should be present")
            .to_str()
            .unwrap()
            .to_string();

        // The signature is the sha256=-prefixed MAC over
        // timestamp "." delivery-id "." raw body.
        let expected_sig = compute_delivery_signature(secret, &ts_header, &id_header, &req.body);
        assert_eq!(sig_header, expected_sig, "delivery HMAC should match");
        assert!(
            sig_header.starts_with("sha256="),
            "the emitted value carries the algorithm prefix: {sig_header}"
        );
        assert_eq!(sig_header.len(), "sha256=".len() + 64);

        // Verify payload structure: the in-body timestamp remains for old parsers.
        let payload: serde_json::Value =
            serde_json::from_slice(&req.body).expect("body should be valid JSON");
        assert_eq!(payload["event"], "user.created");
        assert_eq!(
            payload["timestamp"], ts_header,
            "body timestamp records the same minted delivery instant"
        );
        assert_eq!(payload["data"]["id"], "usr_test123");
        assert_eq!(payload["data"]["email"], "alice@example.com");
    }

    /// A captured old `(body, signature)` pair must not validate as a new
    /// delivery under the documented receiver algorithm — the exact migration
    /// hazard the receiver docs describe.
    #[test]
    fn body_only_signature_from_the_old_scheme_never_validates() {
        let secret = "test-secret-key";
        let body = br#"{"event":"user.created","timestamp":"2026-01-01T00:00:00Z"}"#;

        // What the OLD sender emitted for this body (hex MAC over body only).
        let mut old_mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("key ok");
        old_mac.update(body);
        let old_emitted = hex::encode(old_mac.finalize().into_bytes());

        // A receiver verifying per the NEW protocol recomputes over
        // timestamp.id.body with fresh header values.
        let receiver_computed = compute_delivery_signature(
            secret,
            "2026-08-05T12:00:00+00:00",
            "01J2ZXQEXAMPLEULID0000000",
            body,
        );

        assert_ne!(
            format!("sha256={old_emitted}"),
            receiver_computed,
            "an old captured pair must fail new-scheme verification"
        );
    }

    #[test]
    fn mutating_any_signed_field_invalidates_the_signature() {
        let secret = "test-secret-key";
        let body = br#"{"event":"user.deleted"}"#;
        let valid =
            compute_delivery_signature(secret, "2026-08-05T12:00:00+00:00", "01JTESTID", body);

        let tampered_timestamp =
            compute_delivery_signature(secret, "2026-08-05T13:00:00+00:00", "01JTESTID", body);
        let tampered_id =
            compute_delivery_signature(secret, "2026-08-05T12:00:00+00:00", "01JOTHERID", body);
        let tampered_body =
            compute_delivery_signature(secret, "2026-08-05T12:00:00+00:00", "01JTESTID", b"{}");

        assert_ne!(valid, tampered_timestamp, "timestamp is inside the MAC");
        assert_ne!(valid, tampered_id, "delivery id is inside the MAC");
        assert_ne!(valid, tampered_body, "body stays inside the MAC");
    }

    #[test]
    fn separator_positions_matter_to_the_mac() {
        // Without the dots, ("ab", "c") and ("a", "bc") would sign identically.
        let with_dots = compute_delivery_signature("s", "abc", "def", b"x");
        let shifted = compute_delivery_signature("s", "abcd", "ef", b"x");
        assert_ne!(with_dots, shifted, "separators make the input unambiguous");
    }

    #[tokio::test]
    async fn independent_deliveries_carry_distinct_delivery_ids() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2)
            .mount(&server)
            .await;

        let sync = WebhookUserSync::new(
            format!("{}/", server.uri()),
            "secret".to_string(),
            std::time::Duration::from_secs(5),
            0,
        );

        let mut user = test_user();
        sync.notify_user_created(&user)
            .await
            .expect("first delivery");
        user.id = "usr_second456".to_string();
        sync.notify_user_created(&user)
            .await
            .expect("second delivery");

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);

        let id_of = |req: &wiremock::Request| {
            req.headers
                .get("X-Webhook-Delivery-Id")
                .expect("delivery id present")
                .to_str()
                .expect("header is ascii")
                .to_string()
        };
        let first_id = id_of(&requests[0]);
        let second_id = id_of(&requests[1]);

        assert_ne!(
            first_id, second_id,
            "two logical deliveries are two occasions with distinct ids"
        );

        // Sanity on id shape: ULIDs are 26 characters.
        assert_eq!(first_id.len(), 26);
    }

    #[tokio::test]
    async fn retry_burst_reuses_the_same_id_timestamp_and_signature() {
        let server = MockServer::start().await;

        // Serve 500 twice, then 200 — three attempts of one delivery burst.
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
            format!("{}/", server.uri()),
            "secret".to_string(),
            std::time::Duration::from_secs(5),
            2,
        );

        sync.notify_user_created(&test_user())
            .await
            .expect("burst should eventually succeed");

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3, "one initial attempt plus two retries");

        let header_of = |req: &wiremock::Request, name: &str| {
            req.headers
                .get(name)
                .unwrap_or_else(|| panic!("{name} present on every attempt"))
                .to_str()
                .expect("header is ascii")
                .to_string()
        };

        let ids: Vec<String> = requests
            .iter()
            .map(|r| header_of(r, "X-Webhook-Delivery-Id"))
            .collect();
        let timestamps: Vec<String> = requests
            .iter()
            .map(|r| header_of(r, "X-Webhook-Timestamp"))
            .collect();
        let signatures: Vec<String> = requests
            .iter()
            .map(|r| header_of(r, "X-Signature-256"))
            .collect();

        assert!(
            ids.windows(2).all(|w| w[0] == w[1]),
            "every attempt in a burst carries ONE delivery id: {ids:?}"
        );
        assert!(
            timestamps.windows(2).all(|w| w[0] == w[1]),
            "every attempt carries the same minted timestamp: {timestamps:?}"
        );
        assert!(
            signatures.windows(2).all(|w| w[0] == w[1]),
            "every attempt carries the one signature minted outside the loop"
        );
        assert!(
            requests.windows(2).all(|w| w[0].body == w[1].body),
            "retry attempts are byte-identical in body"
        );

        // And the shared signature verifies against the shared identity material.
        let recomputed =
            compute_delivery_signature("secret", &timestamps[0], &ids[0], &requests[0].body);
        assert_eq!(signatures[0], recomputed);
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
            format!("{}/", server.uri()),
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
            format!("{}/", server.uri()),
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
            format!("{}/", server.uri()),
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
