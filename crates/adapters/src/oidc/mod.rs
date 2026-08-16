use async_trait::async_trait;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use oidc_exchange_core::domain::provider::OidcProviderConfig;
use oidc_exchange_core::domain::{IdentityClaims, ProviderTokens};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::IdentityProvider;

use crate::shared::claims::coerce_bool;
use crate::shared::jwks::JwksCache;

/// Standard OIDC identity provider adapter (Tier 1 — e.g., Google).
///
/// Uses OIDC discovery, JWKS caching, and JWT validation to implement the
/// full `IdentityProvider` trait on top of the shared utilities.
pub struct OidcProvider {
    provider_id: String,
    client_id: String,
    client_secret: Option<String>,
    token_endpoint: oidc_exchange_core::config::HttpsUrl,
    jwks_cache: JwksCache,
    revocation_endpoint: Option<oidc_exchange_core::config::HttpsUrl>,
    issuer: oidc_exchange_core::config::HttpsUrl,
}

/// Infer the signing algorithm from a JWK that carries no `alg` member.
///
/// Azure-AD-style JWKS omit `alg`; the algorithm is then derived from the trusted
/// key material itself (`kty`, and `crv` for EC keys) rather than trusting the
/// untrusted JWT header. An alg-less RSA key is treated as RS256 (the RSA family is
/// not distinguishable from key parameters alone, and RS256 matches Azure AD's actual
/// signing algorithm). Any other alg-less key type is rejected.
fn infer_alg_from_jwk(jwk: &serde_json::Value) -> Result<Algorithm> {
    let kty = jwk.get("kty").and_then(|k| k.as_str());
    let crv = jwk.get("crv").and_then(|c| c.as_str());

    match (kty, crv) {
        (Some("RSA"), _) => Ok(Algorithm::RS256),
        (Some("EC"), Some("P-256")) => Ok(Algorithm::ES256),
        (Some("EC"), Some("P-384")) => Ok(Algorithm::ES384),
        (Some("OKP"), _) => Ok(Algorithm::EdDSA),
        _ => Err(Error::InvalidGrant {
            reason: "JWK has unsupported or missing algorithm".into(),
        }),
    }
}

/// Find the JWK matching `kid` inside a JWKS response's `keys` array.
///
/// Returns `Ok(None)` on a genuine `kid` miss (the caller decides whether that is terminal
/// or should trigger a forced refetch); errors if the response does not carry a `keys`
/// array at all (a malformed JWKS body, distinct from a miss).
fn find_jwk(
    provider_id: &str,
    jwks: &serde_json::Value,
    kid: &str,
) -> Result<Option<serde_json::Value>> {
    let keys = jwks["keys"]
        .as_array()
        .ok_or_else(|| Error::ProviderError {
            provider: provider_id.to_string(),
            detail: "JWKS response missing 'keys' array".into(),
        })?;
    Ok(keys
        .iter()
        .find(|k| k["kid"].as_str() == Some(kid))
        .cloned())
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
            jwks_cache: JwksCache::new(jwks_uri.as_str().to_string()),
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

        let kid = header.kid.as_deref().ok_or_else(|| Error::InvalidGrant {
            reason: "JWT missing kid header".into(),
        })?;

        // 2. Fetch JWKS (cached)
        let jwks = self.jwks_cache.get_keys().await?;

        // 3. Find matching key by kid. On a miss, force one rate-limited refetch (task 02's
        // `JwksCache::refresh` API, bounded by `MIN_REFRESH_INTERVAL`) and re-search the
        // refetched set before rejecting, so upstream key rotation is picked up immediately
        // instead of waiting out the (much longer) cache TTL.
        let jwk = match find_jwk(&self.provider_id, &jwks, kid)? {
            Some(jwk) => jwk,
            None => {
                self.jwks_cache.refresh().await?;
                let refreshed = self.jwks_cache.get_keys().await?;
                // Provider responses are adversarial (dev-guidelines §Defensive coding):
                // find_jwk fails closed with a ProviderError if `refreshed` is not a
                // `keys` array, so we validate rather than assert/panic on a 2xx body.
                find_jwk(&self.provider_id, &refreshed, kid)?.ok_or_else(|| {
                    Error::InvalidGrant {
                        reason: format!("No matching key for kid: {kid} (after forced refetch)"),
                    }
                })?
            }
        };
        assert_eq!(
            jwk["kid"].as_str(),
            Some(kid),
            "resolved JWK's kid must equal the header kid"
        );

        // 4. Build decoding key from JWK
        let jwk_value: jsonwebtoken::jwk::Jwk =
            serde_json::from_value(jwk.clone()).map_err(|e| Error::InvalidGrant {
                reason: format!("Invalid JWK: {e}"),
            })?;

        let decoding_key = DecodingKey::from_jwk(&jwk_value).map_err(|e| Error::InvalidGrant {
            reason: format!("Cannot build decoding key from JWK: {e}"),
        })?;

        // 5. Configure validation — use the algorithm from the JWK (trusted), not the JWT header (untrusted)
        let jwk_alg = jwk
            .get("alg")
            .and_then(|a| a.as_str())
            .and_then(|a| match a {
                "RS256" => Some(Algorithm::RS256),
                "RS384" => Some(Algorithm::RS384),
                "RS512" => Some(Algorithm::RS512),
                "ES256" => Some(Algorithm::ES256),
                "ES384" => Some(Algorithm::ES384),
                "PS256" => Some(Algorithm::PS256),
                "PS384" => Some(Algorithm::PS384),
                "PS512" => Some(Algorithm::PS512),
                "EdDSA" => Some(Algorithm::EdDSA),
                _ => None,
            })
            .map(Ok)
            .unwrap_or_else(|| infer_alg_from_jwk(&jwk))?;
        let mut validation = Validation::new(jwk_alg);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_audience(&[&self.client_id]);
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        validation.validate_nbf = true;

        // 6. Decode and validate
        let token_data = decode::<serde_json::Value>(id_token, &decoding_key, &validation)
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

        let client = crate::shared::http::client();
        let mut params = vec![("token", token)];

        // Include client credentials if available
        let client_id_owned = self.client_id.clone();
        params.push(("client_id", &client_id_owned));

        let response = client
            .post(endpoint.as_str())
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
            client_secret: Some("test-client-secret".into()),
            jwks_uri: jwks_uri.map(test_endpoint),
            token_endpoint: token_endpoint.map(test_endpoint),
            revocation_endpoint: revocation_endpoint.map(test_endpoint),
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
}
