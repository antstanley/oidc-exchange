use oidc_exchange_core::error::{Error, Result};
use serde::Deserialize;

/// Parsed OIDC provider discovery document.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryDocument {
    pub issuer: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub revocation_endpoint: Option<String>,
    // Other fields are ignored via serde's default behavior.
}

/// Fetch and parse an OIDC provider's `.well-known/openid-configuration` document.
///
/// Per RFC 8414 §3.3, the `issuer` field in the returned document must be identical to
/// the issuer URL used to construct the discovery request URL; a mismatch is rejected.
pub async fn discover(issuer_url: &str) -> Result<DiscoveryDocument> {
    assert!(!issuer_url.is_empty(), "issuer_url must not be empty");

    let normalised_issuer = issuer_url.trim_end_matches('/');
    let url = format!("{normalised_issuer}/.well-known/openid-configuration");
    let response = crate::shared::http::client()
        .get(&url)
        .send()
        .await
        .map_err(|e| Error::ProviderError {
            provider: issuer_url.to_string(),
            detail: e.to_string(),
        })?;
    let doc = response
        .json::<DiscoveryDocument>()
        .await
        .map_err(|e| Error::ProviderError {
            provider: issuer_url.to_string(),
            detail: e.to_string(),
        })?;

    if doc.issuer.trim_end_matches('/') != normalised_issuer {
        return Err(Error::ProviderError {
            provider: issuer_url.to_string(),
            detail: format!(
                "discovered issuer '{}' does not match configured issuer '{}'",
                doc.issuer, normalised_issuer
            ),
        });
    }

    assert_eq!(
        doc.issuer.trim_end_matches('/'),
        normalised_issuer,
        "discovery document issuer must match the configured issuer after normalisation"
    );

    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn discover_parses_openid_configuration() {
        let server = MockServer::start().await;

        let body = serde_json::json!({
            "issuer": server.uri(),
            "token_endpoint": format!("{}/oauth/token", server.uri()),
            "jwks_uri": format!("{}/.well-known/jwks.json", server.uri()),
            "revocation_endpoint": format!("{}/oauth/revoke", server.uri()),
            "authorization_endpoint": format!("{}/oauth/authorize", server.uri())
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        let doc = discover(&server.uri())
            .await
            .expect("discovery should succeed");

        assert_eq!(doc.issuer, server.uri());
        assert_eq!(doc.token_endpoint, format!("{}/oauth/token", server.uri()));
        assert_eq!(
            doc.jwks_uri,
            format!("{}/.well-known/jwks.json", server.uri())
        );
        assert_eq!(
            doc.revocation_endpoint.as_deref(),
            Some(format!("{}/oauth/revoke", server.uri()).as_str())
        );
    }

    #[tokio::test]
    async fn discover_handles_missing_optional_fields() {
        let server = MockServer::start().await;

        let body = serde_json::json!({
            "issuer": server.uri(),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri())
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        let doc = discover(&server.uri())
            .await
            .expect("discovery should succeed");

        assert_eq!(doc.issuer, server.uri());
        assert!(doc.revocation_endpoint.is_none());
    }

    #[tokio::test]
    async fn discover_returns_error_on_invalid_json() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let result = discover(&server.uri()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn discover_strips_trailing_slash_from_issuer_url() {
        let server = MockServer::start().await;

        let body = serde_json::json!({
            "issuer": server.uri(),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri())
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        // Pass URL with trailing slash
        let url_with_slash = format!("{}/", server.uri());
        let doc = discover(&url_with_slash)
            .await
            .expect("discovery should succeed with trailing slash");

        assert_eq!(doc.issuer, server.uri());
    }

    #[tokio::test]
    async fn discover_rejects_mismatched_issuer() {
        let server = MockServer::start().await;

        let body = serde_json::json!({
            "issuer": "https://evil.example.com",
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri())
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        let result = discover(&server.uri()).await;

        let err = result.expect_err("mismatched issuer should be rejected");
        match err {
            Error::ProviderError { provider, detail } => {
                assert_eq!(provider, server.uri());
                assert!(detail.contains(&server.uri()));
                assert!(detail.contains("evil.example.com"));
            }
            other => panic!("expected ProviderError, got {other:?}"),
        }
    }
}
