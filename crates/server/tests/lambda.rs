//! Integration coverage for Lambda runtime mode: drives an API Gateway HTTP-API (v2) event
//! through `lambda_http::request::from_str` and into `bootstrap::build_routers`' single-plane
//! output via `tower::ServiceExt::oneshot` — the same tower `Service` call `lambda_http::run`
//! makes on every real invocation, minus the runtime-API polling loop itself. See task 03 of
//! `.specs/plans/2026-07-02-implement_lambda_runtime/plan.md`.

use std::collections::HashMap;

use http_body_util::BodyExt;
use tower::ServiceExt;

use oidc_exchange::bootstrap::build_routers;
use oidc_exchange_core::config::AppConfig;
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
};

/// Build the real production router a Lambda invocation would serve
/// (`bootstrap::build_routers` + the single-plane rule, full middleware stack
/// included) backed by mock adapters — the exact `app` value `main.rs` hands to
/// `lambda_http::run`.
fn build_app() -> axum::Router {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let mut config = AppConfig::default();
    config.server.issuer = "https://auth.example.com".to_string();

    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        providers,
        config.clone(),
    );

    build_routers(&config, service)
        .single_plane()
        .expect("the exchange role always yields a servable plane")
}

/// Build a minimal API Gateway HTTP-API (payload format v2) `GET` event for `path`, in the
/// same shape `lambda_http`'s own test fixtures use
/// (`apigw_v2_proxy_request_minimal.json`) — this is exactly the JSON API Gateway hands the
/// runtime API for a real invocation.
fn apigw_v2_get_event(path: &str) -> String {
    format!(
        r#"{{
            "version": "2.0",
            "routeKey": "$default",
            "rawPath": "{path}",
            "rawQueryString": "",
            "headers": {{
                "accept": "*/*",
                "content-length": "0",
                "host": "xxx.execute-api.us-east-1.amazonaws.com",
                "user-agent": "curl/7.64.1",
                "x-forwarded-for": "65.78.31.245",
                "x-forwarded-port": "443",
                "x-forwarded-proto": "https"
            }},
            "requestContext": {{
                "accountId": "123456789012",
                "apiId": "xxx",
                "domainName": "xxx.execute-api.us-east-1.amazonaws.com",
                "domainPrefix": "xxx",
                "http": {{
                    "method": "GET",
                    "path": "{path}",
                    "protocol": "HTTP/1.1",
                    "sourceIp": "65.78.31.245",
                    "userAgent": "curl/7.64.1"
                }},
                "requestId": "MIZRNhJtIAMEMDw=",
                "routeKey": "$default",
                "stage": "$default",
                "time": "06/May/2020:22:36:55 +0000",
                "timeEpoch": 1588804615616
            }},
            "isBase64Encoded": false
        }}"#
    )
}

/// A `GET /keys` API Gateway v2 event, parsed by `lambda_http` and routed through the same
/// single-plane router output the hyper path serves, returns 200 with a JWKS body — proving the
/// Lambda code path (`lambda_http`'s event-to-`tower::Service` translation) reaches the
/// identical router as the hyper branch, not a fork.
#[tokio::test]
async fn apigw_v2_event_for_keys_returns_200_with_jwks() {
    let app = build_app();
    let event = apigw_v2_get_event("/keys");
    let request = lambda_http::request::from_str(&event).expect("valid apigw v2 event parses");

    let response = app.oneshot(request).await.expect("router call never fails");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("jwks body is json");
    assert!(
        json["keys"].is_array(),
        "JWKS response must carry a `keys` array, got: {json}"
    );
}

/// Negative space: an API Gateway v2 event for a path no route serves returns 404 through the
/// exact same `lambda_http` → router path — the Lambda translation must not swallow or
/// mis-route an unknown path into a false 200.
#[tokio::test]
async fn apigw_v2_event_for_unknown_path_returns_404() {
    let app = build_app();
    let event = apigw_v2_get_event("/this-route-does-not-exist");
    let request = lambda_http::request::from_str(&event).expect("valid apigw v2 event parses");

    let response = app.oneshot(request).await.expect("router call never fails");

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}
