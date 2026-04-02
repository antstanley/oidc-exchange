use async_trait::async_trait;
use aws_sdk_kms::primitives::Blob;
use aws_sdk_kms::types::{MessageType, SigningAlgorithmSpec};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::KeyManager;
use rsa::pkcs8::DecodePublicKey;
use rsa::traits::PublicKeyParts;

/// AWS KMS-backed key manager that uses the KMS Sign API for JWT signing.
pub struct KmsKeyManager {
    client: aws_sdk_kms::Client,
    key_id: String,
    algorithm: String,
    kid: String,
    /// Cached public key JWK (fetched once from KMS GetPublicKey)
    public_key: tokio::sync::OnceCell<serde_json::Value>,
}

impl KmsKeyManager {
    pub fn new(
        client: aws_sdk_kms::Client,
        key_id: String,
        algorithm: String,
        kid: String,
    ) -> Self {
        Self {
            client,
            key_id,
            algorithm,
            kid,
            public_key: tokio::sync::OnceCell::new(),
        }
    }

    /// Parse the algorithm string into the AWS SDK enum.
    fn signing_algorithm(&self) -> Result<SigningAlgorithmSpec> {
        match self.algorithm.as_str() {
            "RS256" => Ok(SigningAlgorithmSpec::RsassaPkcs1V15Sha256),
            "RS384" => Ok(SigningAlgorithmSpec::RsassaPkcs1V15Sha384),
            "RS512" => Ok(SigningAlgorithmSpec::RsassaPkcs1V15Sha512),
            "PS256" => Ok(SigningAlgorithmSpec::RsassaPssSha256),
            "PS384" => Ok(SigningAlgorithmSpec::RsassaPssSha384),
            "PS512" => Ok(SigningAlgorithmSpec::RsassaPssSha512),
            "ES256" => Ok(SigningAlgorithmSpec::EcdsaSha256),
            "ES384" => Ok(SigningAlgorithmSpec::EcdsaSha384),
            "ES512" => Ok(SigningAlgorithmSpec::EcdsaSha512),
            other => Err(Error::KeyError {
                detail: format!("unsupported KMS signing algorithm: {other}"),
            }),
        }
    }

    /// Fetch the public key from KMS and build an RFC 7517 compliant JWK.
    async fn fetch_public_jwk(&self) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get_public_key()
            .key_id(&self.key_id)
            .send()
            .await
            .map_err(|e| Error::KeyError {
                detail: format!("KMS GetPublicKey failed: {e}"),
            })?;

        let public_key_der = resp
            .public_key()
            .ok_or_else(|| Error::KeyError {
                detail: "KMS GetPublicKey response missing public_key field".to_string(),
            })?
            .as_ref();

        parse_spki_to_jwk(public_key_der, &self.algorithm, &self.kid)
    }
}

/// Parse a DER-encoded SubjectPublicKeyInfo into an RFC 7517 JWK JSON value.
///
/// Supports RSA (RS256/384/512, PS256/384/512) and EC (ES256, ES384) keys.
fn parse_spki_to_jwk(spki_der: &[u8], algorithm: &str, kid: &str) -> Result<serde_json::Value> {
    match algorithm {
        a if a.starts_with("RS") || a.starts_with("PS") => {
            let public_key =
                rsa::RsaPublicKey::from_public_key_der(spki_der).map_err(|e| Error::KeyError {
                    detail: format!("failed to parse RSA public key DER: {e}"),
                })?;

            let n = URL_SAFE_NO_PAD.encode(public_key.n().to_be_bytes());
            let e = URL_SAFE_NO_PAD.encode(public_key.e().to_be_bytes());

            Ok(serde_json::json!({
                "kty": "RSA",
                "alg": algorithm,
                "use": "sig",
                "kid": kid,
                "n": n,
                "e": e,
            }))
        }
        "ES256" | "ES384" => {
            // EC keys in SPKI DER contain an uncompressed SEC1 point: 0x04 || x || y
            let (crv, coord_len) = match algorithm {
                "ES256" => ("P-256", 32),
                "ES384" => ("P-384", 48),
                _ => unreachable!(),
            };

            let point_len = 1 + 2 * coord_len;
            if spki_der.len() < point_len {
                return Err(Error::KeyError {
                    detail: format!(
                        "SPKI DER too short for {crv}: expected at least {point_len} bytes, got {}",
                        spki_der.len()
                    ),
                });
            }

            let point = &spki_der[spki_der.len() - point_len..];
            if point[0] != 0x04 {
                return Err(Error::KeyError {
                    detail: format!(
                        "expected uncompressed EC point (0x04 prefix), got 0x{:02x}",
                        point[0]
                    ),
                });
            }

            let x = URL_SAFE_NO_PAD.encode(&point[1..1 + coord_len]);
            let y = URL_SAFE_NO_PAD.encode(&point[1 + coord_len..]);

            Ok(serde_json::json!({
                "kty": "EC",
                "crv": crv,
                "alg": algorithm,
                "use": "sig",
                "kid": kid,
                "x": x,
                "y": y,
            }))
        }
        other => Err(Error::KeyError {
            detail: format!("unsupported algorithm for JWK generation: {other}"),
        }),
    }
}

