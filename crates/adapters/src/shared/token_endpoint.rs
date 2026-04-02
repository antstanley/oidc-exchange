use oidc_exchange_core::domain::ProviderTokens;
use oidc_exchange_core::error::{Error, Result};

/// Exchange an authorization code for provider tokens at the given token endpoint.
pub async fn exchange_code(
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
) -> Result<ProviderTokens> {
    let client = reqwest::Client::new();
    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
    ];
    if let Some(secret) = client_secret {
        params.push(("client_secret", secret));
    }

    let response = client
        .post(token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|e| Error::ProviderError {
            provider: token_endpoint.to_string(),
            detail: e.to_string(),
        })?;

    let body: serde_json::Value = response.json().await.map_err(|e| Error::ProviderError {
        provider: token_endpoint.to_string(),
        detail: e.to_string(),
    })?;

    Ok(ProviderTokens {
        id_token: body["id_token"].as_str().unwrap_or_default().to_string(),
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
}
