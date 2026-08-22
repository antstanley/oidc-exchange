use oidc_exchange_core::error::{Error, Result};
use serde::Deserialize;

use crate::shared::transport::ProviderTransport;

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
///
/// The fetch goes through [`ProviderTransport`], so the response status is checked
/// before any body is read and the document body cannot exceed the shared byte
/// ceiling — an oversized "discovery document" is rejected before it is materialised.
pub async fn discover(issuer_url: &str) -> Result<DiscoveryDocument> {
    assert!(!issuer_url.is_empty(), "issuer_url must not be empty");

    let normalised_issuer = issuer_url.trim_end_matches('/');
    let url = format!("{normalised_issuer}/.well-known/openid-configuration");
    let doc: DiscoveryDocument = ProviderTransport
        .get_json(issuer_url, &url)
        .await?
        .parsed(issuer_url)?;

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

    #[tokio::test]
    async fn discover_rejects_oversized_success_document_before_parsing() {
        // A discovery document is a few kilobytes; anything at the shared byte
        // ceiling is a hostile or broken origin, and must be rejected as the
        // distinctive cap error before JSON materialisation, never parsed.
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "x".repeat(crate::shared::http::MAX_UPSTREAM_BODY_BYTES as usize + 1),
            ))
            .expect(1)
            .mount(&server)
            .await;

        let err = discover(&server.uri())
            .await
            .expect_err("an oversized discovery document must not be accepted");

        let message = err.to_string();
        assert!(
            message.contains("exceeded the"),
            "must be the distinctive cap error, got: {message}"
        );
        assert!(
            message.contains(&server.uri()),
            "the cap error must name the endpoint: {message}"
        );
        assert!(
            !message.contains("invalid JSON"),
            "the cap error must stay distinct from a parse failure: {message}"
        );
    }

    #[tokio::test]
    async fn discover_rejects_non_success_status_with_safe_detail() {
        // The transport owns the status check now that it is the sole caller of
        // the discovery fetch: a 404 must be an error whose detail comes from
        // the safe path, not a JSON parse error over an HTML body.
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(404).set_body_string("<html>nope</html>"))
            .expect(1)
            .mount(&server)
            .await;

        let err = discover(&server.uri())
            .await
            .expect_err("a 404 discovery response must be an error");

        let message = err.to_string();
        assert!(
            message.contains("404"),
            "the status must be named in the failure: {message}"
        );
        assert!(
            !message.contains("<html>"),
            "the error body must never be echoed: {message}"
        );
    }
}
