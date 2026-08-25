use async_trait::async_trait;
use jsonwebtoken::{decode, decode_header, Algorithm, Validation};
use oidc_exchange_core::domain::provider::OidcProviderConfig;
use oidc_exchange_core::domain::{IdentityClaims, ProviderTokens};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::secret::Secret;

use crate::shared::claims::coerce_bool;
use crate::shared::jwks::JwksCache;
use crate::shared::origins::{parse_https_origin, EndpointOrigins, MAX_ENDPOINT_ORIGINS};

/// The algorithms the generic (Tier 1) provider admits for ID-token signatures:
/// the nine JWS signature algorithms this adapter has always accepted. Deliberately
/// a named per-provider policy — not derived from Apple's set and never unioned
/// with it; see task 04a ("keep provider-specific admitted algorithm policies
/// explicit") of the outbound-boundary plan.
pub const OIDC_ADMITTED_ALGORITHMS: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::EdDSA,
];

/// Standard OIDC identity provider adapter (Tier 1 — e.g., Google).
///
/// Uses OIDC discovery, JWKS caching, and JWT validation to implement the
/// full `IdentityProvider` trait on top of the shared utilities.
pub struct OidcProvider {
    provider_id: String,
    client_id: String,
    client_secret: Option<Secret<String>>,
    token_endpoint: oidc_exchange_core::config::HttpsUrl,
    jwks_cache: JwksCache,
    revocation_endpoint: Option<oidc_exchange_core::config::HttpsUrl>,
    issuer: oidc_exchange_core::config::HttpsUrl,
}

impl OidcProvider {
    /// Build an `OidcProvider` from an `OidcProviderConfig`.
    ///
    /// If `token_endpoint` or `jwks_uri` are absent from the config they are
    /// resolved via OIDC discovery on the configured `issuer`. Discovery runs
    /// against the provider's pinned endpoint-origin set — the issuer's own
    /// origin, the origins of explicitly configured endpoints, and every
    /// declared `endpoint_origins` entry — so a discovery document can never
    /// introduce an origin at runtime. The set is fixed here, at construction;
    /// nothing that happens later in the provider's life widens it.
    pub async fn from_config(provider_id: &str, config: &OidcProviderConfig) -> Result<Self> {
        // The adapter re-validates what the config layer validated: paired
        // checks at both ends of the boundary, because a future caller could
        // construct this config without passing through TOML loading.
        assert!(
            config.endpoint_origins.len() <= MAX_ENDPOINT_ORIGINS,
            "endpoint_origins exceeds MAX_ENDPOINT_ORIGINS"
        );
        for entry in &config.endpoint_origins {
            parse_https_origin(entry).map_err(|e| Error::ConfigError {
                detail: format!("provider '{provider_id}': invalid endpoint_origins entry: {e}"),
            })?;
        }

        let configured_endpoints: Vec<&str> = [
            config.token_endpoint.as_ref(),
            config.jwks_uri.as_ref(),
            config.revocation_endpoint.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(oidc_exchange_core::config::HttpsUrl::as_str)
        .collect();
        let permitted_origins = EndpointOrigins::from_parts(
            config.issuer.as_str(),
            &configured_endpoints,
            &config.endpoint_origins,
        );
        debug_assert!(
            permitted_origins.admits(config.issuer.as_str()),
            "the issuer's own origin is always a member of its pinned set"
        );

        let discovery = if config.token_endpoint.is_some() && config.jwks_uri.is_some() {
            None
        } else {
            Some(crate::shared::discovery::discover(&config.issuer, &permitted_origins).await?)
        };

        let token_endpoint = config
            .token_endpoint
            .clone()
            .or_else(|| discovery.as_ref().map(|d| d.token_endpoint.clone()))
            .ok_or_else(|| Error::ConfigError {
                detail: "token_endpoint not configured and discovery failed".into(),
            })?;

        let jwks_uri = config
            .jwks_uri
            .clone()
            .or_else(|| discovery.as_ref().map(|d| d.jwks_uri.clone()))
            .ok_or_else(|| Error::ConfigError {
                detail: "jwks_uri not configured and discovery failed".into(),
            })?;

        let revocation_endpoint = config
            .revocation_endpoint
            .clone()
            .or_else(|| discovery.and_then(|d| d.revocation_endpoint));

        Ok(Self {
            provider_id: provider_id.to_string(),
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            token_endpoint,
            jwks_cache: JwksCache::new(jwks_uri.as_str().to_string(), OIDC_ADMITTED_ALGORITHMS),
            revocation_endpoint,
            issuer: config.issuer.clone(),
        })
    }
}

#[async_trait]
impl IdentityProvider for OidcProvider {
    async fn exchange_code(&self, code: &str, redirect_uri: &str) -> Result<ProviderTokens> {
        crate::shared::token_endpoint::exchange_code(
            self.token_endpoint.as_str(),
            &self.client_id,
            // Reveal only at the outbound form-post boundary.
            self.client_secret.as_ref().map(|s| s.expose().as_str()),
            code,
            redirect_uri,
        )
        .await
    }

