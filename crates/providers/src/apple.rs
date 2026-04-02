use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use jsonwebtoken::{
    decode, decode_header, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use oidc_exchange_adapters::shared::jwks::JwksCache;
use oidc_exchange_core::domain::{IdentityClaims, ProviderTokens};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::IdentityProvider;
use serde::{Deserialize, Serialize};

const APPLE_ISSUER: &str = "https://appleid.apple.com";
const APPLE_JWKS_URI: &str = "https://appleid.apple.com/auth/keys";
const APPLE_TOKEN_ENDPOINT: &str = "https://appleid.apple.com/auth/token";
const APPLE_REVOCATION_ENDPOINT: &str = "https://appleid.apple.com/auth/revoke";

/// Client secret JWT lifetime: 5 minutes.
const CLIENT_SECRET_LIFETIME_SECS: u64 = 300;

/// Apple Sign-In identity provider.
///
/// Generates short-lived ES256 client JWTs (instead of a static `client_secret`)
/// for each token endpoint call, as required by Apple's OIDC implementation.
///
/// `EncodingKey` does not implement `Debug`, so we provide a manual implementation.
pub struct AppleProvider {
    client_id: String,
    team_id: String,
    key_id: String,
    signing_key: EncodingKey,
    token_endpoint: String,
    jwks_cache: JwksCache,
    revocation_endpoint: Option<String>,
}

impl std::fmt::Debug for AppleProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppleProvider")
            .field("client_id", &self.client_id)
            .field("team_id", &self.team_id)
            .field("key_id", &self.key_id)
            .field("signing_key", &"<redacted>")
            .field("token_endpoint", &self.token_endpoint)
            .field("revocation_endpoint", &self.revocation_endpoint)
            .finish()
    }
}

/// Claims for the client secret JWT sent to Apple's token endpoint.
#[derive(Debug, Serialize, Deserialize)]
struct ClientSecretClaims {
    iss: String,
    sub: String,
    aud: String,
    iat: u64,
    exp: u64,
}

impl AppleProvider {
    /// Build an `AppleProvider` from a raw TOML config map.
    ///
    /// Expected keys:
    /// - `client_id` — the Apple Services ID (e.g., "com.example.app")
    /// - `team_id` — Apple Developer Team ID
    /// - `key_id` — key identifier for the private key registered with Apple
    /// - `private_key_path` — filesystem path to the ES256 `.p8` private key
    pub async fn from_config(config: &HashMap<String, toml::Value>) -> Result<Self> {
        let client_id = config
            .get("client_id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| Error::ConfigError {
                detail: "apple: missing 'client_id'".into(),
            })?
            .to_string();

        let team_id = config
            .get("team_id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| Error::ConfigError {
                detail: "apple: missing 'team_id'".into(),
            })?
            .to_string();

        let key_id = config
            .get("key_id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| Error::ConfigError {
                detail: "apple: missing 'key_id'".into(),
            })?
            .to_string();

        let private_key_path = config
            .get("private_key_path")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| Error::ConfigError {
                detail: "apple: missing 'private_key_path'".into(),
            })?;

        let pem_bytes: Vec<u8> =
            tokio::fs::read(private_key_path)
                .await
                .map_err(|e| Error::ConfigError {
                    detail: format!("apple: failed to read private key at {private_key_path}: {e}"),
                })?;

        let signing_key =
            EncodingKey::from_ec_pem(&pem_bytes).map_err(|e| Error::ConfigError {
                detail: format!("apple: invalid ES256 private key: {e}"),
            })?;

        // Use well-known Apple endpoints (or discover them).
        // Apple's discovery document is stable, so we use the known values directly.
        let token_endpoint = config
            .get("token_endpoint")
            .and_then(toml::Value::as_str)
            .unwrap_or(APPLE_TOKEN_ENDPOINT)
            .to_string();

        let jwks_uri = config
            .get("jwks_uri")
            .and_then(toml::Value::as_str)
            .unwrap_or(APPLE_JWKS_URI)
            .to_string();

        let revocation_endpoint = Some(
            config
                .get("revocation_endpoint")
                .and_then(toml::Value::as_str)
                .unwrap_or(APPLE_REVOCATION_ENDPOINT)
                .to_string(),
        );

        Ok(Self {
            client_id,
            team_id,
            key_id,
            signing_key,
            token_endpoint,
            jwks_cache: JwksCache::new(jwks_uri),
            revocation_endpoint,
        })
    }

    /// Create an `AppleProvider` directly (useful for testing with injected endpoints).
    #[cfg(test)]
    fn new_for_test(
        client_id: String,
        team_id: String,
        key_id: String,
        signing_key: EncodingKey,
        token_endpoint: String,
        jwks_uri: String,
        revocation_endpoint: Option<String>,
    ) -> Self {
        Self {
            client_id,
            team_id,
            key_id,
            signing_key,
            token_endpoint,
            jwks_cache: JwksCache::new(jwks_uri),
            revocation_endpoint,
        }
    }

    /// Generate a short-lived ES256-signed client secret JWT for Apple's token endpoint.
    fn generate_client_secret(&self) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::ProviderError {
                provider: "apple".into(),
                detail: format!("system time error: {e}"),
            })?
            .as_secs();

        let claims = ClientSecretClaims {
            iss: self.team_id.clone(),
            sub: self.client_id.clone(),
            aud: APPLE_ISSUER.to_string(),
            iat: now,
            exp: now + CLIENT_SECRET_LIFETIME_SECS,
        };

        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.key_id.clone());

        encode(&header, &claims, &self.signing_key).map_err(|e| Error::ProviderError {
            provider: "apple".into(),
            detail: format!("failed to sign client secret JWT: {e}"),
        })
    }
}