#[async_trait]
impl KeyManager for KmsKeyManager {
    async fn sign(&self, payload: &[u8]) -> Result<Vec<u8>> {
        let algorithm = self.signing_algorithm()?;

        let resp = self
            .client
            .sign()
            .key_id(&self.key_id)
            .signing_algorithm(algorithm)
            .message_type(MessageType::Raw)
            .message(Blob::new(payload))
            .send()
            .await
            .map_err(|e| Error::KeyError {
                detail: format!("KMS Sign failed: {e}"),
            })?;

        let signature = resp
            .signature()
            .ok_or_else(|| Error::KeyError {
                detail: "KMS Sign response missing signature field".to_string(),
            })?
            .as_ref()
            .to_vec();

        Ok(signature)
    }

    async fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<bool> {
        let algorithm = self.signing_algorithm()?;

        let result = self
            .client
            .verify()
            .key_id(&self.key_id)
            .signing_algorithm(algorithm)
            .message_type(MessageType::Raw)
            .message(Blob::new(payload))
            .signature(Blob::new(signature))
            .send()
            .await
            .map_err(|e| Error::KeyError {
                detail: format!("KMS Verify failed: {e}"),
            })?;

        Ok(result.signature_valid())
    }

    async fn public_jwk(&self) -> Result<serde_json::Value> {
        self.public_key
            .get_or_try_init(|| self.fetch_public_jwk())
            .await
            .cloned()
    }

    fn algorithm(&self) -> &str {
        &self.algorithm
    }

