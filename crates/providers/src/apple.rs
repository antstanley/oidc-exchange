use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use jsonwebtoken::{decode, decode_header, encode, Algorithm, EncodingKey, Header, Validation};
use oidc_exchange_adapters::shared::claims::coerce_bool;
use oidc_exchange_adapters::shared::jwks::JwksCache;
use oidc_exchange_adapters::shared::origins::{
    origin_of, parse_https_origin, EndpointOrigins, MAX_ENDPOINT_ORIGINS,
    MAX_ENDPOINT_ORIGIN_LEN_BYTES,
};
use oidc_exchange_core::config::HttpsUrl;
use oidc_exchange_core::domain::{IdentityClaims, ProviderTokens};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::Secret;
use serde::{Deserialize, Serialize};

const APPLE_ISSUER: &str = "https://appleid.apple.com";
const APPLE_JWKS_URI: &str = "https://appleid.apple.com/auth/keys";
const APPLE_TOKEN_ENDPOINT: &str = "https://appleid.apple.com/auth/token";
const APPLE_REVOCATION_ENDPOINT: &str = "https://appleid.apple.com/auth/revoke";

/// Client secret JWT lifetime: 5 minutes.
const CLIENT_SECRET_LIFETIME_SECS: u64 = 300;

/// The algorithms Apple's validator admits for ID-token signatures: the two
/// algorithms Apple's own tokens have always used. Deliberately a named
/// per-provider policy, narrower than the generic adapter's nine — consolidating
/// the selector must not widen Apple to the OIDC union (task 04a of the
/// outbound-boundary plan).
pub const APPLE_ADMITTED_ALGORITHMS: &[Algorithm] = &[Algorithm::RS256, Algorithm::ES256];

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
    token_endpoint: HttpsUrl,
    jwks_cache: JwksCache,
    revocation_endpoint: Option<HttpsUrl>,
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

