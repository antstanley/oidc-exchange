use async_trait::async_trait;
use aws_sdk_kms::primitives::Blob;
use aws_sdk_kms::types::{MessageType, SigningAlgorithmSpec};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::KeyManager;
use rsa::pkcs8::{AssociatedOid, DecodePublicKey};
use rsa::signature::digest::Digest;
use rsa::signature::Verifier as _;
use rsa::traits::PublicKeyParts;

/// AWS KMS-backed key manager that uses the KMS Sign API for JWT signing.
pub struct KmsKeyManager {
    client: aws_sdk_kms::Client,
    key_id: String,
    algorithm: String,
    kid: String,
    /// Cached public key material, fetched once from KMS `GetPublicKey`: the
    /// raw SPKI DER bytes (consumed by local `verify`, never re-fetched) and
    /// the JWK JSON derived from them (served at `/keys`).
    public_key: tokio::sync::OnceCell<(Vec<u8>, serde_json::Value)>,
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

    /// Fetch the public key from KMS once and build both the raw SPKI DER
    /// (consumed by local `verify`) and its RFC 7517 compliant JWK. This is
    /// the single `GetPublicKey` call the adapter makes; `verify` never
    /// triggers a second one.
    async fn fetch_public_key_material(&self) -> Result<(Vec<u8>, serde_json::Value)> {
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
            .as_ref()
            .to_vec();

        let jwk = parse_spki_to_jwk(&public_key_der, &self.algorithm, &self.kid)?;

        Ok((public_key_der, jwk))
    }
}

/// A single zero octet, preserved when encoding a zero-valued integer so the
/// Base64urlUInt encoding is never an empty string.
const ZERO_VALUE_OCTET: [u8; 1] = [0u8];

/// Raw `r || s` byte length of a JWS ES256 signature (RFC 7518 §3.4): two
/// concatenated 32-byte P-256 field elements.
const RAW_SIG_LEN_ES256: usize = 64;

/// Raw `r || s` byte length of a JWS ES384 signature: two concatenated
/// 48-byte P-384 field elements.
const RAW_SIG_LEN_ES384: usize = 96;

/// Raw `r || s` byte length of a JWS ES512 signature: two concatenated
/// 66-byte P-521 field elements.
const RAW_SIG_LEN_ES512: usize = 132;

/// The expected raw `r || s` signature width for a JWS ES* algorithm, or
/// `None` for an algorithm that isn't ECDSA (RS*/PS* signatures have no
/// fixed width — it tracks the RSA key size).
fn ecdsa_raw_signature_len(algorithm: &str) -> Option<usize> {
    match algorithm {
        "ES256" => Some(RAW_SIG_LEN_ES256),
        "ES384" => Some(RAW_SIG_LEN_ES384),
        "ES512" => Some(RAW_SIG_LEN_ES512),
        _ => None,
    }
}

/// Convert a KMS-returned DER-encoded `Ecdsa-Sig-Value` into the raw
/// fixed-width `r || s` form JWS ES* signatures require (RFC 7515 §3 / RFC
/// 7518 §3.4). `algorithm` selects the curve (and therefore the field width)
/// and must be one of `ES256`/`ES384`/`ES512`.
fn der_to_raw_ecdsa(der_signature: &[u8], algorithm: &str) -> Result<Vec<u8>> {
    let raw = match algorithm {
        "ES256" => p256::ecdsa::Signature::from_der(der_signature)
            .map_err(|e| Error::KeyError {
                detail: format!("failed to parse ES256 DER signature from KMS: {e}"),
            })?
            .to_vec(),
        "ES384" => p384::ecdsa::Signature::from_der(der_signature)
            .map_err(|e| Error::KeyError {
                detail: format!("failed to parse ES384 DER signature from KMS: {e}"),
            })?
            .to_vec(),
        "ES512" => p521::ecdsa::Signature::from_der(der_signature)
            .map_err(|e| Error::KeyError {
                detail: format!("failed to parse ES512 DER signature from KMS: {e}"),
            })?
            .to_vec(),
        other => {
            return Err(Error::KeyError {
                detail: format!("der_to_raw_ecdsa called with a non-ECDSA algorithm: {other}"),
            });
        }
    };

    let expected_len = ecdsa_raw_signature_len(algorithm).unwrap_or_else(|| {
        unreachable!("der_to_raw_ecdsa only reaches here for ES256/ES384/ES512")
    });
    assert_eq!(
        raw.len(),
        expected_len,
        "DER->raw ECDSA conversion for {algorithm} must yield the fixed raw r||s width"
    );

    Ok(raw)
}

