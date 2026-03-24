use async_trait::async_trait;
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use oidc_exchange_core::domain::provider::OidcProviderConfig;
use oidc_exchange_core::domain::{IdentityClaims, ProviderTokens};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::IdentityProvider;

use crate::shared::jwks::JwksCache;

/// Standard OIDC identity provider adapter (Tier 1 — e.g., Google).
///
/// Uses OIDC discovery, JWKS caching, and JWT validation to implement the
/// full `IdentityProvider` trait on top of the shared utilities.
pub struct OidcProvider {
    provider_id: String,
    client_id: String,
    client_secret: Option<String>,
    token_endpoint: String,
    jwks_cache: JwksCache,
    revocation_endpoint: Option<String>,
    issuer: String,
}

impl OidcProvider {
    /// Build an `OidcProvider` from an `OidcProviderConfig`.
    ///
    /// If `token_endpoint` or `jwks_uri` are absent from the config they are
    /// resolved via OIDC discovery on the configured `issuer`.
    pub async fn from_config(provider_id: &str, config: &OidcProviderConfig) -> Result<Self> {
        let discovery = if config.token_endpoint.is_some() && config.jwks_uri.is_some() {
            None
        } else {
            Some(crate::shared::discovery::discover(&config.issuer).await?)
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
            jwks_cache: JwksCache::new(jwks_uri),
            revocation_endpoint,
            issuer: config.issuer.clone(),
        })
    }
}

#[async_trait]
impl IdentityProvider for OidcProvider {
    async fn exchange_code(&self, code: &str, redirect_uri: &str) -> Result<ProviderTokens> {
        crate::shared::token_endpoint::exchange_code(
            &self.token_endpoint,
            &self.client_id,
            self.client_secret.as_deref(),
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

        // 2. Fetch JWKS (cached)
        let jwks = self.jwks_cache.get_keys().await?;

        // 3. Find matching key by kid
        let kid = header
            .kid
            .as_deref()
            .ok_or_else(|| Error::InvalidGrant {
                reason: "JWT missing kid header".into(),
            })?;

        let keys = jwks["keys"].as_array().ok_or_else(|| Error::ProviderError {
            provider: self.provider_id.clone(),
            detail: "JWKS response missing 'keys' array".into(),
        })?;

        let jwk = keys
            .iter()
            .find(|k| k["kid"].as_str() == Some(kid))
            .ok_or_else(|| Error::InvalidGrant {
                reason: format!("No matching key for kid: {kid}"),
            })?;

        // 4. Build decoding key from JWK
        let jwk_value: jsonwebtoken::jwk::Jwk =
            serde_json::from_value(jwk.clone()).map_err(|e| Error::InvalidGrant {
                reason: format!("Invalid JWK: {e}"),
            })?;

        let decoding_key =
            DecodingKey::from_jwk(&jwk_value).map_err(|e| Error::InvalidGrant {
                reason: format!("Cannot build decoding key from JWK: {e}"),
            })?;

        // 5. Configure validation
        let alg = header.alg;
        let mut validation = Validation::new(alg);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.client_id]);

        // 6. Decode and validate
        let token_data = decode::<serde_json::Value>(id_token, &decoding_key, &validation)
            .map_err(|e| Error::InvalidGrant {
                reason: format!("JWT validation failed: {e}"),
            })?;

        let claims = &token_data.claims;

        Ok(IdentityClaims {
            subject: claims["sub"].as_str().unwrap_or_default().to_string(),
            email: claims["email"].as_str().map(String::from),
            email_verified: claims["email_verified"].as_bool(),
            name: claims["name"].as_str().map(String::from),
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

        let client = reqwest::Client::new();
        let mut params = vec![("token", token)];

        // Include client credentials if available
        let client_id_owned = self.client_id.clone();
        params.push(("client_id", &client_id_owned));

        let response = client
            .post(endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|e| Error::ProviderError {
                provider: self.provider_id.clone(),
                detail: format!("Revocation request failed: {e}"),
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::ProviderError {
                provider: self.provider_id.clone(),
                detail: format!("Revocation returned {status}: {body}"),
            });
        }

        Ok(())
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
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

        let rsa_key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        let pkcs8_pem = rsa_key
            .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap();
        let encoding_key = EncodingKey::from_rsa_pem(pkcs8_pem.as_bytes()).unwrap();

        // Extract public key components for JWKS
        let public_key = rsa_key.to_public_key();
        let n = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
        let e = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());

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

    fn now_epoch() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn make_config(
        server_uri: &str,
        token_endpoint: Option<String>,
        jwks_uri: Option<String>,
        revocation_endpoint: Option<String>,
    ) -> OidcProviderConfig {
        OidcProviderConfig {
            provider_id: "test-provider".into(),
            issuer: server_uri.to_string(),
            client_id: "test-client-id".into(),
            client_secret: Some("test-client-secret".into()),
            jwks_uri,
            token_endpoint,
            revocation_endpoint,
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
        assert_eq!(
            tokens.refresh_token.as_deref(),
            Some("refresh-token-value")
        );
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
        assert_eq!(provider.token_endpoint, format!("{uri}/oauth/token"));
        assert_eq!(
            provider.revocation_endpoint.as_deref(),
            Some(format!("{uri}/oauth/revoke").as_str())
        );
    }
}
