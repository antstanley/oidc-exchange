use oidc_exchange_core::config::HttpsUrl;
use oidc_exchange_core::error::{Error, Result};
use serde::Deserialize;

/// Parsed OIDC provider discovery document.
#[derive(Debug, Clone)]
pub struct DiscoveryDocument {
    pub issuer: HttpsUrl,
    pub token_endpoint: HttpsUrl,
    pub jwks_uri: HttpsUrl,
    pub revocation_endpoint: Option<HttpsUrl>,
    // Other fields are ignored via serde's default behavior.
}

/// Fetch and parse an OIDC provider's `.well-known/openid-configuration` document.
///
/// Per RFC 8414 §3.3, the `issuer` field in the returned document must be identical to
/// the issuer URL used to construct the discovery request URL; a mismatch is rejected.
pub async fn discover(issuer_url: &HttpsUrl) -> Result<DiscoveryDocument> {
    let normalised_issuer = issuer_url.as_str().trim_end_matches('/');
    let url = format!("{normalised_issuer}/.well-known/openid-configuration");
    let response = crate::shared::http::client()
        .get(&url)
        .send()
        .await
        .map_err(|e| Error::ProviderError {
            provider: issuer_url.as_str().to_string(),
            detail: e.to_string(),
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::ProviderError {
            provider: issuer_url.as_str().to_string(),
            detail: format!("discovery endpoint returned non-2xx status {status}"),
        });
    }

    #[derive(Deserialize)]
    struct RawDiscoveryDocument {
        issuer: String,
        token_endpoint: String,
        jwks_uri: String,
        revocation_endpoint: Option<String>,
    }

    let raw = response
        .json::<RawDiscoveryDocument>()
        .await
        .map_err(|e| Error::ProviderError {
            provider: issuer_url.as_str().to_string(),
            detail: e.to_string(),
        })?;

    if raw.issuer.trim_end_matches('/') != normalised_issuer {
        return Err(Error::ProviderError {
            provider: issuer_url.as_str().to_string(),
            detail: format!(
                "discovered issuer '{}' does not match configured issuer '{}'",
                raw.issuer, normalised_issuer
            ),
        });
    }

    Ok(DiscoveryDocument {
        issuer: HttpsUrl::parse(raw.issuer)?,
        token_endpoint: HttpsUrl::parse(raw.token_endpoint)?,
        jwks_uri: HttpsUrl::parse(raw.jwks_uri)?,
        revocation_endpoint: raw.revocation_endpoint.map(HttpsUrl::parse).transpose()?,
    })
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

        let doc = discover(&HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL"))
            .await
            .expect("discovery should succeed");

        assert_eq!(doc.issuer.as_str(), server.uri());
        assert_eq!(doc.token_endpoint.as_str(), format!("{}/oauth/token", server.uri()));
        assert_eq!(
            doc.jwks_uri.as_str(),
            format!("{}/.well-known/jwks.json", server.uri())
        );
        assert_eq!(
            doc.revocation_endpoint.as_ref().map(HttpsUrl::as_str),
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

        let doc = discover(&HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL"))
            .await
            .expect("discovery should succeed");

        assert_eq!(doc.issuer.as_str(), server.uri());
        assert!(doc.revocation_endpoint.is_none());
    }

    #[tokio::test]
    async fn discover_rejects_well_formed_document_on_non_success_status() {
        for status in [404, 500] {
            let server = MockServer::start().await;
            let body = serde_json::json!({
                "issuer": server.uri(),
                "token_endpoint": "https://provider.example/token",
                "jwks_uri": "https://provider.example/jwks"
            });

            Mock::given(method("GET"))
                .and(path("/.well-known/openid-configuration"))
                .respond_with(ResponseTemplate::new(status).set_body_json(&body))
                .mount(&server)
                .await;

            let issuer = HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL");
            let err = discover(&issuer)
                .await
                .expect_err("non-success discovery response must be rejected before parsing");
            match err {
                Error::ProviderError { provider, detail } => {
                    assert_eq!(provider, server.uri());
                    assert!(detail.contains(&status.to_string()));
                }
                other => panic!("expected ProviderError, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn discover_rejects_http_endpoints_after_successful_parse() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "issuer": server.uri(),
            "token_endpoint": "http://provider.example/token",
            "jwks_uri": "https://provider.example/jwks"
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        let issuer = HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL");
        assert!(discover(&issuer).await.is_err());
    }

    #[tokio::test]
    async fn discover_returns_error_on_invalid_json() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let result = discover(&HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL")).await;
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
        let doc = discover(&HttpsUrl::parse_for_test(url_with_slash).expect("wiremock URL"))
            .await
            .expect("discovery should succeed with trailing slash");

        assert_eq!(doc.issuer.as_str(), server.uri());
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

        let result = discover(&HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL")).await;

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