/// Convert a raw KMS `Sign` response body into JWS wire-form signature
/// bytes. ES256/ES384/ES512 signatures are DER-encoded by KMS and are
/// converted here to the raw fixed-width `r || s` form JWS requires; RS*/PS*
/// signatures are already JWS-ready and pass through byte-identical.
fn signature_to_jws_form(algorithm: &str, kms_signature: Vec<u8>) -> Result<Vec<u8>> {
    match algorithm {
        "ES256" | "ES384" | "ES512" => der_to_raw_ecdsa(&kms_signature, algorithm),
        "RS256" | "RS384" | "RS512" | "PS256" | "PS384" | "PS512" => Ok(kms_signature),
        other => Err(Error::KeyError {
            detail: format!("unsupported KMS signing algorithm for signature encoding: {other}"),
        }),
    }
}

/// Coordinate byte length of a NIST P-256 point (RFC 7518 §6.2.1.2/6.2.1.3),
/// used to size the `x`/`y` split of a SEC1 uncompressed EC point.
const EC_COORD_LEN_P256: usize = 32;

/// Coordinate byte length of a NIST P-384 point (RFC 7518 §6.2.1.2/6.2.1.3).
const EC_COORD_LEN_P384: usize = 48;

/// Coordinate byte length of a NIST P-521 point (RFC 7518 §6.2.1.2/6.2.1.3).
/// P-521 field elements are 521 bits, which pack into 66 bytes (ceil(521/8)).
const EC_COORD_LEN_P521: usize = 66;

/// Encode a big-endian unsigned integer as an RFC 7518 §6.3 Base64urlUInt:
/// base64url (no padding) of the minimal big-endian byte string, with every
/// leading `0x00` octet stripped. A value of zero still encodes as a single
/// zero octet (never an empty string).
fn base64url_uint(be_bytes: &[u8]) -> String {
    let trimmed = match be_bytes.iter().position(|&b| b != 0) {
        Some(idx) => &be_bytes[idx..],
        None => &ZERO_VALUE_OCTET[..],
    };
    URL_SAFE_NO_PAD.encode(trimmed)
}