    async fn validate_id_token(&self, id_token: &str) -> Result<IdentityClaims> {
        // 1. Decode header to get kid + alg
        let header = decode_header(id_token).map_err(|e| Error::InvalidGrant {
            reason: format!("Invalid JWT header: {e}"),
        })?;

        let kid = header.kid.as_deref().ok_or_else(|| Error::InvalidGrant {
            reason: "JWT missing kid header".into(),
        })?;

        // 2. Resolve the kid through the cached key set. The cache owns the
        // miss path: on a miss it forces one rate-limited refetch and re-looks
        // up, so a rotated key is picked up immediately without waiting out the
        // TTL, and a kid that matches only ineligible entries fails closed
        // exactly like an absent one (invariant I3's shape).
        let verification_key = self.jwks_cache.get_key(kid).await?;
        debug_assert_eq!(
            verification_key.kid(),
            kid,
            "the resolved key is the one published under the requested kid"
        );

        // 3. Configure validation from the KEY SET, not from the token header:
        // the algorithm travels with the key it belongs to.
        let mut validation = Validation::new(verification_key.algorithm());
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_audience(&[&self.client_id]);
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        validation.validate_nbf = true;

        // 4. Decode and validate
        let token_data =
            decode::<serde_json::Value>(id_token, verification_key.decoding_key(), &validation)
                .map_err(|e| Error::InvalidGrant {
                    reason: format!("JWT validation failed: {e}"),
                })?;

        let claims = &token_data.claims;

        let subject = claims["sub"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::InvalidGrant {
                reason: "ID token missing required 'sub' claim".into(),
            })?
            .to_string();

        Ok(IdentityClaims {
            subject,
            email: claims["email"].as_str().map(String::from),
            email_verified: coerce_bool(&claims["email_verified"]),
            name: claims["name"].as_str().map(String::from),
            is_private_email: None,
            // The algorithm this token actually verified with (carried by the
            // resolved key-set entry), surfaced for the core's at_hash check.
            signing_alg: crate::shared::jwks::jws_alg_name(verification_key.algorithm())
                .to_string(),
            raw_claims: claims
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default(),
        })
    }

