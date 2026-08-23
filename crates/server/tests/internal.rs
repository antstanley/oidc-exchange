use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use oidc_exchange::bootstrap::build_routers;
use oidc_exchange_core::config::AppConfig;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockKeyManager, MockRateLimiter, MockRepository, MockUserSync,
};

const TEST_SECRET: &str = "test-internal-secret-1234";

/// The production admin plane (`bootstrap::build_routers`, role = "admin",
/// full middleware stack including operator auth) over mock adapters —
/// exactly what a role = "admin" process serves on its listener.
fn build_admin_plane() -> Router {
    let config = test_config();

    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(MockRateLimiter::new()),
        HashMap::new(),
        config.clone(),
    );

    let routers = build_routers(&config, service).expect("the admin test config builds routers");
    assert!(
        routers.public.is_none(),
        "role = \"admin\" must not produce a public router"
    );
    routers
        .admin
        .expect("role = \"admin\" produces the admin router")
}

fn test_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.server.issuer = "https://auth.example.com".to_string();
    config.server.role = "admin".to_string();
    config.internal_api.enabled = true;
    config.internal_api.shared_secret = Some(TEST_SECRET.to_string());
    config
}

async fn body_to_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

// ---------------------------------------------------------------------------
// 1. Internal auth rejection: no auth header → 401
// ---------------------------------------------------------------------------

#[tokio::test]
async fn internal_auth_rejects_missing_auth() {
    let app = build_admin_plane();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"external_id": "ext1", "provider": "google"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "unauthorized");
}

// ---------------------------------------------------------------------------
// 2. Internal auth rejection: wrong secret → 401
// ---------------------------------------------------------------------------

