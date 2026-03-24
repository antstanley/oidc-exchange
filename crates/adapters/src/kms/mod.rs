use async_trait::async_trait;
use aws_sdk_kms::primitives::Blob;
use aws_sdk_kms::types::{MessageType, SigningAlgorithmSpec};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::KeyManager;

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

    /// Fetch the public key from KMS and build a JWK-like structure.
    ///
    /// For v1, this returns the raw DER-encoded public key in a JWK-compatible
    /// structure. The exact JWK conversion (parsing ASN.1 to extract x/y
    /// coordinates for EC keys, or n/e for RSA) can be refined later.
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

        let key_type = match self.algorithm.as_str() {
            a if a.starts_with("RS") || a.starts_with("PS") => "RSA",
            a if a.starts_with("ES") => "EC",
            _ => "unknown",
        };

        // v1: Return the DER bytes base64url-encoded in a JWK-like structure.
        // A production implementation would parse the SubjectPublicKeyInfo ASN.1
        // to extract the actual key components (n/e for RSA, x/y/crv for EC).
        Ok(serde_json::json!({
            "kty": key_type,
            "alg": self.algorithm,
            "use": "sig",
            "kid": self.kid,
            "x5c": [URL_SAFE_NO_PAD.encode(public_key_der)],
        }))
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

    async fn public_jwk(&self) -> Result<serde_json::Value> {
        self.public_key
            .get_or_try_init(|| self.fetch_public_jwk())
            .await
            .cloned()
    }

    fn algorithm(&self) -> &str {
        &self.algorithm
    }

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