/// Parse a DER-encoded SubjectPublicKeyInfo into an RFC 7517 JWK JSON value.
///
/// Supports RSA (RS256/384/512, PS256/384/512) and EC (ES256/ES384/ES512,
/// i.e. P-256/P-384/P-521) keys.
fn parse_spki_to_jwk(spki_der: &[u8], algorithm: &str, kid: &str) -> Result<serde_json::Value> {
    match algorithm {
        a if a.starts_with("RS") || a.starts_with("PS") => {
            let public_key =
                rsa::RsaPublicKey::from_public_key_der(spki_der).map_err(|e| Error::KeyError {
                    detail: format!("failed to parse RSA public key DER: {e}"),
                })?;

            let n = base64url_uint(&public_key.n().to_be_bytes());
            let e = base64url_uint(&public_key.e().to_be_bytes());

            Ok(serde_json::json!({
                "kty": "RSA",
                "alg": algorithm,
                "use": "sig",
                "kid": kid,
                "n": n,
                "e": e,
            }))
        }
        "ES256" | "ES384" | "ES512" => {
            // EC keys in SPKI DER contain an uncompressed SEC1 point: 0x04 || x || y
            let (crv, coord_len) = match algorithm {
                "ES256" => ("P-256", EC_COORD_LEN_P256),
                "ES384" => ("P-384", EC_COORD_LEN_P384),
                "ES512" => ("P-521", EC_COORD_LEN_P521),
                _ => unreachable!("outer match already restricted algorithm to ES256/ES384/ES512"),
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

/// Verify an RSASSA-PKCS1-v1.5 (RS*) signature locally against a DER-encoded
/// SPKI public key. An SPKI that fails to parse is a key-material error; a
/// signature that fails to parse or fails cryptographic verification is
/// simply an invalid signature (`Ok(false)`), never an error.
fn verify_rsa_pkcs1v15<D>(spki_der: &[u8], payload: &[u8], signature: &[u8]) -> Result<bool>
where
    D: Digest + AssociatedOid,
{
    let public_key =
        rsa::RsaPublicKey::from_public_key_der(spki_der).map_err(|e| Error::KeyError {
            detail: format!("failed to parse RSA public key DER for local verify: {e}"),
        })?;
    let verifying_key = rsa::pkcs1v15::VerifyingKey::<D>::new(public_key);

    let Ok(parsed_signature) = rsa::pkcs1v15::Signature::try_from(signature) else {
        return Ok(false);
    };

    Ok(verifying_key.verify(payload, &parsed_signature).is_ok())
}

/// Verify an RSASSA-PSS (PS*) signature locally against a DER-encoded SPKI
/// public key. Same error/false split as [`verify_rsa_pkcs1v15`].
fn verify_rsa_pss<D>(spki_der: &[u8], payload: &[u8], signature: &[u8]) -> Result<bool>
where
    D: Digest + rsa::signature::digest::FixedOutputReset,
{
    let public_key =
        rsa::RsaPublicKey::from_public_key_der(spki_der).map_err(|e| Error::KeyError {
            detail: format!("failed to parse RSA public key DER for local verify: {e}"),
        })?;
    let verifying_key = rsa::pss::VerifyingKey::<D>::new(public_key);

    let Ok(parsed_signature) = rsa::pss::Signature::try_from(signature) else {
        return Ok(false);
    };

    Ok(verifying_key.verify(payload, &parsed_signature).is_ok())
}

/// Verify a raw `r || s` ES256 signature locally against a DER-encoded SPKI
/// public key. The signature bytes are consumed directly in JWS raw form via
/// `ecdsa::Signature::from_slice` — no raw→DER conversion is performed
/// anywhere in this adapter. Same error/false split as
/// [`verify_rsa_pkcs1v15`]: an unparseable SPKI is a key-material error, an
/// unparseable or cryptographically invalid signature is `Ok(false)`.
fn verify_ecdsa_p256(spki_der: &[u8], payload: &[u8], signature: &[u8]) -> Result<bool> {
    use p256::pkcs8::DecodePublicKey as _;

    let verifying_key =
        p256::ecdsa::VerifyingKey::from_public_key_der(spki_der).map_err(|e| Error::KeyError {
            detail: format!("failed to parse ES256 public key DER for local verify: {e}"),
        })?;

    let Ok(parsed_signature) = p256::ecdsa::Signature::from_slice(signature) else {
        return Ok(false);
    };

    Ok(verifying_key.verify(payload, &parsed_signature).is_ok())
}

/// Verify a raw `r || s` ES384 signature locally. See [`verify_ecdsa_p256`].
fn verify_ecdsa_p384(spki_der: &[u8], payload: &[u8], signature: &[u8]) -> Result<bool> {
    use p384::pkcs8::DecodePublicKey as _;

    let verifying_key =
        p384::ecdsa::VerifyingKey::from_public_key_der(spki_der).map_err(|e| Error::KeyError {
            detail: format!("failed to parse ES384 public key DER for local verify: {e}"),
        })?;

    let Ok(parsed_signature) = p384::ecdsa::Signature::from_slice(signature) else {
        return Ok(false);
    };

    Ok(verifying_key.verify(payload, &parsed_signature).is_ok())
}

/// Verify a raw `r || s` ES512 signature locally. See [`verify_ecdsa_p256`].
fn verify_ecdsa_p521(spki_der: &[u8], payload: &[u8], signature: &[u8]) -> Result<bool> {
    use p521::pkcs8::DecodePublicKey as _;

    let verifying_key =
        p521::ecdsa::VerifyingKey::from_public_key_der(spki_der).map_err(|e| Error::KeyError {
            detail: format!("failed to parse ES512 public key DER for local verify: {e}"),
        })?;

    let Ok(parsed_signature) = p521::ecdsa::Signature::from_slice(signature) else {
        return Ok(false);
    };

    Ok(verifying_key.verify(payload, &parsed_signature).is_ok())
}

/// Verify a signature locally against the cached SPKI, dispatching on the
/// configured algorithm. RS*/PS* use `rsa` `pkcs1v15`/`pss` with the matching
/// `sha2` digest; ES256/384/512 use the curve's `ecdsa::VerifyingKey`
/// consuming the raw `r || s` wire form directly. An unsupported algorithm is
/// a key-material error, not a silent `false`, so the match stays exhaustive
/// over every algorithm string this adapter is configured with.
fn verify_locally(
    spki_der: &[u8],
    algorithm: &str,
    payload: &[u8],
    signature: &[u8],
) -> Result<bool> {
    match algorithm {
        "RS256" => verify_rsa_pkcs1v15::<rsa::sha2::Sha256>(spki_der, payload, signature),
        "RS384" => verify_rsa_pkcs1v15::<rsa::sha2::Sha384>(spki_der, payload, signature),
        "RS512" => verify_rsa_pkcs1v15::<rsa::sha2::Sha512>(spki_der, payload, signature),
        "PS256" => verify_rsa_pss::<rsa::sha2::Sha256>(spki_der, payload, signature),
        "PS384" => verify_rsa_pss::<rsa::sha2::Sha384>(spki_der, payload, signature),
        "PS512" => verify_rsa_pss::<rsa::sha2::Sha512>(spki_der, payload, signature),
        "ES256" => verify_ecdsa_p256(spki_der, payload, signature),
        "ES384" => verify_ecdsa_p384(spki_der, payload, signature),
        "ES512" => verify_ecdsa_p521(spki_der, payload, signature),
        other => Err(Error::KeyError {
            detail: format!("unsupported algorithm for local verify: {other}"),
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
        assert!(
            !signature.is_empty(),
            "KMS Sign response signature must not be empty"
        );

        let converted = signature_to_jws_form(&self.algorithm, signature)?;

        if let Some(expected_len) = ecdsa_raw_signature_len(&self.algorithm) {
            assert_eq!(
                converted.len(),
                expected_len,
                "sign() must return the fixed raw r||s width for {}",
                self.algorithm
            );
        }

        Ok(converted)
    }

    async fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<bool> {
        // No KMS Verify round-trip: check the signature in-process against
        // the SPKI already fetched (and cached) for the JWK. `sign` already
        // produces the raw `r || s` JWS wire form for ES*, so this consumes
        // it directly with no raw→DER conversion anywhere in the adapter.
        let (spki_der, _jwk) = self
            .public_key
            .get_or_try_init(|| self.fetch_public_key_material())
            .await?;

        verify_locally(spki_der, &self.algorithm, payload, signature)
    }

    async fn public_jwk(&self) -> Result<serde_json::Value> {
        self.public_key
            .get_or_try_init(|| self.fetch_public_key_material())
            .await
            .map(|(_spki_der, jwk)| jwk.clone())
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
            (42..=44).contains(&x_len),
            "x should be ~43 base64url chars, got {x_len}"
        );
        assert!(
            (42..=44).contains(&y_len),
            "y should be ~43 base64url chars, got {y_len}"
        );
    }

    #[test]
    fn test_parse_p521_public_key_to_jwk() {
        use p521::ecdsa::SigningKey;
        use p521::elliptic_curve::Generate;
        use p521::pkcs8::EncodePublicKey;

        let signing_key = SigningKey::generate();
        let public_key = signing_key.verifying_key();
        let spki_der = p521::PublicKey::from(public_key)
            .to_public_key_der()
            .expect("DER encoding should work");

        let jwk =
            parse_spki_to_jwk(spki_der.as_ref(), "ES512", "test-kid").expect("should parse EC key");

        assert_eq!(jwk["kty"], "EC");
        assert_eq!(jwk["crv"], "P-521");
        assert_eq!(jwk["alg"], "ES512");
        assert_eq!(jwk["kid"], "test-kid");
        let x_len = jwk["x"].as_str().expect("should have x coordinate").len();
        let y_len = jwk["y"].as_str().expect("should have y coordinate").len();
        // 66-byte coordinates base64url (no padding) encode to
        // ceil(66 * 4 / 3) = 88 characters, exactly (66 is divisible by 3).
        assert_eq!(
            x_len, 88,
            "x should be 88 base64url chars for a 66-byte P-521 coordinate"
        );
        assert_eq!(
            y_len, 88,
            "y should be 88 base64url chars for a 66-byte P-521 coordinate"
        );
    }

    #[test]
    fn test_parse_ec_public_key_spki_too_short_is_key_error() {
        // One byte short of the minimum P-256 uncompressed point
        // (1 prefix byte + 32 + 32 coordinate bytes = 65 bytes minimum).
        let too_short = vec![0x04u8; 64];
        let result = parse_spki_to_jwk(&too_short, "ES256", "test-kid");
        assert!(
            result.is_err(),
            "SPKI DER too short for the curve must be rejected, not panic"
        );
        assert!(
            matches!(result, Err(Error::KeyError { .. })),
            "must surface as a KeyError, got {result:?}"
        );
    }

    #[test]
    fn test_parse_ec_public_key_missing_uncompressed_prefix_is_key_error() {
        // Right length for a P-256 point (65 bytes), but the point prefix
        // byte is not 0x04 (uncompressed), e.g. 0x02 (compressed, even y).
        let mut bad_prefix = vec![0x02u8; 65];
        bad_prefix[0] = 0x02;
        let result = parse_spki_to_jwk(&bad_prefix, "ES256", "test-kid");
        assert!(
            result.is_err(),
            "a non-0x04 point prefix must be rejected, not panic"
        );
        assert!(
            matches!(result, Err(Error::KeyError { .. })),
            "must surface as a KeyError, got {result:?}"
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
        // RsaPrivateKey::new() always uses the standard public exponent
        // 65537 (0x010001); Base64urlUInt of that value is "AQAB" (no
        // leading zero octet from the 0x01 high byte's own padding, and no
        // extra byte for the value's own 3-byte minimal encoding).
        assert_eq!(
            jwk["e"], "AQAB",
            "e=65537 should encode as Base64urlUInt \"AQAB\" with no leading zero octet"
        );

        let n_bytes = URL_SAFE_NO_PAD
            .decode(jwk["n"].as_str().unwrap())
            .expect("n should be valid base64url");
        assert!(
            n_bytes.first() != Some(&0x00),
            "n must not have a leading zero octet (Base64urlUInt, RFC 7518 §6.3)"
        );
    }

    #[test]
    fn test_base64url_uint_strips_leading_zeros_but_not_the_value() {
        // A value whose minimal big-endian encoding starts with a genuine
        // leading zero octet in the *unstripped* input (e.g. as produced by
        // a fixed-width to_be_bytes() call) must have that zero stripped.
        let with_leading_zero: [u8; 4] = [0x00, 0x01, 0x00, 0x01];
        assert_eq!(base64url_uint(&with_leading_zero), "AQAB");

        // Multiple leading zero octets are all stripped.
        let multiple_leading_zeros: [u8; 3] = [0x00, 0x00, 0x2a];
        assert_eq!(base64url_uint(&multiple_leading_zeros), "Kg");

        // A zero-valued input still yields a non-empty, single-zero-octet
        // encoding rather than an empty string.
        let all_zero: [u8; 3] = [0x00, 0x00, 0x00];
        let encoded = base64url_uint(&all_zero);
        assert!(
            !encoded.is_empty(),
            "zero value must not encode to empty string"
        );
        assert_eq!(encoded, URL_SAFE_NO_PAD.encode([0x00u8]));
    }

    #[test]
    fn test_parse_spki_unsupported_algorithm() {
        let result = parse_spki_to_jwk(&[0u8; 32], "EdDSA", "kid");
        assert!(result.is_err());
    }

    #[test]
    fn test_der_to_raw_ecdsa_es256_round_trips_and_is_fixed_width() {
        use p256::ecdsa::{signature::Signer, Signature, SigningKey};
        use p256::elliptic_curve::Generate;

        let signing_key = SigningKey::generate();
        let signature: Signature = signing_key.sign(b"payload for ES256");
        let (expected_r, expected_s) = signature.split_bytes();
        let der = signature.to_der();

        let raw = der_to_raw_ecdsa(der.as_bytes(), "ES256").expect("DER->raw should succeed");

        assert_eq!(
            raw.len(),
            RAW_SIG_LEN_ES256,
            "ES256 raw signature must be exactly 64 bytes"
        );
        assert_eq!(&raw[..32], expected_r.as_slice(), "r must round-trip");
        assert_eq!(&raw[32..], expected_s.as_slice(), "s must round-trip");
    }

    #[test]
    fn test_der_to_raw_ecdsa_es384_round_trips_and_is_fixed_width() {
        use p384::ecdsa::{signature::Signer, Signature, SigningKey};
        use p384::elliptic_curve::Generate;

        let signing_key = SigningKey::generate();
        let signature: Signature = signing_key.sign(b"payload for ES384");
        let (expected_r, expected_s) = signature.split_bytes();
        let der = signature.to_der();

        let raw = der_to_raw_ecdsa(der.as_bytes(), "ES384").expect("DER->raw should succeed");

        assert_eq!(
            raw.len(),
            RAW_SIG_LEN_ES384,
            "ES384 raw signature must be exactly 96 bytes"
        );
        assert_eq!(&raw[..48], expected_r.as_slice(), "r must round-trip");
        assert_eq!(&raw[48..], expected_s.as_slice(), "s must round-trip");
    }

    #[test]
    fn test_der_to_raw_ecdsa_es512_round_trips_and_is_fixed_width() {
        use p521::ecdsa::{signature::Signer, Signature, SigningKey};
        use p521::elliptic_curve::Generate;

        let signing_key = SigningKey::generate();
        let signature: Signature = signing_key.sign(b"payload for ES512");
        let (expected_r, expected_s) = signature.split_bytes();
        let der = signature.to_der();

        let raw = der_to_raw_ecdsa(der.as_bytes(), "ES512").expect("DER->raw should succeed");

        assert_eq!(
            raw.len(),
            RAW_SIG_LEN_ES512,
            "ES512 raw signature must be exactly 132 bytes"
        );
        assert_eq!(&raw[..66], expected_r.as_slice(), "r must round-trip");
        assert_eq!(&raw[66..], expected_s.as_slice(), "s must round-trip");
    }

    #[test]
    fn test_der_to_raw_ecdsa_malformed_der_is_key_error() {
        // Not a valid ASN.1 SEQUENCE at all.
        let malformed = vec![0xFFu8; 8];
        let result = der_to_raw_ecdsa(&malformed, "ES256");
        assert!(
            result.is_err(),
            "malformed DER must be rejected, not panic or produce garbage bytes"
        );
        assert!(
            matches!(result, Err(Error::KeyError { .. })),
            "must surface as a KeyError, got {result:?}"
        );
    }

    #[test]
    fn test_der_to_raw_ecdsa_truncated_der_is_key_error() {
        // A syntactically-plausible but truncated DER SEQUENCE header with no
        // integer contents.
        let truncated = vec![0x30u8, 0x06, 0x02, 0x01, 0x01];
        let result = der_to_raw_ecdsa(&truncated, "ES384");
        assert!(result.is_err(), "truncated DER must be rejected, not panic");
        assert!(matches!(result, Err(Error::KeyError { .. })));
    }

    #[test]
    fn test_signature_to_jws_form_passes_rsa_and_pss_through_unchanged() {
        let fake_pkcs1v15_sig = vec![0xAB; 256]; // stand-in for a 2048-bit RSA signature
        for alg in ["RS256", "RS384", "RS512", "PS256", "PS384", "PS512"] {
            let result = signature_to_jws_form(alg, fake_pkcs1v15_sig.clone())
                .unwrap_or_else(|e| panic!("{alg} must pass through, got error {e:?}"));
            assert_eq!(
                result, fake_pkcs1v15_sig,
                "{alg} signature bytes must be byte-identical to the KMS response"
            );
        }
    }

    #[test]
    fn test_signature_to_jws_form_converts_es_algorithms() {
        use p256::ecdsa::{signature::Signer, Signature, SigningKey};
        use p256::elliptic_curve::Generate;

        let signing_key = SigningKey::generate();
        let signature: Signature = signing_key.sign(b"payload");
        let der = signature.to_der().as_bytes().to_vec();

        let result = signature_to_jws_form("ES256", der).expect("ES256 conversion should succeed");
        assert_eq!(result.len(), RAW_SIG_LEN_ES256);
    }

    // --- Local verify: accept a validly-signed payload, reject tampering,
    // with no KMS client involved at all (`verify_locally` is a pure
    // function over a locally-generated key and SPKI). ---

    /// XOR-ing the final byte of a signature is enough to invalidate any of
    /// the signature schemes exercised below (fixed-width raw ECDSA,
    /// PKCS#1 v1.5, or PSS) without needing scheme-specific knowledge of the
    /// encoding.
    fn tamper_last_byte(bytes: &[u8]) -> Vec<u8> {
        let mut tampered = bytes.to_vec();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xFF;
        tampered
    }

    macro_rules! rsa_pkcs1v15_verify_test {
        ($test_name:ident, $alg:literal, $digest:ty) => {
            #[test]
            fn $test_name() {
                use rsa::pkcs1v15::SigningKey;
                use rsa::pkcs8::EncodePublicKey;
                use rsa::signature::{SignatureEncoding, Signer};

                let private_key = rsa::RsaPrivateKey::new(&mut rand::rng(), 2048)
                    .expect("RSA key generation should succeed");
                let spki_der = private_key
                    .to_public_key()
                    .to_public_key_der()
                    .expect("DER encoding should work");

                let signing_key = SigningKey::<$digest>::new(private_key);
                let payload = concat!("payload for ", $alg, " local verify").as_bytes();
                let signature: rsa::pkcs1v15::Signature = signing_key.sign(payload);
                let sig_bytes = signature.to_vec();

                assert!(
                    verify_locally(spki_der.as_ref(), $alg, payload, &sig_bytes)
                        .expect("verify must not error for well-formed input"),
                    concat!("a validly signed ", $alg, " payload must verify")
                );
                assert!(
                    !verify_locally(
                        spki_der.as_ref(),
                        $alg,
                        payload,
                        &tamper_last_byte(&sig_bytes)
                    )
                    .expect("verify must not error for a tampered signature"),
                    concat!("a tampered ", $alg, " signature must not verify")
                );
                assert!(
                    !verify_locally(
                        spki_der.as_ref(),
                        $alg,
                        b"a different payload entirely",
                        &sig_bytes
                    )
                    .expect("verify must not error for a mismatched payload"),
                    concat!(
                        "a ",
                        $alg,
                        " signature must not verify against a different payload"
                    )
                );
            }
        };
    }

    macro_rules! rsa_pss_verify_test {
        ($test_name:ident, $alg:literal, $digest:ty) => {
            #[test]
            fn $test_name() {
                use rsa::pkcs8::EncodePublicKey;
                use rsa::pss::SigningKey;
                use rsa::signature::{RandomizedSigner, SignatureEncoding};

                let private_key = rsa::RsaPrivateKey::new(&mut rand::rng(), 2048)
                    .expect("RSA key generation should succeed");
                let spki_der = private_key
                    .to_public_key()
                    .to_public_key_der()
                    .expect("DER encoding should work");

                let signing_key = SigningKey::<$digest>::new(private_key);
                let payload = concat!("payload for ", $alg, " local verify").as_bytes();
                let signature: rsa::pss::Signature =
                    signing_key.sign_with_rng(&mut rand::rng(), payload);
                let sig_bytes = signature.to_vec();

                assert!(
                    verify_locally(spki_der.as_ref(), $alg, payload, &sig_bytes)
                        .expect("verify must not error for well-formed input"),
                    concat!("a validly signed ", $alg, " payload must verify")
                );
                assert!(
                    !verify_locally(
                        spki_der.as_ref(),
                        $alg,
                        payload,
                        &tamper_last_byte(&sig_bytes)
                    )
                    .expect("verify must not error for a tampered signature"),
                    concat!("a tampered ", $alg, " signature must not verify")
                );
                assert!(
                    !verify_locally(
                        spki_der.as_ref(),
                        $alg,
                        b"a different payload entirely",
                        &sig_bytes
                    )
                    .expect("verify must not error for a mismatched payload"),
                    concat!(
                        "a ",
                        $alg,
                        " signature must not verify against a different payload"
                    )
                );
            }
        };
    }

    macro_rules! ecdsa_verify_test {
        ($test_name:ident, $alg:literal, $curve_crate:ident) => {
            #[test]
            fn $test_name() {
                use $curve_crate::ecdsa::{signature::Signer, Signature, SigningKey};
                use $curve_crate::elliptic_curve::Generate;
                use $curve_crate::pkcs8::EncodePublicKey;

                let signing_key = SigningKey::generate();
                let public_key = signing_key.verifying_key();
                let spki_der = $curve_crate::PublicKey::from(public_key)
                    .to_public_key_der()
                    .expect("DER encoding should work");

                let payload = concat!("payload for ", $alg, " local verify").as_bytes();
                let signature: Signature = signing_key.sign(payload);
                let sig_bytes = signature.to_vec();

                assert!(
                    verify_locally(spki_der.as_ref(), $alg, payload, &sig_bytes)
                        .expect("verify must not error for well-formed input"),
                    concat!("a validly signed ", $alg, " payload must verify")
                );
                assert!(
                    !verify_locally(
                        spki_der.as_ref(),
                        $alg,
                        payload,
                        &tamper_last_byte(&sig_bytes)
                    )
                    .expect("verify must not error for a tampered signature"),
                    concat!("a tampered ", $alg, " signature must not verify")
                );
                assert!(
                    !verify_locally(
                        spki_der.as_ref(),
                        $alg,
                        b"a different payload entirely",
                        &sig_bytes
                    )
                    .expect("verify must not error for a mismatched payload"),
                    concat!(
                        "a ",
                        $alg,
                        " signature must not verify against a different payload"
                    )
                );
            }
        };
    }

    rsa_pkcs1v15_verify_test!(
        test_verify_locally_rs256_accepts_valid_and_rejects_tampering,
        "RS256",
        rsa::sha2::Sha256
    );
    rsa_pkcs1v15_verify_test!(
        test_verify_locally_rs384_accepts_valid_and_rejects_tampering,
        "RS384",
        rsa::sha2::Sha384
    );
    rsa_pkcs1v15_verify_test!(
        test_verify_locally_rs512_accepts_valid_and_rejects_tampering,
        "RS512",
        rsa::sha2::Sha512
    );
    rsa_pss_verify_test!(
        test_verify_locally_ps256_accepts_valid_and_rejects_tampering,
        "PS256",
        rsa::sha2::Sha256
    );
    rsa_pss_verify_test!(
        test_verify_locally_ps384_accepts_valid_and_rejects_tampering,
        "PS384",
        rsa::sha2::Sha384
    );
    rsa_pss_verify_test!(
        test_verify_locally_ps512_accepts_valid_and_rejects_tampering,
        "PS512",
        rsa::sha2::Sha512
    );
    ecdsa_verify_test!(
        test_verify_locally_es256_accepts_valid_and_rejects_tampering,
        "ES256",
        p256
    );
    ecdsa_verify_test!(
        test_verify_locally_es384_accepts_valid_and_rejects_tampering,
        "ES384",
        p384
    );
    ecdsa_verify_test!(
        test_verify_locally_es512_accepts_valid_and_rejects_tampering,
        "ES512",
        p521
    );

    #[test]
    fn test_verify_locally_unsupported_algorithm_is_key_error() {
        let result = verify_locally(&[0u8; 32], "EdDSA", b"payload", b"signature");
        assert!(
            result.is_err(),
            "an unsupported algorithm must be rejected, not silently treated as false"
        );
        assert!(
            matches!(result, Err(Error::KeyError { .. })),
            "must surface as a KeyError, got {result:?}"
        );
    }

    #[test]
    fn test_verify_locally_unparseable_spki_is_key_error() {
        let garbage_spki = vec![0xFFu8; 8];

        let rsa_result = verify_locally(&garbage_spki, "RS256", b"payload", b"signature");
        assert!(
            rsa_result.is_err(),
            "unparseable RSA SPKI key material must be a KeyError, not Ok(false)"
        );
        assert!(matches!(rsa_result, Err(Error::KeyError { .. })));

        let ec_result = verify_locally(&garbage_spki, "ES256", b"payload", b"signature");
        assert!(
            ec_result.is_err(),
            "unparseable EC SPKI key material must be a KeyError, not Ok(false)"
        );
        assert!(matches!(ec_result, Err(Error::KeyError { .. })));
    }

    #[test]
    fn test_verify_locally_malformed_signature_bytes_returns_false_not_error() {
        use p256::ecdsa::SigningKey;
        use p256::elliptic_curve::Generate;
        use p256::pkcs8::EncodePublicKey;

        let signing_key = SigningKey::generate();
        let public_key = signing_key.verifying_key();
        let spki_der = p256::PublicKey::from(public_key)
            .to_public_key_der()
            .expect("DER encoding should work");

        // Ten bytes is not the fixed 64-byte ES256 raw r||s width: this must
        // be treated as an invalid signature, not a parse error or a panic.
        let too_short_signature = vec![0u8; 10];
        let result = verify_locally(spki_der.as_ref(), "ES256", b"payload", &too_short_signature)
            .expect("a malformed signature must not surface as an error");
        assert!(!result, "a wrong-length signature must not verify");
    }

    #[tokio::test]
    async fn test_public_key_cache_fetches_shared_material_only_once() {
        let cell: tokio::sync::OnceCell<(Vec<u8>, serde_json::Value)> =
            tokio::sync::OnceCell::new();
        let fetch_count = std::sync::atomic::AtomicUsize::new(0);

        let fetch = || async {
            fetch_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok::<(Vec<u8>, serde_json::Value), Error>((
                vec![1, 2, 3, 4],
                serde_json::json!({ "kty": "EC" }),
            ))
        };

        // Mirrors how `verify` and `public_jwk` both read through the same
        // `public_key` cell in `KmsKeyManager`: the second caller must not
        // trigger a second KMS `GetPublicKey` call.
        let first = cell
            .get_or_try_init(fetch)
            .await
            .expect("first fetch should succeed");
        let first_spki = first.0.clone();
        let second = cell
            .get_or_try_init(fetch)
            .await
            .expect("second fetch should succeed");

        assert_eq!(
            fetch_count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the fetch closure must run exactly once no matter how many callers read the cell"
        );
        assert_eq!(
            second.0, first_spki,
            "verify and public_jwk must observe the exact same cached SPKI bytes"
        );
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
