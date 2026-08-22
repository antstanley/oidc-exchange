use oidc_exchange_core::domain::ProviderTokens;
use oidc_exchange_core::error::{Error, Result};

use crate::shared::transport::ProviderTransport;

/// Exchange an authorization code for provider tokens at the given token endpoint.
///
/// The exchange goes through [`ProviderTransport`]: the response status is
/// inspected before any body is read, and the single body read is bounded by the
/// shared byte ceiling. The same borrowed bytes serve both branches — the OAuth
/// error detail on non-success and the token parsing on success — so the body is
/// never read twice.
pub async fn exchange_code(
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
) -> Result<ProviderTokens> {
    assert!(
        !token_endpoint.is_empty(),
        "token_endpoint must not be empty"
    );

    let mut params = vec![
        ("grant_type".to_string(), "authorization_code".to_string()),
        ("code".to_string(), code.to_string()),
        ("redirect_uri".to_string(), redirect_uri.to_string()),
        ("client_id".to_string(), client_id.to_string()),
    ];
    if let Some(secret) = client_secret {
        params.push(("client_secret".to_string(), secret.to_string()));
    }

    let upstream = ProviderTransport
        .post_form(token_endpoint, token_endpoint, &params)
        .await?;

    if !upstream.is_success() {
        return Err(upstream.error_into(token_endpoint));
    }

    // Single bounded read, already performed inside the transport; both the
    // error branch above and this success branch borrow the same bytes.
    let body: serde_json::Value =
        serde_json::from_slice(upstream.bytes()).map_err(|e| Error::ProviderError {
            provider: token_endpoint.to_string(),
            detail: format!("invalid JSON in token endpoint response: {e}"),
        })?;

    let id_token = body["id_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::ProviderError {
            provider: token_endpoint.to_string(),
            detail: "token endpoint response is missing id_token".to_string(),
        })?
        .to_string();
    assert!(
        !id_token.is_empty(),
        "id_token must be non-empty on the success path"
    );

    Ok(ProviderTokens {
        id_token,
        refresh_token: body["refresh_token"].as_str().map(String::from),
        access_token: body["access_token"].as_str().map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn exchange_code_sends_correct_form_and_parses_response() {
        let server = MockServer::start().await;

        let token_response = serde_json::json!({
            "id_token": "eyJhbGciOiJSUzI1NiJ9.test-id-token",
            "access_token": "ya29.test-access-token",
            "refresh_token": "1//test-refresh-token",
            "token_type": "Bearer",
            "expires_in": 3600
        });

        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=test-auth-code"))
            .and(body_string_contains("redirect_uri="))
            .and(body_string_contains("client_id=my-client"))
            .and(body_string_contains("client_secret=my-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&token_response))
            .expect(1)
            .mount(&server)
            .await;

        let result = exchange_code(
            &format!("{}/oauth/token", server.uri()),
            "my-client",
            Some("my-secret"),
            "test-auth-code",
            "https://example.com/callback",
        )
        .await
        .expect("exchange should succeed");

        assert_eq!(result.id_token, "eyJhbGciOiJSUzI1NiJ9.test-id-token");
        assert_eq!(
            result.access_token.as_deref(),
            Some("ya29.test-access-token")
        );
        assert_eq!(
            result.refresh_token.as_deref(),
            Some("1//test-refresh-token")
        );
    }

    #[tokio::test]
    async fn exchange_code_without_client_secret() {
        let server = MockServer::start().await;

        let token_response = serde_json::json!({
            "id_token": "id-token-value",
            "access_token": "access-token-value"
        });

        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("client_id=public-client"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&token_response))
            .expect(1)
            .mount(&server)
            .await;

        let result = exchange_code(
            &format!("{}/oauth/token", server.uri()),
            "public-client",
            None,
            "auth-code",
            "https://example.com/cb",
        )
        .await
        .expect("exchange should succeed");

        assert_eq!(result.id_token, "id-token-value");
        assert_eq!(result.access_token.as_deref(), Some("access-token-value"));
        assert!(result.refresh_token.is_none());
    }

    #[tokio::test]
    async fn exchange_code_handles_missing_optional_tokens() {
        let server = MockServer::start().await;

        // Minimal response with only id_token
        let token_response = serde_json::json!({
            "id_token": "minimal-id-token"
        });

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&token_response))
            .mount(&server)
            .await;

        let result = exchange_code(
            &format!("{}/token", server.uri()),
            "client",
            None,
            "code",
            "https://example.com/cb",
        )
        .await
        .expect("exchange should succeed");

        assert_eq!(result.id_token, "minimal-id-token");
        assert!(result.access_token.is_none());
        assert!(result.refresh_token.is_none());
    }

    #[tokio::test]
    async fn exchange_code_returns_error_on_network_failure() {
        // Use a port that nothing listens on.
        let result = exchange_code(
            "http://127.0.0.1:1/oauth/token",
            "client",
            None,
            "code",
            "https://example.com/cb",
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn exchange_code_surfaces_oauth_error_on_non_2xx() {
        let server = MockServer::start().await;

        let error_body = serde_json::json!({
            "error": "invalid_grant",
            "error_description": "the authorization code has expired"
        });

        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(&error_body))
            .mount(&server)
            .await;

        let result = exchange_code(
            &format!("{}/oauth/token", server.uri()),
            "client",
            None,
            "expired-code",
            "https://example.com/cb",
        )
        .await;

        let err = result.expect_err("a 400 OAuth error must not succeed");
        let message = err.to_string();
        assert!(
            message.contains("invalid_grant"),
            "error should name the OAuth error code, got: {message}"
        );
        assert!(
            message.contains("the authorization code has expired"),
            "error should include the error_description, got: {message}"
        );
    }

    #[tokio::test]
    async fn exchange_code_non_2xx_never_echoes_a_non_protocol_body() {
        // The old path fell back to embedding the raw response body in the
        // error detail; through the transport's safe detail path an HTML or
        // otherwise non-protocol failure body must not reach the error string.
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(
                ResponseTemplate::new(502).set_body_string("<html>bad gateway page</html>"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let err = exchange_code(
            &format!("{}/oauth/token", server.uri()),
            "client",
            None,
            "code",
            "https://example.com/cb",
        )
        .await
        .expect_err("a 502 must fail");

        let message = err.to_string();
        assert!(
            message.contains("502"),
            "the status must be named, got: {message}"
        );
        assert!(
            !message.contains("<html>") && !message.contains("bad gateway page"),
            "the raw failure body must never reach the error surface: {message}"
        );
    }

    #[tokio::test]
    async fn exchange_code_rejects_oversized_success_body() {
        // Even a "successful" token response is bounded by the shared ceiling:
        // nothing about a 2xx status makes an unbounded body worth buffering.
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "p".repeat(crate::shared::http::MAX_UPSTREAM_BODY_BYTES as usize + 1),
            ))
            .expect(1)
            .mount(&server)
            .await;

        let err = exchange_code(
            &format!("{}/oauth/token", server.uri()),
            "client",
            None,
            "code",
            "https://example.com/cb",
        )
        .await
        .expect_err("an over-ceiling token response must fail");

        assert!(
            format!("{err}").contains("exceeded the"),
            "must be the distinctive cap error, got: {err}"
        );
    }

    #[tokio::test]
    async fn exchange_code_rejects_2xx_response_missing_id_token() {
        let server = MockServer::start().await;

        // A 2xx body with no id_token must be rejected, never defaulted to "".
        let token_response = serde_json::json!({
            "access_token": "access-token-value"
        });

        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&token_response))
            .mount(&server)
            .await;

        let result = exchange_code(
            &format!("{}/oauth/token", server.uri()),
            "client",
            None,
            "code",
            "https://example.com/cb",
        )
        .await;

        assert!(
            result.is_err(),
            "a 2xx response without id_token must be an error"
        );
        assert!(
            matches!(result.unwrap_err(), Error::ProviderError { .. }),
            "the missing-id_token error must be a ProviderError"
        );
    }
}