    #[allow(clippy::misnamed_getters)] // field is `kid` (JWT Key ID), method is `key_id` per trait
    fn key_id(&self) -> &str {
        &self.kid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signing_algorithm_mapping() {
        let client = {
            // Create a dummy client for testing the algorithm mapping.
            // We won't actually call any KMS APIs.
            let conf = aws_sdk_kms::Config::builder()
                .behavior_version(aws_sdk_kms::config::BehaviorVersion::latest())
                .region(aws_sdk_kms::config::Region::new("us-east-1"))
                .credentials_provider(aws_sdk_kms::config::Credentials::new(
                    "fake", "fake", None, None, "test",
                ))
                .build();
            aws_sdk_kms::Client::from_conf(conf)
        };

        // Test supported algorithms
        let test_cases = vec![
            ("RS256", SigningAlgorithmSpec::RsassaPkcs1V15Sha256),
            ("RS384", SigningAlgorithmSpec::RsassaPkcs1V15Sha384),
            ("RS512", SigningAlgorithmSpec::RsassaPkcs1V15Sha512),
            ("PS256", SigningAlgorithmSpec::RsassaPssSha256),
            ("PS384", SigningAlgorithmSpec::RsassaPssSha384),
            ("PS512", SigningAlgorithmSpec::RsassaPssSha512),
            ("ES256", SigningAlgorithmSpec::EcdsaSha256),
            ("ES384", SigningAlgorithmSpec::EcdsaSha384),
            ("ES512", SigningAlgorithmSpec::EcdsaSha512),
        ];

        for (alg_str, expected) in test_cases {
            let mgr = KmsKeyManager::new(
                client.clone(),
                "key-id".to_string(),
                alg_str.to_string(),
                "kid-1".to_string(),
            );
            let result = mgr.signing_algorithm().expect("should map algorithm");
            assert_eq!(result, expected, "algorithm mapping for {alg_str}");
        }

        // Test unsupported algorithm
        let mgr = KmsKeyManager::new(
            client.clone(),
            "key-id".to_string(),
            "EdDSA".to_string(),
            "kid-1".to_string(),
        );
        let result = mgr.signing_algorithm();
        assert!(result.is_err(), "EdDSA should not be supported for KMS");
    }

    #[test]
    fn test_key_id_and_algorithm() {
        let conf = aws_sdk_kms::Config::builder()
            .behavior_version(aws_sdk_kms::config::BehaviorVersion::latest())
            .region(aws_sdk_kms::config::Region::new("us-east-1"))
            .credentials_provider(aws_sdk_kms::config::Credentials::new(
                "fake", "fake", None, None, "test",
            ))
            .build();
        let client = aws_sdk_kms::Client::from_conf(conf);

        let mgr = KmsKeyManager::new(
            client,
            "arn:aws:kms:us-east-1:123456789012:key/test-key".to_string(),
            "ES256".to_string(),
            "my-kid-42".to_string(),
        );

        assert_eq!(mgr.algorithm(), "ES256");
        assert_eq!(mgr.key_id(), "my-kid-42");
    }

    #[test]
    fn test_parse_ec_public_key_to_jwk() {
        use p256::ecdsa::SigningKey;
        use p256::elliptic_curve::Generate;
        use p256::pkcs8::EncodePublicKey;

        let signing_key = SigningKey::generate();
        let public_key = signing_key.verifying_key();
        let spki_der = p256::PublicKey::from(public_key)
            .to_public_key_der()
            .expect("DER encoding should work");

        let jwk =
            parse_spki_to_jwk(spki_der.as_ref(), "ES256", "test-kid").expect("should parse EC key");

        assert_eq!(jwk["kty"], "EC");
        assert_eq!(jwk["crv"], "P-256");
        assert_eq!(jwk["alg"], "ES256");
        assert_eq!(jwk["kid"], "test-kid");
        assert!(jwk["x"].as_str().is_some(), "should have x coordinate");
        assert!(jwk["y"].as_str().is_some(), "should have y coordinate");
        let x_len = jwk["x"].as_str().unwrap().len();
        let y_len = jwk["y"].as_str().unwrap().len();
        assert!(
            x_len >= 42 && x_len <= 44,
            "x should be ~43 base64url chars, got {x_len}"
        );
        assert!(
            y_len >= 42 && y_len <= 44,
            "y should be ~43 base64url chars, got {y_len}"
        );
    }

    #[test]
    fn test_parse_rsa_public_key_to_jwk() {
        use rsa::pkcs8::EncodePublicKey;
        use rsa::RsaPrivateKey;

        let private_key = RsaPrivateKey::new(&mut rand::rng(), 2048).unwrap();
        let public_key = private_key.to_public_key();
        let spki_der = public_key
            .to_public_key_der()
            .expect("DER encoding should work");

        let jwk = parse_spki_to_jwk(spki_der.as_ref(), "RS256", "test-kid")
            .expect("should parse RSA key");

        assert_eq!(jwk["kty"], "RSA");
        assert_eq!(jwk["alg"], "RS256");
        assert_eq!(jwk["kid"], "test-kid");
        assert!(jwk["n"].as_str().is_some(), "should have modulus");
        assert!(jwk["e"].as_str().is_some(), "should have exponent");
    }

    #[test]
    fn test_parse_spki_unsupported_algorithm() {
        let result = parse_spki_to_jwk(&[0u8; 32], "EdDSA", "kid");
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore] // Requires LocalStack or real KMS
    async fn test_kms_sign_integration() {
        // This would need LocalStack with a pre-created KMS key.
        // Placeholder for integration testing.
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .endpoint_url("http://localhost:4566")
            .region(aws_config::Region::new("us-east-1"))
            .load()
            .await;

        let client = aws_sdk_kms::Client::new(&config);
        let mgr = KmsKeyManager::new(
            client,
            "alias/test-signing-key".to_string(),
            "ES256".to_string(),
            "test-kid".to_string(),
        );

        let payload = b"test payload for signing";
        let signature = mgr.sign(payload).await.expect("sign should succeed");
        assert!(!signature.is_empty(), "signature should not be empty");

        let jwk = mgr.public_jwk().await.expect("public_jwk should succeed");
        assert_eq!(jwk["alg"], "ES256");
        assert_eq!(jwk["kid"], "test-kid");
    }
}
