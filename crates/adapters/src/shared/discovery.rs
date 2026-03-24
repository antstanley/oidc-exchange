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
pub async fn discover(issuer_url: &str) -> Result<DiscoveryDocument> {
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer_url.trim_end_matches('/')
    );
    let response = reqwest::get(&url).await.map_err(|e| Error::ProviderError {
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

        let doc = discover(&server.uri()).await.expect("discovery should succeed");

        assert_eq!(doc.issuer, server.uri());
        assert_eq!(
            doc.token_endpoint,
            format!("{}/oauth/token", server.uri())
        );
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

        let doc = discover(&server.uri()).await.expect("discovery should succeed");

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
}