    async fn revoke_token(&self, token: &str) -> Result<()> {
        let endpoint = match &self.revocation_endpoint {
            Some(ep) => ep,
            None => return Ok(()), // Provider doesn't support revocation
        };

        let client_id_owned = self.client_id.clone();
        let params = vec![
            ("token".to_string(), token.to_string()),
            ("client_id".to_string(), client_id_owned),
        ];

        // The revocation POST goes through the shared transport: status before
        // body, bounded body, and the one redacting error-detail constructor —
        // an intermediary that echoes the submitted form cannot put the token
        // being revoked into the detail (and from there into a log line).
        let upstream = crate::shared::transport::ProviderTransport
            .post_form(&self.provider_id, endpoint.as_str(), &params)
            .await?;
        if !upstream.is_success() {
            return Err(upstream.error_into(&self.provider_id));
        }

        Ok(())
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn client_id(&self) -> &str {
        &self.client_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use p256::ecdsa::SigningKey;
    use p256::pkcs8::EncodePrivateKey;
    use serde_json::json;
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Helper: generate an RSA key pair, returning (encoding_key, jwks_json, kid).
    fn generate_rsa_test_keys() -> (EncodingKey, serde_json::Value, String) {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        use rsa::pkcs8::EncodePrivateKey;
        use rsa::traits::PublicKeyParts;

        let rsa_key = rsa::RsaPrivateKey::new(&mut rand::rng(), 2048).unwrap();
        let pkcs8_pem = rsa_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let encoding_key = EncodingKey::from_rsa_pem(pkcs8_pem.as_bytes()).unwrap();

        // Extract public key components for JWKS
        let public_key = rsa_key.to_public_key();
        let n_bytes = public_key.n().to_be_bytes();
        let e_bytes = public_key.e().to_be_bytes();
        let n = URL_SAFE_NO_PAD.encode(&n_bytes);
        let e = URL_SAFE_NO_PAD.encode(&e_bytes);

        let kid = "test-key-1".to_string();
        let jwks = json!({
            "keys": [{
                "kty": "RSA",
                "kid": &kid,
                "alg": "RS256",
                "use": "sig",
                "n": n,
                "e": e,
            }]
        });

        (encoding_key, jwks, kid)
    }

    /// Helper: generate an RSA key pair whose JWK carries no `alg` member, returning
    /// (encoding_key, jwks_json, kid).
    fn generate_rsa_test_keys_no_alg() -> (EncodingKey, serde_json::Value, String) {
        let (encoding_key, mut jwks, kid) = generate_rsa_test_keys();
        jwks["keys"][0].as_object_mut().unwrap().remove("alg");
        (encoding_key, jwks, kid)
    }

    /// Helper: generate an EC P-256 key pair, returning (encoding_key, jwks_json, kid).
    /// `with_alg` controls whether the JWK carries an explicit `alg: "ES256"` member.
    fn generate_es256_test_keys(with_alg: bool) -> (EncodingKey, serde_json::Value, String) {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        use p256::elliptic_curve::Generate;

        let signing_key = SigningKey::generate();
        let pem = signing_key
            .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
            .expect("PEM encoding should work");
        let encoding_key = EncodingKey::from_ec_pem(pem.as_bytes()).unwrap();

        let verifying_key = signing_key.verifying_key();
        let public_key = p256::PublicKey::from(verifying_key);
        let sec1_bytes = public_key.to_sec1_bytes();
        let x = URL_SAFE_NO_PAD.encode(&sec1_bytes[1..33]);
        let y = URL_SAFE_NO_PAD.encode(&sec1_bytes[33..65]);

        let kid = "test-ec-key-1".to_string();
        let mut jwk = json!({
            "kty": "EC",
            "kid": &kid,
            "use": "sig",
            "crv": "P-256",
            "x": x,
            "y": y,
        });
        if with_alg {
            jwk["alg"] = json!("ES256");
        }
        let jwks = json!({ "keys": [jwk] });

        (encoding_key, jwks, kid)
    }

    fn now_epoch() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn test_endpoint(value: impl Into<String>) -> oidc_exchange_core::config::HttpsUrl {
        oidc_exchange_core::config::HttpsUrl::parse_for_test(value)
            .expect("wiremock test fixture URL")
    }

    fn make_config(
        server_uri: &str,
        token_endpoint: Option<String>,
        jwks_uri: Option<String>,
        revocation_endpoint: Option<String>,
    ) -> OidcProviderConfig {
        OidcProviderConfig {
            provider_id: "test-provider".into(),
            issuer: test_endpoint(server_uri),
            client_id: "test-client-id".into(),
            client_secret: Some(Secret::new("test-client-secret".to_string())),
            jwks_uri: jwks_uri.map(test_endpoint),
            token_endpoint: token_endpoint.map(test_endpoint),
            revocation_endpoint: revocation_endpoint.map(test_endpoint),
            endpoint_origins: Vec::new(),
            scopes: vec!["openid".into()],
            additional_params: HashMap::new(),
        }
    }

    async fn mount_discovery(server: &MockServer, server_uri: &str) {
        let body = json!({
            "issuer": server_uri,
            "token_endpoint": format!("{server_uri}/oauth/token"),
            "jwks_uri": format!("{server_uri}/.well-known/jwks.json"),
            "revocation_endpoint": format!("{server_uri}/oauth/revoke"),
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(server)
            .await;
    }

    // ---------------------------------------------------------------
    // Test 1: Code exchange via mock token endpoint
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn exchange_code_returns_provider_tokens() {
        let server = MockServer::start().await;
        let uri = server.uri();

        mount_discovery(&server, &uri).await;

        let token_response = json!({
            "id_token": "id-token-value",
            "access_token": "access-token-value",
            "refresh_token": "refresh-token-value",
            "token_type": "Bearer",
            "expires_in": 3600
        });

        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=auth-code-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&token_response))
            .expect(1)
            .mount(&server)
            .await;

        let provider = OidcProvider::from_config("google", &make_config(&uri, None, None, None))
            .await
            .expect("from_config should succeed");

        let tokens = provider
            .exchange_code("auth-code-123", "https://example.com/callback")
            .await
            .expect("exchange_code should succeed");

        assert_eq!(tokens.id_token, "id-token-value");
        assert_eq!(tokens.access_token.as_deref(), Some("access-token-value"));
        assert_eq!(tokens.refresh_token.as_deref(), Some("refresh-token-value"));
    }

    // ---------------------------------------------------------------
    // Test 2: ID token validation with JWKS
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_succeeds_for_valid_jwt() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, kid) = generate_rsa_test_keys();

        // Mount JWKS endpoint
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let now = now_epoch();
        let claims = json!({
            "iss": &uri,
            "aud": "test-client-id",
            "sub": "user-123",
            "email": "user@example.com",
            "email_verified": true,
            "name": "Test User",
            "iat": now,
            "exp": now + 3600,
        });

        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(kid);

        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let identity = provider
            .validate_id_token(&id_token)
            .await
            .expect("validate_id_token should succeed");

        assert_eq!(identity.subject, "user-123");
        assert_eq!(identity.email.as_deref(), Some("user@example.com"));
        assert_eq!(identity.email_verified, Some(true));
        assert_eq!(identity.name.as_deref(), Some("Test User"));
        // Core-facing metadata: the JWK's verified algorithm, reported as data.
        assert_eq!(identity.signing_alg, "RS256");
        assert_eq!(
            provider.client_id(),
            "test-client-id",
            "the port must report the configured audience"
        );
        assert!(identity.raw_claims.contains_key("iss"));
    }

