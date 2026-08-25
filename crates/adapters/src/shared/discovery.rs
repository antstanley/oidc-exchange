use oidc_exchange_core::config::HttpsUrl;
use oidc_exchange_core::error::{Error, Result};
use serde::Deserialize;

use crate::shared::origins::{check_pinned_origin, EndpointOrigins, ENDPOINT_ORIGIN_CHECK_MODE};
use crate::shared::transport::ProviderTransport;

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
///
/// Each endpoint the document supplies (`token_endpoint`, `jwks_uri`, and
/// `revocation_endpoint` when present) is checked against the provider's pinned
/// endpoint-origin set. The check runs in the shipped [`ENDPOINT_ORIGIN_CHECK_MODE`]:
/// warning first for one release, then a separately reviewed enforcement flip — a
/// discovery document may confirm which origins this service talks to but can never
/// widen them.
///
/// The fetch goes through [`ProviderTransport`], so the response status is checked
/// before any body is read and the document body cannot exceed the shared byte
/// ceiling — an oversized "discovery document" is rejected before it is materialised.
pub async fn discover(
    issuer_url: &HttpsUrl,
    permitted_origins: &EndpointOrigins,
) -> Result<DiscoveryDocument> {
    assert!(
        !permitted_origins.as_list().is_empty(),
        "the issuer's own origin always pins at least one permitted origin"
    );

    let normalised_issuer = issuer_url.as_str().trim_end_matches('/');
    let url = format!("{normalised_issuer}/.well-known/openid-configuration");

    #[derive(Deserialize)]
    struct RawDiscoveryDocument {
        issuer: String,
        token_endpoint: String,
        jwks_uri: String,
        revocation_endpoint: Option<String>,
    }

    let raw: RawDiscoveryDocument = ProviderTransport
        .get_json(issuer_url.as_str(), &url)
        .await?
        .parsed(issuer_url.as_str())?;

    if raw.issuer.trim_end_matches('/') != normalised_issuer {
        return Err(Error::ProviderError {
            provider: issuer_url.as_str().to_string(),
            detail: format!(
                "discovered issuer '{}' does not match configured issuer '{}'",
                raw.issuer, normalised_issuer
            ),
        });
    }

    // Origin pinning runs after the issuer self-consistency check so a hostile
    // document is already bound to one issuer before its endpoints are judged.
    // Every supplied endpoint passes through the same mode decision — there is
    // no per-endpoint escape hatch that could let enforcement erode quietly.
    check_pinned_origin(
        issuer_url.as_str(),
        "token_endpoint",
        &raw.token_endpoint,
        permitted_origins,
        ENDPOINT_ORIGIN_CHECK_MODE,
    )?;
    check_pinned_origin(
        issuer_url.as_str(),
        "jwks_uri",
        &raw.jwks_uri,
        permitted_origins,
        ENDPOINT_ORIGIN_CHECK_MODE,
    )?;
    if let Some(revocation) = &raw.revocation_endpoint {
        check_pinned_origin(
            issuer_url.as_str(),
            "revocation_endpoint",
            revocation,
            permitted_origins,
            ENDPOINT_ORIGIN_CHECK_MODE,
        )?;
    }

    Ok(DiscoveryDocument {
        issuer: parse_discovered_endpoint(raw.issuer)?,
        token_endpoint: parse_discovered_endpoint(raw.token_endpoint)?,
        jwks_uri: parse_discovered_endpoint(raw.jwks_uri)?,
        revocation_endpoint: raw
            .revocation_endpoint
            .map(parse_discovered_endpoint)
            .transpose()?,
    })
}

/// Parse a discovery endpoint. Production accepts HTTPS only; test builds may use Wiremock HTTP.
#[cfg(not(test))]
fn parse_discovered_endpoint(value: String) -> Result<HttpsUrl> {
    HttpsUrl::parse(value)
}