/// Extract and validate the optional `endpoint_origins` array from Apple's raw
/// TOML config map.
///
/// Every entry must be a bare `https` origin — the same strict rule the config
/// layer applies to Tier 1 providers — because these entries declare what a
/// discovery document (or a future Apple endpoint relocation) may name.
fn parse_declared_endpoint_origins(config: &HashMap<String, toml::Value>) -> Result<Vec<String>> {
    let Some(raw) = config.get("endpoint_origins") else {
        return Ok(Vec::new());
    };

    let Some(entries) = raw.as_array() else {
        return Err(Error::ConfigError {
            detail: "apple: 'endpoint_origins' must be an array of https origins".into(),
        });
    };

    assert!(
        entries.len() <= MAX_ENDPOINT_ORIGINS,
        "endpoint_origins exceeds MAX_ENDPOINT_ORIGINS"
    );

    entries
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let Some(entry) = value.as_str() else {
                return Err(Error::ConfigError {
                    detail: format!("apple: endpoint_origins[{index}] must be a string"),
                });
            };
            if entry.len() > MAX_ENDPOINT_ORIGIN_LEN_BYTES {
                // Rejected before any parse so an oversized entry can never
                // become log or error text; the message names only the index.
                return Err(Error::ConfigError {
                    detail: format!(
                        "apple: endpoint_origins[{index}] exceeds \
                         {MAX_ENDPOINT_ORIGIN_LEN_BYTES} bytes"
                    ),
                });
            }
            parse_https_origin(entry).map_err(|e| Error::ConfigError {
                detail: format!("apple: invalid endpoint_origins[{index}]: {e}"),
            })
        })
        .collect()
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

        let signing_key = EncodingKey::from_ec_pem(&pem_bytes).map_err(|e| Error::ConfigError {
            detail: format!("apple: invalid ES256 private key: {e}"),
        })?;

        // Use well-known Apple endpoints (or discover them).
        // Apple's discovery document is stable, so we use the known values directly.
        let endpoint = |name: &str, default: &str| {
            HttpsUrl::parse(
                config
                    .get(name)
                    .and_then(toml::Value::as_str)
                    .unwrap_or(default),
            )
            .map_err(|_| Error::ConfigError {
                detail: format!("apple: {name} must be a non-empty HTTPS URL"),
            })
        };

        let token_endpoint = endpoint("token_endpoint", APPLE_TOKEN_ENDPOINT)?;
        let jwks_uri = endpoint("jwks_uri", APPLE_JWKS_URI)?;
        let revocation_endpoint = Some(endpoint("revocation_endpoint", APPLE_REVOCATION_ENDPOINT)?);

        // Endpoint-origin pinning, same shape as a Tier 1 provider: the pinned
        // set is the issuer's own origin (the `appleid.apple.com` constant),
        // the origins of any explicitly configured overrides, and every
        // declared `endpoint_origins` entry. The overrides are operator input
        // here, so each one must at minimum parse as an absolute URL — an
        // override that pins no origin would otherwise silently escape the
        // set. Apple performs no runtime discovery, so the set is a
        // construction-time invariant rather than a per-fetch check; it is
        // asserted below instead of being skipped.
        let declared_extras = parse_declared_endpoint_origins(config)?;

        let overrides: Vec<&str> = [
            Some(token_endpoint.as_str()),
            Some(jwks_uri.as_str()),
            revocation_endpoint.as_ref().map(HttpsUrl::as_str),
        ]
        .into_iter()
        .flatten()
        .collect();
        for endpoint in &overrides {
            if origin_of(endpoint).is_none() {
                return Err(Error::ConfigError {
                    detail: "apple: endpoint override is not an absolute URL".into(),
                });
            }
        }

        let pinned_origins =
            EndpointOrigins::from_parts(APPLE_ISSUER, &overrides, &declared_extras);
        for endpoint in &overrides {
            debug_assert!(
                pinned_origins.admits(endpoint),
                "an explicitly configured endpoint's own origin is always admitted"
            );
        }

        Ok(Self {
            client_id,
            team_id,
            key_id,
            signing_key,
            token_endpoint,
            jwks_cache: JwksCache::new(jwks_uri.as_str().to_string(), APPLE_ADMITTED_ALGORITHMS),
            revocation_endpoint,
        })
    }

    /// Create an `AppleProvider` directly (useful for testing with injected
    /// endpoints). Hidden test seam: integration suites (the upstream leak
    /// corpus) need wiremock endpoints that the strict `from_config` HTTPS
    /// validation rightly refuses.
    #[doc(hidden)]
    pub fn new_for_test(
        client_id: String,
        team_id: String,
        key_id: String,
        signing_key: EncodingKey,
        token_endpoint: HttpsUrl,
        jwks_uri: HttpsUrl,
        revocation_endpoint: Option<HttpsUrl>,
    ) -> Self {
        Self {
            client_id,
            team_id,
            key_id,
            signing_key,
            token_endpoint,
            jwks_cache: JwksCache::new(jwks_uri.as_str().to_string(), APPLE_ADMITTED_ALGORITHMS),
            revocation_endpoint,
        }
    }

    /// Generate a short-lived ES256-signed client secret JWT for Apple's token endpoint.
    ///
    /// Returns the assertion wrapped as `Secret<String>`: it is a freshly minted bearer
    /// credential, so it can be posted but never formatted — the only legitimate use is
    /// `expose()` at the outbound form boundary.
    fn generate_client_secret(&self) -> Result<Secret<String>> {
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

        encode(&header, &claims, &self.signing_key)
            .map(Secret::new)
            .map_err(|e| Error::ProviderError {
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
            self.token_endpoint.as_str(),
            &self.client_id,
            // Reveal the freshly signed assertion only at the outbound form boundary.
            Some(client_secret.expose().as_str()),
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

        let kid = header.kid.as_deref().ok_or_else(|| Error::InvalidGrant {
            reason: "JWT missing kid header".into(),
        })?;

        // 2. Resolve the kid through the cached key set (shared `VerificationKeySet`,
        // built with Apple's admitted-algorithm set). The cache owns the miss path:
        // one rate-limited refetch, then fail closed — a rotated Apple signing key is
        // picked up immediately instead of waiting out the TTL.
        let verification_key = self.jwks_cache.get_key(kid).await?;
        debug_assert_eq!(
            verification_key.kid(),
            kid,
            "the resolved key is the one published under the requested kid"
        );

        // 3. Configure validation from the KEY SET, not from the token header.
        let mut validation = Validation::new(verification_key.algorithm());
        validation.set_issuer(&[APPLE_ISSUER]);
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
            is_private_email: coerce_bool(&claims["is_private_email"]),
            // The algorithm this token actually verified with, carried by the
            // resolved key-set entry (Apple pins ES256 today; the key decides,
            // not the header).
            signing_alg: oidc_exchange_adapters::shared::jwks::jws_alg_name(
                verification_key.algorithm(),
            )
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
            None => return Ok(()),
        };

        let client_secret = self.generate_client_secret()?;

        let params = vec![
            ("token".to_string(), token.to_string()),
            ("client_id".to_string(), self.client_id.clone()),
            // The assertion is revealed only inside the outbound form body.
            ("client_secret".to_string(), client_secret.expose().clone()),
            ("token_type_hint".to_string(), "access_token".to_string()),
        ];

        // The revocation POST goes through the shared transport: status before
        // body, bounded body, and the one redacting error-detail constructor —
        // neither the token being revoked, nor a hostile echo of it, nor the
        // posted assertion can reach the detail (and from there an error log).
        let upstream = oidc_exchange_adapters::shared::transport::ProviderTransport
            .post_form("apple", endpoint.as_str(), &params)
            .await?;
        if !upstream.is_success() {
            return Err(upstream.error_into("apple"));
        }

        Ok(())
    }

    fn provider_id(&self) -> &str {
        "apple"
    }

    fn client_id(&self) -> &str {
        &self.client_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{
        decode as jwt_decode, encode as jwt_encode, DecodingKey, Header as JwtHeader,
    };
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
            HttpsUrl::parse_for_test(token_endpoint).expect("test token URL"),
            HttpsUrl::parse_for_test(jwks_uri).expect("test JWKS URL"),
            revocation_endpoint
                .map(HttpsUrl::parse_for_test)
                .transpose()
                .expect("test revocation URL"),
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

        // The assertion is a Secret now; unwrap it deliberately for the test's
        // inspection only.
        let secret = secret.into_inner();

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
            .expect("should generate client secret")
            .into_inner();

        // Build a decoding key from the JWKS
        let key_json = &jwks["keys"][0];
        let jwk: jsonwebtoken::jwk::Jwk =
            serde_json::from_value(key_json.clone()).expect("valid JWK");
        let decoding_key = DecodingKey::from_jwk(&jwk).expect("valid decoding key");

        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_audience(&["https://appleid.apple.com"]);
        validation.set_issuer(&["ABCDEF1234"]);

        let token_data = jwt_decode::<ClientSecretClaims>(&secret, &decoding_key, &validation)
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
        let id_token =
            jwt_encode(&id_header, &id_claims, &encoding_key).expect("should encode ID token");

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
            HttpsUrl::parse_for_test(format!("{uri}/auth/token")).expect("wiremock URL"),
            HttpsUrl::parse_for_test(format!("{uri}/auth/keys")).expect("wiremock URL"),
            Some(HttpsUrl::parse_for_test(format!("{uri}/auth/revoke")).expect("wiremock URL")),
        );

        // Step 1: Exchange code
        let tokens = provider
            .exchange_code("apple-auth-code", "https://example.com/callback")
            .await
            .expect("exchange_code should succeed");

        assert_eq!(tokens.id_token, id_token);
        assert_eq!(tokens.access_token.as_deref(), Some("apple-access-token"));
        assert_eq!(tokens.refresh_token.as_deref(), Some("apple-refresh-token"));

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
        // Core-facing metadata: the JWK's verified algorithm, reported as data.
        assert_eq!(identity.signing_alg, "ES256");
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
            HttpsUrl::parse_for_test(format!("{uri}/auth/token")).expect("wiremock URL"),
            HttpsUrl::parse_for_test(format!("{uri}/auth/keys")).expect("wiremock URL"),
            Some(HttpsUrl::parse_for_test(format!("{uri}/auth/revoke")).expect("wiremock URL")),
        );

        provider
            .revoke_token("some-refresh-token")
            .await
            .expect("revoke should succeed");
    }

    // ---------------------------------------------------------------
    // Test 6b: Revocation non-2xx is bounded + redacted — neither the token
    // being revoked nor the generated client assertion can reach the error
    // detail, raw or percent-encoded (plan task 05).
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn revoke_non_2xx_never_leaks_token_or_generated_assertion() {
        let server = MockServer::start().await;
        let uri = server.uri();
        let (pem, _jwks, _kid) = generate_es256_test_keys();
        let token_endpoint_uri = format!("{uri}/auth/token");
        let jwks_uri = format!("{uri}/auth/keys");
        let revoke_uri = format!("{uri}/auth/revoke");

        let provider = make_test_provider(&pem, &token_endpoint_uri, &jwks_uri, Some(revoke_uri));

        // Phase 1: drive one failing revoke to capture the assertion the provider
        // actually signed — wiremock records the request bodies it received.
        Mock::given(method("POST"))
            .and(path("/auth/revoke"))
            .respond_with(ResponseTemplate::new(400).set_body_string("rejected"))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        assert!(
            provider.revoke_token("SENTINEL-APPLE-TOKEN").await.is_err(),
            "the phase-1 400 must fail"
        );
        let requests = server
            .received_requests()
            .await
            .expect("request recording must be available");
        assert!(!requests.is_empty(), "phase 1 must have hit the mock");

        // Pull the client_secret pair out of the recorded form body.
        let form = String::from_utf8(requests[0].body.clone()).expect("form body is UTF-8");
        let assertion: String = form
            .split('&')
            .find_map(|pair| pair.strip_prefix("client_secret="))
            .map(String::from)
            .filter(|v| !v.is_empty())
            .expect("phase-1 request must carry a generated client_secret");

        // Phase 2: echo the sensitive material back — token raw and percent-encoded,
        // plus the real captured assertion.
        let echo = format!(
            "token=SENTINEL-APPLE-TOKEN&client_secret={assertion}\
             &echo=token%3D1%2F%2FSENTINEL-APPLE-TOKEN"
        );
        Mock::given(method("POST"))
            .and(path("/auth/revoke"))
            .respond_with(ResponseTemplate::new(400).set_body_string(echo))
            .mount(&server)
            .await;

        let err = provider
            .revoke_token("SENTINEL-APPLE-TOKEN")
            .await
            .expect_err("a 400 echo must fail");

        let message = err.to_string();
        assert!(
            !message.contains("SENTINEL-APPLE-TOKEN"),
            "revoked token (raw or decoded) must never reach the detail, got: {message}"
        );
        assert!(
            !message.contains(&assertion),
            "the generated client assertion must never reach the detail, got: {message}"
        );
    }

    #[tokio::test]
    async fn revoke_non_2xx_structured_error_stays_visible_and_masked() {
        let server = MockServer::start().await;
        let uri = server.uri();
        let (pem, _jwks, _kid) = generate_es256_test_keys();
        let token_endpoint_uri = format!("{uri}/auth/token");
        let jwks_uri = format!("{uri}/auth/keys");
        let revoke_uri = format!("{uri}/auth/revoke");

        let provider = make_test_provider(&pem, &token_endpoint_uri, &jwks_uri, Some(revoke_uri));

        // Structured RFC 6749 content: the error code stays visible to operators while
        // an echoed pair inside the description is masked.
        let body = r#"{"error":"invalid_request","error_description":"rejected token=SENTINEL-STRUCT-ECHO"}"#;
        Mock::given(method("POST"))
            .and(path("/auth/revoke"))
            .respond_with(ResponseTemplate::new(400).set_body_string(body))
            .mount(&server)
            .await;

        let message = provider
            .revoke_token("irrelevant")
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
            HttpsUrl::parse("https://appleid.apple.com/auth/token").expect("HTTPS token URL"),
            HttpsUrl::parse("https://appleid.apple.com/auth/keys").expect("HTTPS JWKS URL"),
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

    #[tokio::test]
    async fn from_config_rejects_http_endpoint_override() {
        let (pem_bytes, _jwks, _kid) = generate_es256_test_keys();
        let pem = tempfile::NamedTempFile::new().expect("temporary PEM file");
        std::fs::write(pem.path(), pem_bytes).expect("write PEM");
        let mut config = HashMap::from([
            (
                "client_id".into(),
                toml::Value::String("com.example.app".into()),
            ),
            ("team_id".into(), toml::Value::String("TEAMID".into())),
            ("key_id".into(), toml::Value::String("KEYID".into())),
            (
                "private_key_path".into(),
                toml::Value::String(pem.path().display().to_string()),
            ),
            (
                "token_endpoint".into(),
                toml::Value::String("http://apple.example/token".into()),
            ),
        ]);

        let err = AppleProvider::from_config(&config)
            .await
            .expect_err("HTTP Apple override must be rejected");
        assert!(err.to_string().contains("token_endpoint"));
        config.remove("token_endpoint");
    }

    // ---------------------------------------------------------------
    // Tests 8-13: validate_id_token hardening — required claims, nbf,
    // and bool-or-string coercion of email_verified / is_private_email
    // ---------------------------------------------------------------

    /// Offset (seconds) used to place a test token's `nbf` in the future.
    const TEST_FUTURE_NBF_OFFSET_SECS: u64 = 3600;

    /// Start a mock server serving the given JWKS at `/auth/keys` and return an
    /// `AppleProvider` wired to it (token endpoint is unused by these tests).
    async fn provider_with_mock_jwks(
        pem: &[u8],
        jwks: &serde_json::Value,
    ) -> (AppleProvider, MockServer) {
        let server = MockServer::start().await;
        let uri = server.uri();

        Mock::given(method("GET"))
            .and(path("/auth/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(jwks))
            .mount(&server)
            .await;

        let provider = AppleProvider::new_for_test(
            "com.example.app".into(),
            "ABCDEF1234".into(),
            "apple-test-key-1".into(),
            EncodingKey::from_ec_pem(pem).expect("valid EC PEM"),
            HttpsUrl::parse_for_test(format!("{uri}/auth/token")).expect("wiremock URL"),
            HttpsUrl::parse_for_test(format!("{uri}/auth/keys")).expect("wiremock URL"),
            None,
        );

        (provider, server)
    }

    /// Sign the given claims into an ES256 ID token using the test key and kid.
    fn sign_id_token(pem: &[u8], kid: &str, claims: &serde_json::Value) -> String {
        let encoding_key = EncodingKey::from_ec_pem(pem).expect("valid EC PEM");
        let mut header = JwtHeader::new(Algorithm::ES256);
        header.kid = Some(kid.to_string());
        jwt_encode(&header, claims, &encoding_key).expect("should encode ID token")
    }

    #[tokio::test]
    async fn validate_id_token_rejects_missing_aud() {
        let (pem, jwks, kid) = generate_es256_test_keys();
        let (provider, _server) = provider_with_mock_jwks(&pem, &jwks).await;

        let now = now_epoch();
        let claims = json!({
            "iss": "https://appleid.apple.com",
            "sub": "apple-user-002",
            "iat": now,
            "exp": now + 3600,
        });
        let id_token = sign_id_token(&pem, &kid, &claims);

        let result = provider.validate_id_token(&id_token).await;
        assert!(result.is_err(), "token missing 'aud' must be rejected");
        assert!(
            matches!(result.unwrap_err(), Error::InvalidGrant { .. }),
            "rejection must be Error::InvalidGrant"
        );
    }

    #[tokio::test]
    async fn validate_id_token_rejects_missing_iss() {
        let (pem, jwks, kid) = generate_es256_test_keys();
        let (provider, _server) = provider_with_mock_jwks(&pem, &jwks).await;

        let now = now_epoch();
        let claims = json!({
            "aud": "com.example.app",
            "sub": "apple-user-003",
            "iat": now,
            "exp": now + 3600,
        });
        let id_token = sign_id_token(&pem, &kid, &claims);

        let result = provider.validate_id_token(&id_token).await;
        assert!(result.is_err(), "token missing 'iss' must be rejected");
        assert!(
            matches!(result.unwrap_err(), Error::InvalidGrant { .. }),
            "rejection must be Error::InvalidGrant"
        );
    }

    #[tokio::test]
    async fn validate_id_token_rejects_future_nbf() {
        let (pem, jwks, kid) = generate_es256_test_keys();
        let (provider, _server) = provider_with_mock_jwks(&pem, &jwks).await;

        let now = now_epoch();
        let claims = json!({
            "iss": "https://appleid.apple.com",
            "aud": "com.example.app",
            "sub": "apple-user-004",
            "iat": now,
            "nbf": now + TEST_FUTURE_NBF_OFFSET_SECS,
            "exp": now + TEST_FUTURE_NBF_OFFSET_SECS + 3600,
        });
        let id_token = sign_id_token(&pem, &kid, &claims);

        let result = provider.validate_id_token(&id_token).await;
        assert!(result.is_err(), "token with future 'nbf' must be rejected");
        assert!(
            matches!(result.unwrap_err(), Error::InvalidGrant { .. }),
            "rejection must be Error::InvalidGrant"
        );
    }

    #[tokio::test]
    async fn validate_id_token_coerces_string_email_verified() {
        let (pem, jwks, kid) = generate_es256_test_keys();
        let (provider, _server) = provider_with_mock_jwks(&pem, &jwks).await;

        let now = now_epoch();
        let claims = json!({
            "iss": "https://appleid.apple.com",
            "aud": "com.example.app",
            "sub": "apple-user-005",
            "email": "user@example.com",
            "email_verified": "true",
            "iat": now,
            "exp": now + 3600,
        });
        let id_token = sign_id_token(&pem, &kid, &claims);

        let identity = provider
            .validate_id_token(&id_token)
            .await
            .expect("well-formed token should validate");

        assert_eq!(identity.email_verified, Some(true));
        assert_eq!(identity.email.as_deref(), Some("user@example.com"));
    }

    #[tokio::test]
    async fn validate_id_token_coerces_string_is_private_email() {
        let (pem, jwks, kid) = generate_es256_test_keys();
        let (provider, _server) = provider_with_mock_jwks(&pem, &jwks).await;

        let now = now_epoch();
        let claims = json!({
            "iss": "https://appleid.apple.com",
            "aud": "com.example.app",
            "sub": "apple-user-006",
            "is_private_email": "true",
            "iat": now,
            "exp": now + 3600,
        });
        let id_token = sign_id_token(&pem, &kid, &claims);

        let identity = provider
            .validate_id_token(&id_token)
            .await
            .expect("well-formed token should validate");

        assert_eq!(identity.is_private_email, Some(true));
        assert_eq!(identity.subject, "apple-user-006");
    }

    #[tokio::test]
    async fn validate_id_token_coerces_bool_is_private_email() {
        let (pem, jwks, kid) = generate_es256_test_keys();
        let (provider, _server) = provider_with_mock_jwks(&pem, &jwks).await;

        let now = now_epoch();
        let claims = json!({
            "iss": "https://appleid.apple.com",
            "aud": "com.example.app",
            "sub": "apple-user-007",
            "is_private_email": true,
            "iat": now,
            "exp": now + 3600,
        });
        let id_token = sign_id_token(&pem, &kid, &claims);

        let identity = provider
            .validate_id_token(&id_token)
            .await
            .expect("well-formed token should validate");

        assert_eq!(identity.is_private_email, Some(true));
        assert_eq!(identity.subject, "apple-user-007");
    }

    // ---------------------------------------------------------------
    // Unknown kid triggers one rate-limited refetch, then validates
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_refetches_jwks_on_unknown_kid_then_validates() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (pem, jwks, kid) = generate_es256_test_keys();

        // The initially cached JWKS response omits the token's `kid` — simulating an Apple
        // signing key that rotated after the cache was last populated.
        let stale_jwks = json!({ "keys": [] });

        Mock::given(method("GET"))
            .and(path("/auth/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&stale_jwks))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        // The forced refetch (triggered by the kid miss) serves the rotated key set that
        // contains the token's kid.
        Mock::given(method("GET"))
            .and(path("/auth/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .expect(1)
            .mount(&server)
            .await;

        let provider = AppleProvider::new_for_test(
            "com.example.app".into(),
            "ABCDEF1234".into(),
            "apple-test-key-1".into(),
            EncodingKey::from_ec_pem(&pem).expect("valid EC PEM"),
            HttpsUrl::parse_for_test(format!("{uri}/auth/token")).expect("wiremock URL"),
            HttpsUrl::parse_for_test(format!("{uri}/auth/keys")).expect("wiremock URL"),
            None,
        );

        let now = now_epoch();
        let claims = json!({
            "iss": "https://appleid.apple.com",
            "aud": "com.example.app",
            "sub": "apple-user-rotated",
            "iat": now,
            "exp": now + 3600,
        });
        let id_token = sign_id_token(&pem, &kid, &claims);

        // No TTL sleep anywhere in this test: the rotated key must validate on the very
        // next call, driven entirely by the kid-miss forced refetch.
        let identity = provider
            .validate_id_token(&id_token)
            .await
            .expect("token should validate after one forced refetch picks up the rotated key");

        assert_eq!(identity.subject, "apple-user-rotated");
        // wiremock's `expect(1)` on each of the two mounted mocks verifies exactly one
        // initial fetch and exactly one forced refetch occurred — not zero, not more (they
        // would panic on drop otherwise).
    }

    // ---------------------------------------------------------------
    // kid still missing after the forced refetch is rejected, and the refetch is
    // rate-limited (negative-space test)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_rejects_kid_still_missing_after_refetch() {
        let server = MockServer::start().await;
        let uri = server.uri();

        let (pem, jwks, _kid) = generate_es256_test_keys();
        // Both the initial cache fill and the one permitted forced refetch return the same
        // set — the presented token's kid is never in it.
        Mock::given(method("GET"))
            .and(path("/auth/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .expect(2) // Initial fetch + exactly one forced refetch; a third call would panic.
            .mount(&server)
            .await;

        let provider = AppleProvider::new_for_test(
            "com.example.app".into(),
            "ABCDEF1234".into(),
            "apple-test-key-1".into(),
            EncodingKey::from_ec_pem(&pem).expect("valid EC PEM"),
            HttpsUrl::parse_for_test(format!("{uri}/auth/token")).expect("wiremock URL"),
            HttpsUrl::parse_for_test(format!("{uri}/auth/keys")).expect("wiremock URL"),
            None,
        );

        let now = now_epoch();
        let claims = json!({
            "iss": "https://appleid.apple.com",
            "aud": "com.example.app",
            "sub": "apple-user-x",
            "iat": now,
            "exp": now + 3600,
        });
        let id_token = sign_id_token(&pem, "unknown-kid", &claims);

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

    // ---------------------------------------------------------------
    // A validated token reports the JWK's algorithm as signing_alg — the
    // core-facing data its at_hash digest selection reads
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn validate_id_token_reports_jwk_signing_algorithm() {
        let (pem, jwks, kid) = generate_es256_test_keys();
        let (provider, _server) = provider_with_mock_jwks(&pem, &jwks).await;

        let now = now_epoch();
        let claims = json!({
            "iss": "https://appleid.apple.com",
            "aud": "com.example.app",
            "sub": "apple-user-alg",
            "email": "alg@example.com",
            "iat": now,
            "exp": now + 3600,
        });
        let id_token = sign_id_token(&pem, &kid, &claims);

        let identity = provider
            .validate_id_token(&id_token)
            .await
            .expect("well-formed token should validate");

        // The reported algorithm must equal the matched JWK's `alg` member and be a
        // faithful JWS name — never copied from (or confusable with) the header.
        assert_eq!(jwks["keys"][0]["alg"], "ES256");
        assert_eq!(identity.signing_alg, "ES256");
        assert_eq!(identity.subject, "apple-user-alg");
    }

    #[tokio::test]
    async fn validate_id_token_rejects_header_alg_mismatching_jwk() {
        let server = MockServer::start().await;
        let uri = server.uri();

        // The JWKS declares RS256 for the only key; the token is genuinely ES256-signed
        // but presents that key's kid. Validation is configured from the trusted JWK
        // alone, so the decode must reject before any signature check.
        let rsa_jwks = json!({
            "keys": [{
                "kty": "RSA",
                "kid": "apple-test-key-1",
                "alg": "RS256",
                "use": "sig",
                "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
                "e": "AQAB"
            }]
        });

        Mock::given(method("GET"))
            .and(path("/auth/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&rsa_jwks))
            .mount(&server)
            .await;

        let (pem, _jwks, _kid) = generate_es256_test_keys();
        let provider = AppleProvider::new_for_test(
            "com.example.app".into(),
            "ABCDEF1234".into(),
            "apple-test-key-1".into(),
            EncodingKey::from_ec_pem(&pem).expect("valid EC PEM"),
            HttpsUrl::parse_for_test(format!("{uri}/auth/token")).expect("wiremock URL"),
            HttpsUrl::parse_for_test(format!("{uri}/auth/keys")).expect("wiremock URL"),
            None,
        );

        let now = now_epoch();
        let claims = json!({
            "iss": "https://appleid.apple.com",
            "aud": "com.example.app",
            "sub": "apple-user-confusion",
            "iat": now,
            "exp": now + 3600,
        });
        let id_token = sign_id_token(&pem, "apple-test-key-1", &claims);

        let result = provider.validate_id_token(&id_token).await;
        assert!(
            result.is_err(),
            "a header alg disagreeing with the Apple JWK must never validate"
        );
        assert!(
            matches!(result.unwrap_err(), Error::InvalidGrant { .. }),
            "alg mismatch must be reported as InvalidGrant"
        );
    }

    // ---------------------------------------------------------------
    // client_id reports the configured Services ID through the port
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn client_id_returns_configured_services_id() {
        let (pem, _jwks, _kid) = generate_es256_test_keys();

        let provider = AppleProvider::new_for_test(
            "com.example.app".into(),
            "ABCDEF1234".into(),
            "apple-test-key-1".into(),
            EncodingKey::from_ec_pem(&pem).expect("valid EC PEM"),
            HttpsUrl::parse_for_test("https://appleid.apple.com/auth/token").expect("static URL"),
            HttpsUrl::parse_for_test("https://appleid.apple.com/auth/keys").expect("static URL"),
            None,
        );

        // The audience validation pins and the port's client_id() must be the same
        // configured value, so the core's azp check needs no config access.
        assert_eq!(provider.client_id(), "com.example.app");
        assert_eq!(provider.provider_id(), "apple");
    }
}