    // ---------------------------------------------------------------
    // Test 3: Expired token is rejected
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_rejects_expired_jwt() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, kid) = generate_rsa_test_keys();

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        // Create a JWT that expired 1 hour ago
        let now = now_epoch();
        let claims = json!({
            "iss": &uri,
            "aud": "test-client-id",
            "sub": "user-123",
            "iat": now - 7200,
            "exp": now - 3600,
        });

        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(kid);

        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let result = provider.validate_id_token(&id_token).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("JWT validation failed"),
            "Expected 'JWT validation failed' but got: {msg}"
        );
    }

    // ---------------------------------------------------------------
    // Test 4: Wrong audience is rejected
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_rejects_wrong_audience() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, kid) = generate_rsa_test_keys();

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let now = now_epoch();
        let claims = json!({
            "iss": &uri,
            "aud": "wrong-client-id",
            "sub": "user-123",
            "iat": now,
            "exp": now + 3600,
        });

        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(kid);

        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let result = provider.validate_id_token(&id_token).await;
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    // Test 5: Wrong issuer is rejected
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_rejects_wrong_issuer() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, kid) = generate_rsa_test_keys();

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let now = now_epoch();
        let claims = json!({
            "iss": "https://evil.example.com",
            "aud": "test-client-id",
            "sub": "user-123",
            "iat": now,
            "exp": now + 3600,
        });

        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(kid);

        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let result = provider.validate_id_token(&id_token).await;
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    // Test 6: Revoke token succeeds
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn revoke_token_posts_to_revocation_endpoint() {
        let server = MockServer::start().await;
        let uri = server.uri();

        Mock::given(method("POST"))
            .and(path("/oauth/revoke"))
            .and(body_string_contains("token=some-token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            Some(format!("{uri}/oauth/revoke")),
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        provider
            .revoke_token("some-token")
            .await
            .expect("revoke should succeed");
    }

    // ---------------------------------------------------------------
    // Test 7: Revoke is a no-op when no endpoint
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn revoke_token_is_noop_without_endpoint() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        // Should succeed without making any HTTP request
        provider
            .revoke_token("some-token")
            .await
            .expect("revoke should succeed as no-op");
    }

    // ---------------------------------------------------------------
    // Test 8: provider_id returns correct value
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn provider_id_returns_configured_id() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("my-google", &config)
            .await
            .expect("from_config should succeed");

        assert_eq!(provider.provider_id(), "my-google");
    }

    // ---------------------------------------------------------------
    // Test 9: from_config uses discovery when endpoints are absent
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn from_config_uses_discovery_for_missing_endpoints() {
        let server = MockServer::start().await;
        let uri = server.uri();

        mount_discovery(&server, &uri).await;

        let config = make_config(&uri, None, None, None);
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config with discovery should succeed");

        assert_eq!(provider.provider_id(), "google");
        assert_eq!(
            provider.token_endpoint.as_str(),
            format!("{uri}/oauth/token")
        );
        assert_eq!(
            provider
                .revocation_endpoint
                .as_ref()
                .map(oidc_exchange_core::config::HttpsUrl::as_str),
            Some(format!("{uri}/oauth/revoke").as_str())
        );
    }

    // ---------------------------------------------------------------
    // Origin pinning: discovery may confirm origins, never widen them.
    // The shipped mode is Warn (see origins::ENDPOINT_ORIGIN_CHECK_MODE);
    // these tests pin the shipped behaviour and the enforcement shape is
    // covered exhaustively by the mode-parameterised unit tests in
    // shared::origins.
    // ---------------------------------------------------------------

    /// Mount a discovery document on `server` whose endpoints live on a second,
    /// genuinely distinct loopback origin (a second mock server's port).
    async fn mount_cross_origin_discovery(server: &MockServer, cross_origin_base: &str) {
        let body = json!({
            "issuer": server.uri(),
            "token_endpoint": format!("{cross_origin_base}/oauth/token"),
            "jwks_uri": format!("{cross_origin_base}/.well-known/jwks.json"),
            "revocation_endpoint": format!("{cross_origin_base}/oauth/revoke"),
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn undeclared_cross_origin_discovery_is_served_in_warning_mode() {
        // Two servers give two genuinely different loopback origins (distinct ports).
        let issuer_server = MockServer::start().await;
        let cross_origin_server = MockServer::start().await;

        mount_cross_origin_discovery(&issuer_server, &cross_origin_server.uri()).await;

        // No endpoint_origins declared: the cross-origin document violates the
        // pinned set, but warning mode must not reject the deployment — it is
        // the one-release window operators use to learn what to declare.
        let mut config = make_config(&issuer_server.uri(), None, None, None);
        config.endpoint_origins.clear();

        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("warning mode must accept an undeclared cross-origin document");

        assert_eq!(
            provider.token_endpoint.as_str(),
            format!("{}/oauth/token", cross_origin_server.uri()),
            "the discovered endpoint is adopted unchanged under warning mode"
        );
    }

    #[tokio::test]
    async fn configured_cross_origin_jwks_admits_discovered_endpoints_on_that_origin() {
        // An explicitly configured endpoint's own origin joins the pinned set,
        // so a discovery document naming further endpoints on that origin is
        // admitted. The JWKS itself stays on the configured (loopback) URL;
        // declaring *new* origins is covered by the strict-parse unit tests in
        // shared::origins and Google's shape below, because declared entries
        // must be bare https origins and loopback test origins are plain http.
        let issuer_server = MockServer::start().await;
        let key_server = MockServer::start().await;

        mount_cross_origin_discovery(&issuer_server, &key_server.uri()).await;

        let (encoding_key, jwks, kid) = generate_rsa_test_keys();
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .expect(1) // The token's kid resolves in this set; no further fetch.
            .mount(&key_server)
            .await;

        let now = now_epoch();
        let claims = json!({
            "iss": &issuer_server.uri(),
            "aud": "test-client-id",
            "sub": "user-cross-origin",
            "iat": now,
            "exp": now + 3600,
        });
        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(kid);
        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        // jwks_uri configured explicitly (its origin therefore joins the pinned
        // set), token_endpoint left absent so discovery still runs.
        let config = make_config(
            &issuer_server.uri(),
            None,
            Some(format!("{}/.well-known/jwks.json", key_server.uri())),
            None,
        );

        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("cross-origin discovery over a configured endpoint's origin must be accepted");

        assert_eq!(
            provider.token_endpoint.as_str(),
            format!("{}/oauth/token", key_server.uri()),
            "the discovered token endpoint on the admitted origin is adopted"
        );

        let identity = provider
            .validate_id_token(&id_token)
            .await
            .expect("the jwks on the admitted origin must actually serve keys");

        assert_eq!(identity.subject, "user-cross-origin");
    }

    #[tokio::test]
    async fn google_multi_origin_discovery_shape_passes_when_all_origins_are_declared() {
        // Google publishes its token/revocation endpoints on oauth2.googleapis.com
        // and its JWKS on www.googleapis.com — two origins, neither of them the
        // issuer's. The fixture names those real-world origins in the document
        // while serving it from loopback; only the parsed strings are checked.
        let issuer_server = MockServer::start().await;
        let body = json!({
            "issuer": issuer_server.uri(),
            "token_endpoint": "https://oauth2.googleapis.com/token",
            "jwks_uri": "https://www.googleapis.com/oauth2/v3/certs",
            "revocation_endpoint": "https://oauth2.googleapis.com/revoke",
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&issuer_server)
            .await;

        let mut config = make_config(&issuer_server.uri(), None, None, None);
        config.endpoint_origins = vec![
            "https://oauth2.googleapis.com".to_string(),
            "https://www.googleapis.com".to_string(),
        ];

        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("Google's documented multi-origin shape must parse when declared");

        assert_eq!(
            provider.token_endpoint.as_str(),
            "https://oauth2.googleapis.com/token"
        );
        assert_eq!(
            provider
                .revocation_endpoint
                .as_ref()
                .map(oidc_exchange_core::config::HttpsUrl::as_str),
            Some("https://oauth2.googleapis.com/revoke"),
            "the discovered revocation endpoint is adopted from the declared document"
        );
    }

    #[tokio::test]
    async fn invalid_endpoint_origins_entries_are_rejected_at_the_adapter_boundary() {
        let issuer_server = MockServer::start().await;

        for bad in [
            "http://not-https.example",       // wrong scheme
            "https://path.example/with/path", // carries a path
            "https://q.example/?query=1",     // carries a query
            "garbage",                        // not a URL at all
        ] {
            let mut config = make_config(
                &issuer_server.uri(),
                Some(format!("{}/oauth/token", issuer_server.uri())),
                Some(format!("{}/.well-known/jwks.json", issuer_server.uri())),
                None,
            );
            config.endpoint_origins = vec![bad.to_string()];

            let err = OidcProvider::from_config("google", &config)
                .await
                .err()
                .expect("invalid declared entries must fail construction");

            assert!(
                matches!(err, Error::ConfigError { .. }),
                "entry {bad:?} must be a config error, got: {err:?}"
            );
        }
    }

    // ---------------------------------------------------------------
    // Test 10: ID token missing 'aud' claim is rejected
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_rejects_missing_aud() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, kid) = generate_rsa_test_keys();

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let now = now_epoch();
        // Deliberately omit 'aud' — e.g. a provider access token presented as an ID token.
        let claims = json!({
            "iss": &uri,
            "sub": "user-123",
            "iat": now,
            "exp": now + 3600,
        });

        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(kid);

        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let result = provider.validate_id_token(&id_token).await;
        assert!(result.is_err(), "token missing 'aud' must be rejected");
        assert!(
            matches!(result.unwrap_err(), Error::InvalidGrant { .. }),
            "missing 'aud' must be reported as InvalidGrant"
        );
    }

    // ---------------------------------------------------------------
    // Test 11: ID token missing 'iss' claim is rejected
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_rejects_missing_iss() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, kid) = generate_rsa_test_keys();

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let now = now_epoch();
        // Deliberately omit 'iss'.
        let claims = json!({
            "aud": "test-client-id",
            "sub": "user-123",
            "iat": now,
            "exp": now + 3600,
        });

        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(kid);

        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let result = provider.validate_id_token(&id_token).await;
        assert!(result.is_err(), "token missing 'iss' must be rejected");
        assert!(
            matches!(result.unwrap_err(), Error::InvalidGrant { .. }),
            "missing 'iss' must be reported as InvalidGrant"
        );
    }

    // ---------------------------------------------------------------
    // Test 12: ID token with a future 'nbf' claim is rejected
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_rejects_future_nbf() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, kid) = generate_rsa_test_keys();

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let now = now_epoch();
        let claims = json!({
            "iss": &uri,
            "aud": "test-client-id",
            "sub": "user-123",
            "iat": now,
            "nbf": now + 3600,
            "exp": now + 7200,
        });

        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(kid);

        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let result = provider.validate_id_token(&id_token).await;
        assert!(result.is_err(), "future 'nbf' must be rejected");
        assert!(
            matches!(result.unwrap_err(), Error::InvalidGrant { .. }),
            "future 'nbf' must be reported as InvalidGrant"
        );
    }

    // ---------------------------------------------------------------
    // Test 13: alg-less RSA JWK infers RS256 and validates
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_alg_less_rsa_jwk_infers_rs256() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, kid) = generate_rsa_test_keys_no_alg();
        assert!(
            jwks["keys"][0].get("alg").is_none(),
            "test fixture must omit 'alg'"
        );

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let now = now_epoch();
        let claims = json!({
            "iss": &uri,
            "aud": "test-client-id",
            "sub": "user-123",
            "iat": now,
            "exp": now + 3600,
        });

        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(kid);

        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let identity = provider
            .validate_id_token(&id_token)
            .await
            .expect("alg-less RSA JWK should validate as RS256");

        assert_eq!(identity.subject, "user-123");
        // The reported algorithm must be the one the JWK resolved to (explicit here),
        // not read back from the token header.
        assert_eq!(identity.signing_alg, "RS256");
        assert!(identity.raw_claims.contains_key("sub"));
    }

    // ---------------------------------------------------------------
    // Test 14: alg-less EC P-256 JWK infers ES256 and validates
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_alg_less_ec_p256_jwk_infers_es256() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, kid) = generate_es256_test_keys(false);
        assert!(
            jwks["keys"][0].get("alg").is_none(),
            "test fixture must omit 'alg'"
        );

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let now = now_epoch();
        let claims = json!({
            "iss": &uri,
            "aud": "test-client-id",
            "sub": "user-456",
            "iat": now,
            "exp": now + 3600,
        });

        let mut header = Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(kid);

        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let identity = provider
            .validate_id_token(&id_token)
            .await
            .expect("alg-less EC P-256 JWK should validate as ES256");

        assert_eq!(identity.subject, "user-456");
        // The JWK carries no `alg`, so this value can only have come from key-material
        // inference (kty EC + crv P-256 → ES256) — never from the header.
        assert_eq!(identity.signing_alg, "ES256");
        assert!(identity.raw_claims.contains_key("sub"));
    }

    // ---------------------------------------------------------------
    // Test 15: alg-less JWK of an unrecognised key type is rejected
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_rejects_unrecognised_alg_less_key() {
        let server = MockServer::start().await;
        let uri = server.uri();

        // An alg-less octet-sequence ("oct") key: not RSA/EC/OKP, so inference must fail.
        let kid = "test-oct-key-1".to_string();
        let jwks = json!({
            "keys": [{
                "kty": "oct",
                "kid": &kid,
                "use": "sig",
                "k": "c2VjcmV0LWtleS1tYXRlcmlhbA",
            }]
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        // We never get far enough to need a validly-signed token; the key lookup and
        // alg-inference happen before signature verification. An arbitrary (unsigned)
        // three-segment string with a matching 'kid' is enough to reach that code path.
        let header = json!({"alg": "HS256", "kid": &kid});
        let header_b64 = base64_url_encode(&serde_json::to_vec(&header).unwrap());
        let payload_b64 =
            base64_url_encode(&serde_json::to_vec(&json!({"sub": "user-1"})).unwrap());
        let id_token = format!("{header_b64}.{payload_b64}.sig");

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let result = provider.validate_id_token(&id_token).await;
        assert!(
            result.is_err(),
            "unrecognised alg-less key must be rejected"
        );
        assert!(
            matches!(result.unwrap_err(), Error::InvalidGrant { .. }),
            "unrecognised alg-less key must be reported as InvalidGrant"
        );
    }

    // ---------------------------------------------------------------
    // Test 15b: a header alg that disagrees with the JWK is rejected — the
    // verification (and the reported signing_alg) come from the JWK only
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_rejects_header_alg_mismatching_jwk() {
        let server = MockServer::start().await;
        let uri = server.uri();

        // The JWKS pins an RSA key declared RS256; the token below is genuinely signed
        // with an EC key but its header names the RSA JWK's kid, so the lookup resolves
        // to the RS256 JWK while the header claims ES256.
        let (_encoding_key, jwks, kid) = generate_rsa_test_keys();
        let (ec_encoding_key, _ec_jwks, _ec_kid) = generate_es256_test_keys(true);
        assert_eq!(jwks["keys"][0]["alg"], "RS256");

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let now = now_epoch();
        let claims = json!({
            "iss": &uri,
            "aud": "test-client-id",
            "sub": "user-123",
            "iat": now,
            "exp": now + 3600,
        });
        // Header claims ES256 while the resolved JWK pins RS256: the alg-confusion case.
        // Validation is configured from the JWK alone, so the decode must reject before
        // any signature check — the algorithm is never taken from the header.
        let mut header = Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(kid);
        let id_token = encode(&header, &claims, &ec_encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let result = provider.validate_id_token(&id_token).await;
        assert!(
            result.is_err(),
            "a header alg disagreeing with the JWK must never validate"
        );
        assert!(
            matches!(result.unwrap_err(), Error::InvalidGrant { .. }),
            "alg mismatch must be reported as InvalidGrant"
        );
    }

    // ---------------------------------------------------------------
    // Test 15c: client_id reports the configured audience through the port
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn client_id_returns_configured_audience() {
        let server = MockServer::start().await;
        let uri = server.uri();

        // Explicit endpoints keep this test off the network.
        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("my-google", &config)
            .await
            .expect("from_config should succeed");

        // The audience validation pins (set_audience) and the port's client_id() must be
        // the same configured value, so the core's azp check needs no config access.
        assert_eq!(provider.client_id(), "test-client-id");
        assert_eq!(provider.provider_id(), "my-google");
    }

    fn base64_url_encode(bytes: &[u8]) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        URL_SAFE_NO_PAD.encode(bytes)
    }

    // ---------------------------------------------------------------
    // Test 16: string 'email_verified' claim coerces to Some(true)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_coerces_string_email_verified() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, kid) = generate_rsa_test_keys();

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let now = now_epoch();
        let claims = json!({
            "iss": &uri,
            "aud": "test-client-id",
            "sub": "user-789",
            "email": "user@example.com",
            "email_verified": "true",
            "iat": now,
            "exp": now + 3600,
        });

        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(kid);

        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let identity = provider
            .validate_id_token(&id_token)
            .await
            .expect("well-formed token with string email_verified should validate");

        assert_eq!(identity.email_verified, Some(true));
        assert_eq!(identity.subject, "user-789");
        assert_eq!(identity.is_private_email, None);
    }

    // ---------------------------------------------------------------
    // Test: unknown kid triggers one rate-limited refetch, then validates
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_refetches_jwks_on_unknown_kid_then_validates() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, kid) = generate_rsa_test_keys();

        // The initially cached JWKS response omits the token's `kid` — simulating a signing
        // key that rotated in upstream after the cache was last populated.
        let stale_jwks = json!({ "keys": [] });

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&stale_jwks))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        // The forced refetch (triggered by the kid miss) serves the rotated key set that
        // contains the token's kid.
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .expect(1)
            .mount(&server)
            .await;

        let now = now_epoch();
        let claims = json!({
            "iss": &uri,
            "aud": "test-client-id",
            "sub": "user-rotated",
            "iat": now,
            "exp": now + 3600,
        });
        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(kid.clone());
        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        // No TTL sleep anywhere in this test: the rotated key must validate on the very
        // next call, driven entirely by the kid-miss forced refetch.
        let identity = provider
            .validate_id_token(&id_token)
            .await
            .expect("token should validate after one forced refetch picks up the rotated key");

        assert_eq!(identity.subject, "user-rotated");
        // wiremock's `expect(1)` on each of the two mounted mocks verifies exactly one
        // initial fetch and exactly one forced refetch occurred — not zero, not more (they
        // would panic on drop otherwise).
    }

    // ---------------------------------------------------------------
    // Test: kid still missing after the forced refetch is rejected, and the refetch
    // is rate-limited (negative-space test)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_rejects_kid_still_missing_after_refetch() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (encoding_key, jwks, _kid) = generate_rsa_test_keys();
        // Both the initial cache fill and the one permitted forced refetch return the same
        // set — the presented token's kid is never in it.
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .expect(2) // Initial fetch + exactly one forced refetch; a third call would panic.
            .mount(&server)
            .await;

        let now = now_epoch();
        let claims = json!({
            "iss": &uri,
            "aud": "test-client-id",
            "sub": "user-x",
            "iat": now,
            "exp": now + 3600,
        });
        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some("unknown-kid".to_string());
        let id_token = encode(&header, &claims, &encoding_key).unwrap();

        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            None,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");

        let result = provider.validate_id_token(&id_token).await;
        assert!(
            matches!(result, Err(Error::InvalidGrant { .. })),
            "kid absent even after a forced refetch must be rejected with InvalidGrant, \
             not a hang or a different error variant"
        );

        // A second call with the same unknown kid must not trigger a second forced refetch:
        // the JWKS endpoint's request budget is already exhausted by wiremock's `expect(2)`
        // above (mounting would panic on drop if a third GET request occurred), proving
        // MIN_REFRESH_INTERVAL held and no infinite refetch loop happened.
        let result2 = provider.validate_id_token(&id_token).await;
        assert!(
            matches!(result2, Err(Error::InvalidGrant { .. })),
            "repeated unknown kid must still fail closed without a new network fetch"
        );
    }

    // -------------------------------------------------------------------
    // Revocation boundary (plan task 05): a non-2xx revocation response is
    // read bounded and rendered only through upstream::error_detail, so an
    // intermediary echoing the submitted form — raw or percent-encoded —
    // cannot put the token being revoked into the detail that later reaches
    // an error log. Sentinels are obviously fake.
    // -------------------------------------------------------------------

    /// Provider wired to explicit endpoints on a fresh mock server; discovery is
    /// skipped by supplying every endpoint in the config. The caller mounts whichever
    /// revocation responses the test drives.
    async fn provider_with_revocation(revocation_path: Option<&str>) -> (OidcProvider, MockServer) {
        let server = MockServer::start().await;
        let uri = server.uri();
        let revocation_endpoint = revocation_path.map(|p| format!("{uri}{p}"));
        let config = make_config(
            &uri,
            Some(format!("{uri}/oauth/token")),
            Some(format!("{uri}/.well-known/jwks.json")),
            revocation_endpoint,
        );
        let provider = OidcProvider::from_config("google", &config)
            .await
            .expect("from_config should succeed");
        (provider, server)
    }

    #[tokio::test]
    async fn revoke_token_returns_ok_on_2xx() {
        let (provider, server) = provider_with_revocation(Some("/oauth/revoke")).await;

        Mock::given(method("POST"))
            .and(path("/oauth/revoke"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        provider
            .revoke_token("SENTINEL-REVOKE-TOKEN")
            .await
            .expect("a 2xx revocation must succeed");
    }

    #[tokio::test]
    async fn revoke_is_noop_without_endpoint() {
        let (provider, server) = provider_with_revocation(None).await;

        provider
            .revoke_token("whatever-token")
            .await
            .expect("without a revocation endpoint this is a documented no-op");

        let requests = server.received_requests().await.unwrap_or_default();
        assert!(
            requests.is_empty(),
            "the no-op path must not touch the network at all"
        );
    }

    #[tokio::test]
    async fn revoke_non_2xx_never_leaks_submitted_token_raw_or_encoded() {
        let (provider, server) = provider_with_revocation(Some("/oauth/revoke")).await;

        // Echo the submitted form back: once as a raw pair, once percent-encoded under
        // the same sensitive key. Both shapes decode to text containing the sentinel,
        // so both must be masked before the detail becomes loggable.
        let echo = "error=invalid_request&error_description=cannot revoke\
                    &token=SENTINEL-REVOKE-TOKEN-VALUE&token=1%2F%2FSENTINEL-REVOKE-TOKEN-VALUE";
        Mock::given(method("POST"))
            .and(path("/oauth/revoke"))
            .respond_with(ResponseTemplate::new(400).set_body_string(echo))
            .mount(&server)
            .await;

        let err = provider
            .revoke_token("SENTINEL-REVOKE-TOKEN-VALUE")
            .await
            .expect_err("a 400 revocation must fail");

        assert!(
            matches!(err, Error::ProviderError { .. }),
            "revocation failure must surface as ProviderError"
        );
        let message = err.to_string();
        assert!(
            !message.contains("SENTINEL-REVOKE-TOKEN-VALUE"),
            "echoed revoked token (raw or decoded) must never reach the detail, \
             got: {message}"
        );
    }

    #[tokio::test]
    async fn revoke_non_2xx_structured_error_stays_conformant_and_masked() {
        let (provider, server) = provider_with_revocation(Some("/oauth/revoke")).await;

        // Structured RFC 6749 content: the error code stays visible to operators while
        // an echoed pair inside the description is masked.
        let body = r#"{"error":"invalid_request","error_description":"rejected token=SENTINEL-STRUCT-ECHO"}"#;
        Mock::given(method("POST"))
            .and(path("/oauth/revoke"))
            .respond_with(ResponseTemplate::new(400).set_body_string(body))
            .mount(&server)
            .await;

        let message = provider
            .revoke_token("irrelevant-token")
            .await
            .expect_err("a 400 revocation must fail")
            .to_string();

        assert!(
            message.contains("invalid_request"),
            "structured OAuth error code must stay visible, got: {message}"
        );
        assert!(
            !message.contains("SENTINEL-STRUCT-ECHO"),
            "an echoed pair inside error_description must be masked, got: {message}"
        );
    }
}