#[async_trait]
impl IdentityProvider for AppleProvider {
    async fn exchange_code(&self, code: &str, redirect_uri: &str) -> Result<ProviderTokens> {
        let client_secret = self.generate_client_secret()?;

        oidc_exchange_adapters::shared::token_endpoint::exchange_code(
            &self.token_endpoint,
            &self.client_id,
            Some(&client_secret),
            code,
            redirect_uri,
        )
        .await
    }

    async fn validate_id_token(&self, id_token: &str) -> Result<IdentityClaims> {
        // 1. Decode header to find kid + alg
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
            provider: "apple".into(),
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

        // 5. Configure validation — derive algorithm from the trusted JWK, not the untrusted JWT header
        let jwk_alg = jwk.get("alg")
            .and_then(|a| a.as_str())
            .and_then(|a| match a {
                "RS256" => Some(Algorithm::RS256),
                "ES256" => Some(Algorithm::ES256),
                _ => None,
            })
            .ok_or_else(|| Error::InvalidGrant {
                reason: "Apple JWK has unsupported or missing algorithm".into(),
            })?;
        let mut validation = Validation::new(jwk_alg);
        validation.set_issuer(&[APPLE_ISSUER]);
        validation.set_audience(&[&self.client_id]);

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
            None => return Ok(()),
        };

        let client_secret = self.generate_client_secret()?;

        let client = reqwest::Client::new();
        let response = client
            .post(endpoint)
            .form(&[
                ("token", token),
                ("client_id", &self.client_id),
                ("client_secret", &client_secret),
                ("token_type_hint", "access_token"),
            ])
            .send()
            .await
            .map_err(|e| Error::ProviderError {
                provider: "apple".into(),
                detail: format!("Revocation request failed: {e}"),
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::ProviderError {
                provider: "apple".into(),
                detail: format!("Revocation returned {status}: {body}"),
            });
        }