#[cfg(test)]
fn parse_discovered_endpoint(value: String) -> Result<HttpsUrl> {
    HttpsUrl::parse_for_test(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::origins::OriginCheckMode;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Issuer-only permitted set: every endpoint these fixtures serve sits on
    /// the mock server's loopback origin, which is the issuer's origin.
    fn issuer_only_origins(issuer: &str) -> EndpointOrigins {
        EndpointOrigins::from_parts(issuer, &[], &[])
    }

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

        let doc = discover(
            &HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL"),
            &issuer_only_origins(&server.uri()),
        )
        .await
        .expect("discovery should succeed");

        assert_eq!(doc.issuer.as_str(), server.uri());
        assert_eq!(
            doc.token_endpoint.as_str(),
            format!("{}/oauth/token", server.uri())
        );
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

        let doc = discover(
            &HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL"),
            &issuer_only_origins(&server.uri()),
        )
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
            let err = discover(&issuer, &issuer_only_origins(&server.uri()))
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
    async fn discover_allows_http_endpoints_only_through_test_fixture_seam() {
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
        let doc = discover(&issuer, &issuer_only_origins(&server.uri()))
            .await
            .expect("test-only fixture seam should admit only Wiremock endpoints");
        assert_eq!(doc.token_endpoint.as_str(), "http://provider.example/token");
    }

    #[tokio::test]
    async fn discover_returns_error_on_invalid_json() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let result = discover(
            &HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL"),
            &issuer_only_origins(&server.uri()),
        )
        .await;
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
        let doc = discover(
            &HttpsUrl::parse_for_test(url_with_slash).expect("wiremock URL"),
            &issuer_only_origins(&server.uri()),
        )
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

        let result = discover(
            &HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL"),
            &issuer_only_origins(&server.uri()),
        )
        .await;

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
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("x".repeat(crate::shared::http::MAX_UPSTREAM_BODY_BYTES + 1)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let err = discover(
            &HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL"),
            &issuer_only_origins(&server.uri()),
        )
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

        let err = discover(
            &HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL"),
            &issuer_only_origins(&server.uri()),
        )
        .await
        .expect_err("a 404 discovery response must be an error");

        let message = err.to_string();
        assert!(
            message.contains("404"),
            "the status must be named in the failure: {message}"
        );
        assert!(
            message.contains("excerpt:"),
            "the body reaches the detail only through the audited redaction \
             pipeline (status, length, bounded excerpt): {message}"
        );
    }

    #[tokio::test]
    async fn discover_serves_warning_mode_for_an_undeclared_cross_origin_endpoint() {
        // The shipped release mode is Warn: a document naming an endpoint on an
        // origin outside the pinned set must still be served (the deployment
        // learns from the structured warning, not an outage) until the
        // separately reviewed enforcement flip lands.
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "issuer": server.uri(),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": "https://undeclared.example/jwks.json",
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        assert_eq!(
            ENDPOINT_ORIGIN_CHECK_MODE,
            OriginCheckMode::Warn,
            "the shipped mode is the warning stage; flipping it is a release decision"
        );

        let doc = discover(
            &HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL"),
            &issuer_only_origins(&server.uri()),
        )
        .await
        .expect("warning mode must not reject the same deployment it warns about");
        assert_eq!(
            doc.jwks_uri.as_str(),
            "https://undeclared.example/jwks.json"
        );
    }

    #[tokio::test]
    async fn discover_accepts_a_declared_cross_origin_endpoint() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "issuer": server.uri(),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": "https://www.googleapis.com/jwks.json",
            "revocation_endpoint": "https://oauth2.googleapis.com/revoke",
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let permitted = EndpointOrigins::from_parts(
            &server.uri(),
            &[],
            &[
                "https://oauth2.googleapis.com".to_string(),
                "https://www.googleapis.com".to_string(),
            ],
        );

        let doc = discover(
            &HttpsUrl::parse_for_test(server.uri()).expect("wiremock URL"),
            &permitted,
        )
        .await
        .expect("declared cross-origin endpoints must be accepted");
        assert_eq!(
            doc.jwks_uri.as_str(),
            "https://www.googleapis.com/jwks.json"
        );
        assert_eq!(
            doc.revocation_endpoint.as_ref().map(HttpsUrl::as_str),
            Some("https://oauth2.googleapis.com/revoke")
        );
    }
}