#[tokio::test]
async fn internal_auth_rejects_wrong_secret() {
    let app = build_admin_plane();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", "Bearer wrong-secret")
                .body(Body::from(
                    json!({"external_id": "ext1", "provider": "google"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "unauthorized");
}

// ---------------------------------------------------------------------------
// 3. Internal auth with correct secret → proceeds to handler
// ---------------------------------------------------------------------------

#[tokio::test]
async fn internal_auth_passes_with_correct_secret() {
    let app = build_admin_plane();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({"external_id": "ext1", "provider": "google", "email": "user@example.com"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should not be 401 — handler should have processed the request
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.status(), StatusCode::CREATED);
}

// ---------------------------------------------------------------------------
// 3b. An empty configured secret must never produce a servable admin plane:
// the auth gate refuses to build with a blank shared secret, so
// `build_routers` fails closed at startup. (`AppConfig::validate` rejects the
// same config at load; this guards the router layer for a hand-built config,
// e.g. from an embedder.)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_configured_secret_fails_router_build() {
    let mut config = test_config();
    config.internal_api.shared_secret = Some(String::new());

    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(MockRateLimiter::new()),
        HashMap::new(),
        config.clone(),
    );

    let outcome = build_routers(&config, service);

    let err = outcome.expect_err("an empty shared secret must fail router construction");
    assert!(
        err.to_string().contains("no secret is configured"),
        "the failure must name the missing credential, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// 4. Create user → 201 with user JSON
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_user_returns_201() {
    let app = build_admin_plane();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({
                        "external_id": "ext-123",
                        "provider": "google",
                        "email": "test@example.com",
                        "display_name": "Test User"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let json = body_to_json(response.into_body()).await;
    assert!(json["id"].as_str().unwrap().starts_with("usr_"));
    assert_eq!(json["external_id"], "ext-123");
    assert_eq!(json["provider"], "google");
    assert_eq!(json["email"], "test@example.com");
    assert_eq!(json["display_name"], "Test User");
    assert_eq!(json["status"], "active");
}

// ---------------------------------------------------------------------------
// 5. Get user → 200 with user JSON
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_user_returns_200() {
    let app = build_admin_plane();

    // First create a user
    let create_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({
                        "external_id": "ext-get",
                        "provider": "google",
                        "email": "get@example.com"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let created = body_to_json(create_resp.into_body()).await;
    let user_id = created["id"].as_str().unwrap();

    // Now get the user
    let get_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/internal/users/{}", user_id))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_resp.status(), StatusCode::OK);

    let json = body_to_json(get_resp.into_body()).await;
    assert_eq!(json["id"], user_id);
    assert_eq!(json["external_id"], "ext-get");
    assert_eq!(json["email"], "get@example.com");
}

// ---------------------------------------------------------------------------
// 6. Claims PATCH merge: create user, PATCH claims, GET → merged
// ---------------------------------------------------------------------------

#[tokio::test]
async fn claims_merge_works() {
    let app = build_admin_plane();

    // Create user
    let create_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({
                        "external_id": "ext-claims",
                        "provider": "google"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let created = body_to_json(create_resp.into_body()).await;
    let user_id = created["id"].as_str().unwrap();

    // PUT initial claims {"a": 1}
    let put_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/internal/users/{}/claims", user_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(json!({"a": 1}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(put_resp.status(), StatusCode::OK);

    // PATCH merge claims {"b": 2}
    let patch_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/internal/users/{}/claims", user_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(json!({"b": 2}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(patch_resp.status(), StatusCode::OK);

    // GET claims → should have both "a" and "b"
    let get_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/internal/users/{}/claims", user_id))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_resp.status(), StatusCode::OK);

    let claims = body_to_json(get_resp.into_body()).await;
    assert_eq!(claims["a"], 1);
    assert_eq!(claims["b"], 2);
}

// ---------------------------------------------------------------------------
// 7. Delete user → 200
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_user_returns_200() {
    let app = build_admin_plane();

    // Create user first
    let create_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({
                        "external_id": "ext-delete",
                        "provider": "google"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let created = body_to_json(create_resp.into_body()).await;
    let user_id = created["id"].as_str().unwrap();

    // Delete user
    let del_resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/internal/users/{}", user_id))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(del_resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// 8. Unknown user id on the mutating internal routes → 404 `not_found`,
// never a 500 `server_error` (negative-space: the pre-check must catch the
// typo before the adapter's `StoreError` backstop would fire).
// ---------------------------------------------------------------------------

const UNKNOWN_USER_ID: &str = "usr_does_not_exist";

#[tokio::test]
async fn update_user_unknown_id_returns_404_not_found() {
    let app = build_admin_plane();

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/internal/users/{}", UNKNOWN_USER_ID))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(json!({"display_name": "New Name"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

#[tokio::test]
async fn delete_user_unknown_id_returns_404_not_found() {
    let app = build_admin_plane();

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/internal/users/{}", UNKNOWN_USER_ID))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

#[tokio::test]
async fn get_claims_unknown_id_returns_404_not_found() {
    let app = build_admin_plane();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/internal/users/{}/claims", UNKNOWN_USER_ID))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

#[tokio::test]
async fn set_claims_unknown_id_returns_404_not_found() {
    let app = build_admin_plane();

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/internal/users/{}/claims", UNKNOWN_USER_ID))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(json!({"a": 1}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

#[tokio::test]
async fn merge_claims_unknown_id_returns_404_not_found() {
    let app = build_admin_plane();

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/internal/users/{}/claims", UNKNOWN_USER_ID))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(json!({"b": 2}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

#[tokio::test]
async fn clear_claims_unknown_id_returns_404_not_found() {
    let app = build_admin_plane();

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/internal/users/{}/claims", UNKNOWN_USER_ID))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

// ===========================================================================
// Bounded cursor pagination over the full admin plane (task 08)
// ===========================================================================

/// Create `count` users through the real admin route and return their ids.
async fn seed_users_via_api(app: &Router, count: usize, tag: &str) -> Vec<String> {
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/users")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {}", TEST_SECRET))
                    .body(Body::from(
                        json!({
                            "external_id": format!("{tag}-{i}"),
                            "provider": "google",
                            "email": format!("{tag}-{i}@example.com")
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let created = body_to_json(response.into_body()).await;
        ids.push(created["id"].as_str().unwrap().to_string());
    }
    ids
}

/// GET /internal/users with an optional query string. The body is parsed as
/// JSON when present — extractor-level rejections (e.g. a negative limit)
/// answer with a plain-text body, rendered here as JSON null.
async fn get_users_page(app: &Router, query: &str) -> (StatusCode, serde_json::Value) {
    let uri = if query.is_empty() {
        "/internal/users".to_string()
    } else {
        format!("/internal/users?{query}")
    };
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// The full contract traversal: pages of `limit` cover every created user
/// exactly once, terminate only at an explicit JSON-null `next_cursor`, and
/// a second independent traversal visits the same order.
#[tokio::test]
async fn list_users_pages_cover_every_user_exactly_once_until_null_cursor() {
    let app = build_admin_plane();
    let seeded = seed_users_via_api(&app, 7, "page").await;

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    let mut first_pass_page_ids: Vec<Vec<String>> = Vec::new();
    loop {
        pages += 1;
        assert!(pages < 100, "traversal must terminate");
        let query = match &cursor {
            Some(c) => format!("limit=3&cursor={}", urlencode(c)),
            None => "limit=3".to_string(),
        };
        let (status, body) = get_users_page(&app, &query).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.as_object()
                .expect("page is an object")
                .contains_key("next_cursor"),
            "next_cursor must be present in the JSON even when null (explicit null, not omitted)"
        );
        let rows: Vec<String> = body["users"]
            .as_array()
            .expect("users is an array")
            .iter()
            .map(|u| u["id"].as_str().unwrap().to_string())
            .collect();
        assert!(rows.len() <= 3, "a page never exceeds its limit");
        first_pass_page_ids.push(rows.clone());
        seen.extend(rows);
        cursor = body["next_cursor"].as_str().map(|s| s.to_string());
        if cursor.is_none() {
            break;
        }
    }

    assert_eq!(pages, 3, "7 users at limit 3 = pages of 3+3+1");
    assert_eq!(seen.len(), 7, "every user exactly once");
    assert_eq!(
        seen.iter().collect::<std::collections::HashSet<_>>().len(),
        7,
        "no duplicates across adjacent pages"
    );
    let mut sorted_seen = seen.clone();
    sorted_seen.sort();
    let mut sorted_seeded = seeded;
    sorted_seeded.sort();
    assert_eq!(
        sorted_seen, sorted_seeded,
        "the traversal covers the seed set"
    );

    // Ordering stability: an independent traversal visits the same sequence.
    let mut second_pass: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let query = match &cursor {
            Some(c) => format!("limit=3&cursor={}", urlencode(c)),
            None => "limit=3".to_string(),
        };
        let (_, body) = get_users_page(&app, &query).await;
        second_pass.extend(
            body["users"]
                .as_array()
                .unwrap()
                .iter()
                .map(|u| u["id"].as_str().unwrap().to_string()),
        );
        cursor = body["next_cursor"].as_str().map(|s| s.to_string());
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen, second_pass, "traversal order is deterministic");
}

/// `limit` absent resolves to the documented default of 50: with 51 users the
/// first page carries exactly 50 rows and a non-null cursor.
#[tokio::test]
async fn list_users_without_limit_serves_the_documented_default_page_size() {
    let app = build_admin_plane();
    seed_users_via_api(&app, 51, "default").await;

    let (status, body) = get_users_page(&app, "").await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["users"].as_array().unwrap();
    assert_eq!(rows.len(), 50, "the default page size is 50");
    assert_eq!(
        body["next_cursor"].as_str(),
        Some("x").map(|_| body["next_cursor"].as_str().unwrap()),
        "a full default page must carry a non-null next_cursor"
    );
}

/// Negative space on the query contract: a removed `offset` is refused with
/// `invalid_request` (never silently ignored), `limit=0` violates the
/// documented minimum, and a tampered cursor is `invalid_request` — all
/// deterministic 400s.
#[tokio::test]
async fn list_users_rejects_offset_zero_limit_and_tampered_cursor() {
    let app = build_admin_plane();
    seed_users_via_api(&app, 2, "neg").await;

    // Any offset — even offset=0 — is dead, not deprecated.
    let (status, body) = get_users_page(&app, "offset=0").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_request");
    assert!(
        body["error_description"]
            .as_str()
            .unwrap()
            .contains("cursor"),
        "the rejection must point the caller at cursor paging"
    );

    // Below the documented minimum.
    let (status, body) = get_users_page(&app, "limit=0").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_request");

    // A non-numeric limit is a malformed query, rejected by the extractor.
    let (status, _) = get_users_page(&app, "limit=-3").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A tampered cursor never silently restarts the listing.
    let (status, body) = get_users_page(&app, "cursor=garbage").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_request");

    // The positive path still works after all that negative space.
    let (status, body) = get_users_page(&app, "limit=1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["users"].as_array().unwrap().len(), 1);
}

/// An above-bound `limit` is clamped by the core, so the route accepts it and
/// serves a bounded page rather than erroring or materializing more than the
/// maximum.
#[tokio::test]
async fn list_users_accepts_and_serves_an_above_bound_limit() {
    let app = build_admin_plane();
    seed_users_via_api(&app, 3, "clamp").await;

    let (status, body) = get_users_page(&app, "limit=999999999").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "above-bound limits clamp, not error"
    );
    assert!(body["users"].as_array().unwrap().len() <= 3);
}

/// Percent-encoding round-trip: a cursor containing URL-significant
/// characters survives `encodeURIComponent`-style escaping in the query
/// string and still resolves.
#[tokio::test]
async fn list_users_cursor_survives_percent_encoding_round_trip() {
    let app = build_admin_plane();
    seed_users_via_api(&app, 4, "enc").await;

    let (_, first) = get_users_page(&app, "limit=2").await;
    let raw_cursor = first["next_cursor"].as_str().expect("more pages remain");

    // Simulate the console: percent-encode the cursor into a query value.
    let encoded: String = raw_cursor
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();

    let (status, second) = get_users_page(&app, &format!("limit=2&cursor={encoded}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an encoded cursor must decode cleanly"
    );
    assert_eq!(
        second["users"].as_array().unwrap().len(),
        2,
        "the second page serves the remaining rows"
    );
    assert!(
        second["next_cursor"].is_null(),
        "the listing is exhausted on the second page"
    );
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
