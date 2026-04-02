use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::SigningKey;
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::KeyManager;

#[derive(Debug)]
pub struct LocalKeyManager {
    signing_key: SigningKey,
    algorithm: String,
    kid: String,
}

impl LocalKeyManager {
    /// Create a `LocalKeyManager` from PEM-encoded PKCS#8 Ed25519 private key bytes.
    pub fn from_pem(pem_contents: &[u8], algorithm: &str, kid: &str) -> Result<Self> {
        let pem_str = std::str::from_utf8(pem_contents).map_err(|e| Error::KeyError {
            detail: format!("invalid UTF-8 in PEM data: {e}"),
        })?;

        let signing_key = SigningKey::from_pkcs8_pem(pem_str).map_err(|e| Error::KeyError {
            detail: format!("failed to parse Ed25519 PKCS#8 PEM: {e}"),
        })?;

        Ok(Self {
            signing_key,
            algorithm: algorithm.to_owned(),
            kid: kid.to_owned(),
        })
    }

    /// Create a `LocalKeyManager` by reading a PEM file from disk.
    pub fn from_file(path: &str, algorithm: &str, kid: &str) -> Result<Self> {
        let contents = std::fs::read(path).map_err(|e| Error::KeyError {
            detail: format!("failed to read key file {path}: {e}"),
        })?;
        Self::from_pem(&contents, algorithm, kid)
    }
}

#[async_trait]
impl KeyManager for LocalKeyManager {
    async fn sign(&self, payload: &[u8]) -> Result<Vec<u8>> {
        use ed25519_dalek::Signer;
        let signature = self.signing_key.sign(payload);
        Ok(signature.to_bytes().to_vec())
    }

    async fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<bool> {
        use ed25519_dalek::{Signature, Verifier};
        let sig_bytes: [u8; 64] = signature.try_into().map_err(|_| Error::KeyError {
            detail: format!("invalid Ed25519 signature length: expected 64, got {}", signature.len()),
        })?;
        let sig = Signature::from_bytes(&sig_bytes);
        Ok(self.signing_key.verifying_key().verify(payload, &sig).is_ok())
    }

    async fn public_jwk(&self) -> Result<serde_json::Value> {
        let verifying_key = self.signing_key.verifying_key();
        let pub_bytes = verifying_key.to_bytes();
        let x = URL_SAFE_NO_PAD.encode(pub_bytes);

        Ok(serde_json::json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "alg": self.algorithm,
            "use": "sig",
            "kid": self.kid,
            "x": x,
        }))
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
    use ed25519_dalek::pkcs8::EncodePrivateKey;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use std::io::Write;

    #[tokio::test]
    async fn test_local_key_manager_sign_and_verify() {
        // 1. Generate an Ed25519 keypair
        let signing_key = SigningKey::generate(&mut rand::rng());

        // 2. Write the private key as PEM to a temp file
        let pem_doc = signing_key
            .to_pkcs8_pem(ed25519_dalek::pkcs8::spki::der::pem::LineEnding::LF)
            .expect("failed to encode signing key as PKCS#8 PEM");

        let mut tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");
        tmp.write_all(pem_doc.as_bytes())
            .expect("failed to write PEM to temp file");
        tmp.flush().expect("failed to flush temp file");

        // 3. Create a LocalKeyManager from the PEM file
        let manager = LocalKeyManager::from_file(
            tmp.path().to_str().unwrap(),
            "EdDSA",
            "my-test-key-42",
        )
        .expect("failed to create LocalKeyManager from PEM file");

        // 4. Sign a payload
        let payload = b"hello, this is a test payload";
        let sig_bytes = manager
            .sign(payload)
            .await
            .expect("signing should succeed");
        assert_eq!(sig_bytes.len(), 64, "Ed25519 signature should be 64 bytes");

        // 5. Verify the signature using the public key from public_jwk()
        let jwk = manager
            .public_jwk()
            .await
            .expect("public_jwk should succeed");

        let x_b64 = jwk["x"].as_str().expect("JWK should have 'x' field");
        let pub_key_bytes = URL_SAFE_NO_PAD
            .decode(x_b64)
            .expect("base64url decode should succeed");
        assert_eq!(pub_key_bytes.len(), 32, "Ed25519 public key should be 32 bytes");

        let pub_key_array: [u8; 32] = pub_key_bytes.try_into().unwrap();
        let verifying_key = VerifyingKey::from_bytes(&pub_key_array)
            .expect("should construct VerifyingKey from bytes");

        let sig_array: [u8; 64] = sig_bytes.try_into().unwrap();
        let signature = Signature::from_bytes(&sig_array);

        verifying_key
            .verify(payload, &signature)
            .expect("signature verification should succeed");

        // 6. Assert algorithm() and key_id()
        assert_eq!(manager.algorithm(), "EdDSA");
        assert_eq!(manager.key_id(), "my-test-key-42");

        // Also verify JWK structure
        assert_eq!(jwk["kty"], "OKP");
        assert_eq!(jwk["crv"], "Ed25519");
        assert_eq!(jwk["alg"], "EdDSA");
        assert_eq!(jwk["use"], "sig");
        assert_eq!(jwk["kid"], "my-test-key-42");
    }

    #[test]
    fn test_from_pem_invalid_data() {
        let result = LocalKeyManager::from_pem(b"not valid pem data", "EdDSA", "kid");
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            Error::KeyError { detail } => {
                assert!(detail.contains("failed to parse"), "unexpected detail: {detail}");
            }
            other => panic!("expected KeyError, got: {other:?}"),
        }
    }

    #[test]
    fn test_from_pem_invalid_utf8() {
        let result = LocalKeyManager::from_pem(&[0xFF, 0xFE], "EdDSA", "kid");
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            Error::KeyError { detail } => {
                assert!(detail.contains("invalid UTF-8"), "unexpected detail: {detail}");
            }
            other => panic!("expected KeyError, got: {other:?}"),
        }
    }

    #[test]
    fn test_from_file_nonexistent() {
        let result = LocalKeyManager::from_file("/nonexistent/path.pem", "EdDSA", "kid");
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            Error::KeyError { detail } => {
                assert!(detail.contains("failed to read"), "unexpected detail: {detail}");
            }
            other => panic!("expected KeyError, got: {other:?}"),
        }
    }
}