        Ok(())
    }

    fn provider_id(&self) -> &str {
        "apple"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode as jwt_encode, Header as JwtHeader};
    use p256::ecdsa::SigningKey;
    use p256::pkcs8::EncodePrivateKey;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Generate an ES256 key pair for testing.
    /// Returns (encoding_key_pem, jwks_json, kid).
    fn generate_es256_test_keys() -> (Vec<u8>, serde_json::Value, String) {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

        use p256::elliptic_curve::Generate;
        let signing_key = SigningKey::generate();
        let pem = signing_key
            .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
            .expect("PEM encoding should work");

        // Extract the public key for JWKS
        let verifying_key = signing_key.verifying_key();
        // Extract raw public key bytes (uncompressed SEC1: 04 || x || y, 65 bytes for P-256)
        let public_key = p256::PublicKey::from(verifying_key);
        let sec1_bytes = public_key.to_sec1_bytes();
        // Skip the 0x04 prefix byte, split into x (32 bytes) and y (32 bytes)
        let x = URL_SAFE_NO_PAD.encode(&sec1_bytes[1..33]);
        let y = URL_SAFE_NO_PAD.encode(&sec1_bytes[33..65]);

        let kid = "apple-test-key-1".to_string();
        let jwks = json!({
            "keys": [{
                "kty": "EC",
                "kid": &kid,
                "alg": "ES256",
                "use": "sig",
                "crv": "P-256",
                "x": x,
                "y": y,
            }]
        });

        (pem.as_bytes().to_vec(), jwks, kid)
    }

    fn now_epoch() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn make_test_provider(
        encoding_key: &[u8],
        token_endpoint: &str,
        jwks_uri: &str,
        revocation_endpoint: Option<String>,
    ) -> AppleProvider {
        let key = EncodingKey::from_ec_pem(encoding_key).expect("valid EC PEM");
        AppleProvider::new_for_test(
            "com.example.app".into(),
            "ABCDEF1234".into(),
            "apple-test-key-1".into(),
            key,
            token_endpoint.into(),
            jwks_uri.into(),
            revocation_endpoint,
        )
    }

    // ---------------------------------------------------------------
    // Test 1: Client JWT generation — correct claims and header
    // ---------------------------------------------------------------
    #[test]
    fn generate_client_secret_has_correct_claims() {
        let (pem, _jwks, _kid) = generate_es256_test_keys();
        let provider = make_test_provider(
            &pem,
            "https://appleid.apple.com/auth/token",
            "https://appleid.apple.com/auth/keys",
            None,
        );

        let secret = provider
            .generate_client_secret()
            .expect("should generate client secret");

        // Decode header (unverified) to check kid + alg
        let header = decode_header(&secret).expect("valid JWT header");
        assert_eq!(header.alg, Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("apple-test-key-1"));

        // Decode claims by manually parsing the JWT payload (no signature verification needed)
        let parts: Vec<&str> = secret.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT should have 3 parts");
        use base64::Engine as _;
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .expect("valid base64url payload");
        let claims: ClientSecretClaims =
            serde_json::from_slice(&payload_bytes).expect("valid JSON claims");

        assert_eq!(claims.iss, "ABCDEF1234");
        assert_eq!(claims.sub, "com.example.app");
        assert_eq!(claims.aud, "https://appleid.apple.com");

        let now = now_epoch();
        assert!(claims.iat <= now);
        assert!(claims.exp > now);
        assert!(claims.exp <= now + CLIENT_SECRET_LIFETIME_SECS + 1);
    }

    // ---------------------------------------------------------------
    // Test 2: Client secret JWT is verifiable with the corresponding public key
    // ---------------------------------------------------------------
    #[test]
    fn generate_client_secret_is_verifiable() {
        let (pem, jwks, _kid) = generate_es256_test_keys();
        let provider = make_test_provider(
            &pem,
            "https://appleid.apple.com/auth/token",
            "https://appleid.apple.com/auth/keys",
            None,
        );

        let secret = provider
            .generate_client_secret()
            .expect("should generate client secret");

        // Build a decoding key from the JWKS
        let key_json = &jwks["keys"][0];
        let jwk: jsonwebtoken::jwk::Jwk =
            serde_json::from_value(key_json.clone()).expect("valid JWK");
        let decoding_key = DecodingKey::from_jwk(&jwk).expect("valid decoding key");

        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_audience(&["https://appleid.apple.com"]);
        validation.set_issuer(&["ABCDEF1234"]);

        let token_data = decode::<ClientSecretClaims>(&secret, &decoding_key, &validation)
            .expect("signature should verify");

        assert_eq!(token_data.claims.sub, "com.example.app");
    }

    // ---------------------------------------------------------------
    // Test 3: provider_id returns "apple"
    // ---------------------------------------------------------------
    #[test]
    fn provider_id_returns_apple() {
        let (pem, _jwks, _kid) = generate_es256_test_keys();
        let provider = make_test_provider(
            &pem,
            "https://appleid.apple.com/auth/token",
            "https://appleid.apple.com/auth/keys",
            None,
        );

        assert_eq!(provider.provider_id(), "apple");
    }

    // ---------------------------------------------------------------
    // Test 4: Full exchange flow — exchange_code + validate_id_token
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn exchange_and_validate_flow() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (pem, jwks, kid) = generate_es256_test_keys();
        let encoding_key = EncodingKey::from_ec_pem(&pem).expect("valid EC PEM");

        // Create an ID token signed with the test key (simulating what Apple would return)
        let now = now_epoch();
        let id_claims = json!({
            "iss": "https://appleid.apple.com",
            "aud": "com.example.app",
            "sub": "apple-user-001",
            "email": "user@privaterelay.appleid.com",
            "email_verified": true,
            "iat": now,
            "exp": now + 3600,
        });

        let mut id_header = JwtHeader::new(Algorithm::ES256);
        id_header.kid = Some(kid);
        let id_token = jwt_encode(&id_header, &id_claims, &encoding_key)
            .expect("should encode ID token");

        // Mount mock token endpoint
        let token_response = json!({
            "id_token": &id_token,
            "access_token": "apple-access-token",
            "refresh_token": "apple-refresh-token",
            "token_type": "Bearer",
            "expires_in": 3600
        });

        Mock::given(method("POST"))
            .and(path("/auth/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=apple-auth-code"))
            .and(body_string_contains("client_secret="))
            .respond_with(ResponseTemplate::new(200).set_body_json(&token_response))
            .expect(1)
            .mount(&server)
            .await;

        // Mount mock JWKS endpoint
        Mock::given(method("GET"))
            .and(path("/auth/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let provider = AppleProvider::new_for_test(
            "com.example.app".into(),
            "ABCDEF1234".into(),
            "apple-test-key-1".into(),
            EncodingKey::from_ec_pem(&pem).unwrap(),
            format!("{uri}/auth/token"),
            format!("{uri}/auth/keys"),
            Some(format!("{uri}/auth/revoke")),
        );

        // Step 1: Exchange code
        let tokens = provider
            .exchange_code("apple-auth-code", "https://example.com/callback")
            .await
            .expect("exchange_code should succeed");

        assert_eq!(tokens.id_token, id_token);
        assert_eq!(tokens.access_token.as_deref(), Some("apple-access-token"));
        assert_eq!(
            tokens.refresh_token.as_deref(),
            Some("apple-refresh-token")
        );

        // Step 2: Validate the ID token
        let identity = provider
            .validate_id_token(&tokens.id_token)
            .await
            .expect("validate_id_token should succeed");

        assert_eq!(identity.subject, "apple-user-001");
        assert_eq!(
            identity.email.as_deref(),
            Some("user@privaterelay.appleid.com")
        );
        assert_eq!(identity.email_verified, Some(true));
    }

    // ---------------------------------------------------------------
    // Test 5: Revoke token sends correct parameters
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn revoke_token_posts_with_client_secret() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (pem, _jwks, _kid) = generate_es256_test_keys();

        Mock::given(method("POST"))
            .and(path("/auth/revoke"))
            .and(body_string_contains("token=some-refresh-token"))
            .and(body_string_contains("client_id=com.example.app"))
            .and(body_string_contains("client_secret="))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let provider = AppleProvider::new_for_test(
            "com.example.app".into(),
            "ABCDEF1234".into(),
            "apple-test-key-1".into(),
            EncodingKey::from_ec_pem(&pem).unwrap(),
            format!("{uri}/auth/token"),
            format!("{uri}/auth/keys"),
            Some(format!("{uri}/auth/revoke")),
        );

        provider
            .revoke_token("some-refresh-token")
            .await
            .expect("revoke should succeed");
    }

    // ---------------------------------------------------------------
    // Test 6: Revoke is a no-op when no endpoint
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn revoke_token_is_noop_without_endpoint() {
        let (pem, _jwks, _kid) = generate_es256_test_keys();

        let provider = AppleProvider::new_for_test(
            "com.example.app".into(),
            "ABCDEF1234".into(),
            "apple-test-key-1".into(),
            EncodingKey::from_ec_pem(&pem).unwrap(),
            "https://appleid.apple.com/auth/token".into(),
            "https://appleid.apple.com/auth/keys".into(),
            None,
        );

        provider
            .revoke_token("some-token")
            .await
            .expect("revoke should succeed as no-op");
    }

    // ---------------------------------------------------------------
    // Test 7: from_config rejects missing fields
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn from_config_rejects_missing_client_id() {
        let config = HashMap::new();
        let result = AppleProvider::from_config(&config).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("client_id"), "Expected client_id error: {err}");
    }

    #[tokio::test]
    async fn from_config_rejects_missing_team_id() {
        let mut config = HashMap::new();
        config.insert(
            "client_id".into(),
            toml::Value::String("com.example.app".into()),
        );
        let result = AppleProvider::from_config(&config).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("team_id"), "Expected team_id error: {err}");
    }
}
