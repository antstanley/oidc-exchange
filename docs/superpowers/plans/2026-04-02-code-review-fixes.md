# Code Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement 4 fixes from the semi-formal code review: audit fallback logging, KMS JWK generation, revoke endpoint validation, and session cleanup.

**Architecture:** Each fix is independent. Task 1 (audit) and Task 4 (session cleanup) are purely additive. Task 2 (KMS JWK) rewrites one method. Task 3 (revoke validation) adds a `verify` method to the `KeyManager` trait and uses it in the revoke path.

**Tech Stack:** Rust, ed25519-dalek, rsa, p256/p384, AWS KMS SDK, tokio, sqlx, heed (LMDB), fred (Valkey)

---

### Task 1: Switch audit fallback logging from println/eprintln to tracing

**Files:**
- Modify: `crates/core/src/service/mod.rs:107-114`

- [ ] **Step 1: Write failing test for audit fallback behavior**

No new test needed here --- the existing audit tests in `crates/core/tests/audit.rs` cover the fallback path, and this change only swaps the output sink. The behavior (blocking vs non-blocking based on severity) is preserved.

- [ ] **Step 2: Replace println/eprintln with tracing in emit_audit**

In `crates/core/src/service/mod.rs`, replace lines 107-114:

```rust
            // Always emit to stdout/stderr as fallback
            let serialized = serde_json::to_string(&event)
                .unwrap_or_else(|_| format!("{:?}", event));

            if event.severity as u8 <= AuditSeverity::Error as u8 {
                eprintln!("{serialized}");
            } else {
                println!("{serialized}");
            }
```

With:

```rust
            // Always emit via tracing as fallback (captured by Lambda, CloudWatch, etc.)
            let serialized = serde_json::to_string(&event)
                .unwrap_or_else(|_| format!("{:?}", event));

            if event.severity as u8 <= AuditSeverity::Error as u8 {
                tracing::error!(audit_fallback = true, "{serialized}");
            } else {
                tracing::info!(audit_fallback = true, "{serialized}");
            }
```

- [ ] **Step 3: Verify build**

Run: `cargo build -p oidc-exchange-core`
Expected: Compiles without errors. `tracing` is already a dependency.

- [ ] **Step 4: Run existing audit tests**

Run: `cargo nextest run -p oidc-exchange-core --test audit`
Expected: All pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/service/mod.rs
git commit -m "fix: use tracing instead of println/eprintln for audit fallback logging"
```

---

### Task 2: Fix KMS JWK generation to produce RFC 7517 compliant JWK

**Files:**
- Modify: `crates/adapters/Cargo.toml` (add `rsa` as regular dep, add `p256`)
- Modify: `crates/adapters/src/kms/mod.rs:58-92` (rewrite `fetch_public_jwk`)
- Test: `crates/adapters/src/kms/mod.rs` (existing + new tests)

- [ ] **Step 1: Add dependencies**

In `crates/adapters/Cargo.toml`, add `rsa` to `[dependencies]` (it's currently only a dev-dep) and add `p256`:

```toml
rsa = { version = "0.10.0-rc.1", features = ["pkcs8"] }
p256 = { version = "0.14", features = ["pkcs8"] }
p384 = { version = "0.14", features = ["pkcs8"] }
```

Keep the existing `rsa` in `[dev-dependencies]` as-is (or remove the duplicate since it's now a regular dep).

- [ ] **Step 2: Write tests for proper JWK output**

Add these tests to the `#[cfg(test)] mod tests` block in `crates/adapters/src/kms/mod.rs`:

```rust
    #[test]
    fn test_parse_ec_public_key_to_jwk() {
        // Generate a P-256 key pair, export as DER, and verify our parsing
        use p256::ecdsa::SigningKey;
        use p256::pkcs8::EncodePublicKey;
        use p256::elliptic_curve::Generate;

        let signing_key = SigningKey::generate();
        let public_key = signing_key.verifying_key();
        let spki_der = p256::PublicKey::from(public_key)
            .to_public_key_der()
            .expect("DER encoding should work");

        let jwk = parse_spki_to_jwk(spki_der.as_ref(), "ES256", "test-kid")
            .expect("should parse EC key");

        assert_eq!(jwk["kty"], "EC");
        assert_eq!(jwk["crv"], "P-256");
        assert_eq!(jwk["alg"], "ES256");
        assert_eq!(jwk["kid"], "test-kid");
        assert!(jwk["x"].as_str().is_some(), "should have x coordinate");
        assert!(jwk["y"].as_str().is_some(), "should have y coordinate");
        // x and y should be 32 bytes -> 43 chars base64url
        let x_len = jwk["x"].as_str().unwrap().len();
        let y_len = jwk["y"].as_str().unwrap().len();
        assert!(x_len >= 42 && x_len <= 44, "x should be ~43 base64url chars, got {x_len}");
        assert!(y_len >= 42 && y_len <= 44, "y should be ~43 base64url chars, got {y_len}");
    }

    #[test]
    fn test_parse_rsa_public_key_to_jwk() {
        use rsa::RsaPrivateKey;
        use rsa::pkcs8::EncodePublicKey;

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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo nextest run -p oidc-exchange-adapters kms`
Expected: FAIL --- `parse_spki_to_jwk` does not exist yet.

- [ ] **Step 4: Implement `parse_spki_to_jwk` and update `fetch_public_jwk`**

In `crates/adapters/src/kms/mod.rs`, add a helper function and rewrite `fetch_public_jwk`:

```rust
use rsa::pkcs8::DecodePublicKey;
use rsa::traits::PublicKeyParts;

/// Parse a DER-encoded SubjectPublicKeyInfo into a JWK JSON value.
///
/// Supports RSA (RS256/384/512, PS256/384/512) and EC (ES256, ES384) keys.
fn parse_spki_to_jwk(
    spki_der: &[u8],
    algorithm: &str,
    kid: &str,
) -> Result<serde_json::Value> {
    match algorithm {
        a if a.starts_with("RS") || a.starts_with("PS") => {
            let public_key = rsa::RsaPublicKey::from_public_key_der(spki_der)
                .map_err(|e| Error::KeyError {
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
            // EC keys: parse the SubjectPublicKeyInfo to get the uncompressed point.
            // SPKI for EC contains: AlgorithmIdentifier + BIT STRING (SEC1 point).
            // SEC1 uncompressed point: 0x04 || x || y
            let (crv, coord_len) = match algorithm {
                "ES256" => ("P-256", 32),
                "ES384" => ("P-384", 48),
                _ => unreachable!(),
            };

            // The SEC1 point is at the end of the SPKI DER.
            // For P-256: SPKI is 91 bytes, point is last 65 bytes (04 + 32 + 32)
            // For P-384: SPKI is 120 bytes, point is last 97 bytes (04 + 48 + 48)
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
```

Then update `fetch_public_jwk` to call it:

```rust
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
```

- [ ] **Step 5: Run tests**

Run: `cargo nextest run -p oidc-exchange-adapters kms`
Expected: All pass including the new JWK parsing tests.

- [ ] **Step 6: Commit**

```bash
git add crates/adapters/Cargo.toml crates/adapters/src/kms/mod.rs
git commit -m "fix: generate RFC 7517 compliant JWK from KMS public keys"
```

---

### Task 3: Verify JWT signature before revoking sessions for access tokens

**Files:**
- Modify: `crates/core/src/ports/key_manager.rs` (add `verify` to trait)
- Modify: `crates/adapters/src/local_keys/mod.rs` (implement `verify`)
- Modify: `crates/adapters/src/kms/mod.rs` (implement `verify` via KMS)
- Modify: `crates/adapters/src/noop/mod.rs` (implement `verify`)
- Modify: `crates/test-utils/src/lib.rs` (implement `verify` for mock)
- Modify: `crates/core/src/service/revoke.rs` (verify before revoking)
- Test: `crates/core/tests/revoke.rs`

- [ ] **Step 1: Add `verify` to KeyManager trait**

In `crates/core/src/ports/key_manager.rs`:

```rust
#[async_trait]
pub trait KeyManager: Send + Sync {
    /// Sign a byte payload, return the signature
    async fn sign(&self, payload: &[u8]) -> Result<Vec<u8>>;

    /// Verify a signature against a payload. Returns Ok(true) if valid,
    /// Ok(false) if invalid, Err on infrastructure failure.
    async fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<bool>;

    /// Return the public key in JWK format for the JWKS endpoint
    async fn public_jwk(&self) -> Result<serde_json::Value>;

    /// Key algorithm identifier (e.g., "EdDSA", "ES256")
    fn algorithm(&self) -> &str;

    /// Key ID for the JWT kid header
    fn key_id(&self) -> &str;
}
```

- [ ] **Step 2: Build to see all compile errors**

Run: `cargo build 2>&1 | head -50`
Expected: Compile errors in every KeyManager implementor (LocalKeyManager, KmsKeyManager, NoopKeyManager, MockKeyManager).

- [ ] **Step 3: Implement verify for LocalKeyManager (Ed25519)**

In `crates/adapters/src/local_keys/mod.rs`, add to the `impl KeyManager for LocalKeyManager` block:

```rust
    async fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<bool> {
        use ed25519_dalek::{Signature, Verifier};
        let sig_bytes: [u8; 64] = signature.try_into().map_err(|_| Error::KeyError {
            detail: format!("invalid Ed25519 signature length: expected 64, got {}", signature.len()),
        })?;
        let sig = Signature::from_bytes(&sig_bytes);
        Ok(self.signing_key.verifying_key().verify(payload, &sig).is_ok())
    }
```

- [ ] **Step 4: Implement verify for KmsKeyManager**

In `crates/adapters/src/kms/mod.rs`, add to the `impl KeyManager for KmsKeyManager` block:

```rust
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
```

- [ ] **Step 5: Implement verify for NoopKeyManager**

In `crates/adapters/src/noop/mod.rs`:

```rust
    async fn verify(&self, _payload: &[u8], _signature: &[u8]) -> Result<bool> {
        Err(Error::KeyError {
            detail: "NoopKeyManager: verification not available in admin-only mode".into(),
        })
    }
```

- [ ] **Step 6: Implement verify for MockKeyManager**

In `crates/test-utils/src/lib.rs`, add to the `impl KeyManager for MockKeyManager` block:

```rust
    async fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<bool> {
        use ed25519_dalek::{Signature, Verifier};
        let sig_bytes: [u8; 64] = signature.try_into().map_err(|_| Error::KeyError {
            detail: format!("invalid Ed25519 signature length: expected 64, got {}", signature.len()),
        })?;
        let sig = Signature::from_bytes(&sig_bytes);
        Ok(self.signing_key.verifying_key().verify(payload, &sig).is_ok())
    }
```

- [ ] **Step 7: Verify build**

Run: `cargo build`
Expected: Compiles without errors.

- [ ] **Step 8: Write test for revoke with forged JWT**

In `crates/core/tests/revoke.rs`, add:

```rust
#[tokio::test]
async fn revoke_forged_access_token_does_not_revoke_sessions() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Exchange to create a session
    let _response = do_exchange(&svc).await;
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1);
    let user_id = sessions[0].user_id.clone();

    // Craft a forged JWT with the real user's sub but an invalid signature
    let forged_header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","typ":"JWT"}"#);
    let forged_payload = URL_SAFE_NO_PAD.encode(
        format!(r#"{{"sub":"{user_id}","iss":"https://auth.test.com","iat":0,"exp":9999999999}}"#).as_bytes()
    );
    let forged_sig = URL_SAFE_NO_PAD.encode(&[0u8; 64]); // bogus signature
    let forged_jwt = format!("{forged_header}.{forged_payload}.{forged_sig}");

    // Revoke with the forged JWT
    let revoke_req = RevokeRequest {
        token: forged_jwt,
        token_type_hint: Some("access_token".to_string()),
    };
    let result = svc.revoke(revoke_req).await;
    assert!(result.is_ok(), "revoke should return Ok per RFC 7009");

    // Sessions should NOT be revoked because the JWT signature is invalid
    let sessions = repo.get_all_sessions().await;
    assert_eq!(
        sessions.len(),
        1,
        "sessions should NOT be revoked for a forged access token"
    );
}
```

- [ ] **Step 9: Run test to verify it fails (current code revokes without verifying)**

Run: `cargo nextest run -p oidc-exchange-core --test revoke revoke_forged_access_token`
Expected: FAIL --- assertion `sessions.len() == 1` fails because current code revokes without verification.

- [ ] **Step 10: Fix revoke.rs to verify JWT signature**

Replace the entire `revoke.rs` content with:

```rust
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::service::AppService;

pub struct RevokeRequest {
    pub token: String,
    pub token_type_hint: Option<String>, // "refresh_token" or "access_token"
}

impl AppService {
    pub async fn revoke(&self, request: RevokeRequest) -> Result<()> {
        match request.token_type_hint.as_deref() {
            Some("access_token") => {
                // Verify the JWT was issued by us before revoking sessions.
                // If verification fails, silently succeed per RFC 7009.
                if let Some(user_id) = self.verify_and_extract_sub(&request.token).await {
                    let _ = self.session_repo.revoke_all_user_sessions(&user_id).await;
                }
                Ok(())
            }
            Some("refresh_token") | None => {
                let token_hash = hex::encode(Sha256::digest(request.token.as_bytes()));
                let _ = self.session_repo.revoke_session(&token_hash).await;
                Ok(())
            }
            Some(_) => {
                let token_hash = hex::encode(Sha256::digest(request.token.as_bytes()));
                let _ = self.session_repo.revoke_session(&token_hash).await;
                Ok(())
            }
        }
    }

    /// Verify a JWT's signature using the service's key manager, then extract the `sub` claim.
    /// Returns None if the token is malformed or the signature is invalid.
    async fn verify_and_extract_sub(&self, token: &str) -> Option<String> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return None;
        }

        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let signature_bytes = URL_SAFE_NO_PAD.decode(parts[2]).ok()?;

        // Verify signature using the service's key manager
        let valid = self
            .keys
            .verify(signing_input.as_bytes(), &signature_bytes)
            .await
            .ok()?;

        if !valid {
            return None;
        }

        // Signature verified --- safe to extract sub
        let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
        payload.get("sub")?.as_str().map(|s| s.to_string())
    }
}
```

- [ ] **Step 11: Run all revoke tests**

Run: `cargo nextest run -p oidc-exchange-core --test revoke`
Expected: All pass, including the new forged-JWT test.

- [ ] **Step 12: Run full test suite to check for regressions**

Run: `cargo nextest run`
Expected: All pass.

- [ ] **Step 13: Commit**

```bash
git add crates/core/src/ports/key_manager.rs \
       crates/core/src/service/revoke.rs \
       crates/core/tests/revoke.rs \
       crates/adapters/src/local_keys/mod.rs \
       crates/adapters/src/kms/mod.rs \
       crates/adapters/src/noop/mod.rs \
       crates/test-utils/src/lib.rs
git commit -m "fix: verify JWT signature before revoking sessions for access tokens"
```

---

### Task 4: Add expired session cleanup to SessionRepository

**Files:**
- Modify: `crates/core/src/ports/repository.rs` (add trait method)
- Modify: `crates/adapters/src/dynamo/mod.rs`
- Modify: `crates/adapters/src/postgres/mod.rs`
- Modify: `crates/adapters/src/sqlite/mod.rs`
- Modify: `crates/adapters/src/valkey/mod.rs`
- Modify: `crates/adapters/src/lmdb/mod.rs`
- Modify: `crates/test-utils/src/lib.rs`
- Test: `crates/core/tests/refresh.rs` or new `crates/core/tests/cleanup.rs`

- [ ] **Step 1: Add `cleanup_expired_sessions` to the SessionRepository trait**

In `crates/core/src/ports/repository.rs`:

```rust
#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn store_refresh_token(&self, session: &Session) -> Result<()>;
    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>>;
    async fn revoke_session(&self, token_hash: &str) -> Result<()>;
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;
    async fn count_active_sessions(&self) -> Result<u64>;

    /// Delete all sessions whose `expires_at` is in the past.
    /// Returns the number of sessions deleted.
    async fn cleanup_expired_sessions(&self) -> Result<u64>;
}
```

- [ ] **Step 2: Build to see all compile errors**

Run: `cargo build 2>&1 | head -30`
Expected: Every SessionRepository implementor fails to compile.

- [ ] **Step 3: Implement for MockRepository**

In `crates/test-utils/src/lib.rs`, add to `impl SessionRepository for MockRepository`:

```rust
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let mut state = self.state.lock().await;
        let now = Utc::now();
        let before = state.sessions.len();
        state.sessions.retain(|_, s| s.expires_at > now);
        Ok((before - state.sessions.len()) as u64)
    }
```

- [ ] **Step 4: Implement for DynamoRepository**

In `crates/adapters/src/dynamo/mod.rs`, add to `impl SessionRepository for DynamoRepository`:

```rust
    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        // DynamoDB TTL handles cleanup automatically, but this provides
        // a manual sweep for items where TTL hasn't fired yet.
        let now = Utc::now();
        let mut deleted: u64 = 0;
        let mut exclusive_start_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let mut scan = self
                .client
                .scan()
                .table_name(&self.table_name)
                .filter_expression("sk = :sk AND expires_at < :now")
                .expression_attribute_values(":sk", AttributeValue::S("SESSION".to_string()))
                .expression_attribute_values(":now", AttributeValue::S(now.to_rfc3339()))
                .projection_expression("pk, sk");

            if let Some(ref start_key) = exclusive_start_key {
                scan = scan.set_exclusive_start_key(Some(start_key.clone()));
            }

            let result = scan.send().await.map_err(Self::store_err)?;
            let items = result.items.unwrap_or_default();

            for chunk in items.chunks(25) {
                let delete_requests: Vec<_> = chunk
                    .iter()
                    .map(|item| {
                        let pk = item.get("pk").cloned().unwrap_or_else(|| {
                            AttributeValue::S("UNKNOWN".to_string())
                        });
                        let sk = item.get("sk").cloned().unwrap_or_else(|| {
                            AttributeValue::S("UNKNOWN".to_string())
                        });

                        aws_sdk_dynamodb::types::WriteRequest::builder()
                            .delete_request(
                                aws_sdk_dynamodb::types::DeleteRequest::builder()
                                    .key("pk", pk)
                                    .key("sk", sk)
                                    .build()
                                    .expect("valid delete request"),
                            )
                            .build()
                    })
                    .collect();

                deleted += delete_requests.len() as u64;

                self.client
                    .batch_write_item()
                    .request_items(&self.table_name, delete_requests)
                    .send()
                    .await
                    .map_err(Self::store_err)?;
            }

            match result.last_evaluated_key {
                Some(key) => exclusive_start_key = Some(key),
                None => break,
            }
        }

        Ok(deleted)
    }
```

- [ ] **Step 5: Implement for PostgresRepository**

In `crates/adapters/src/postgres/mod.rs`, add to `impl SessionRepository for PostgresRepository`:

```rust
    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at < NOW()")
            .execute(&self.pool)
            .await
            .map_err(Self::store_err)?;

        Ok(result.rows_affected())
    }
```

- [ ] **Step 6: Implement for SqliteRepository**

In `crates/adapters/src/sqlite/mod.rs`, add to `impl SessionRepository for SqliteRepository`:

```rust
    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let now_str = Utc::now().to_rfc3339();
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at < ?1")
            .bind(&now_str)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        Ok(result.rows_affected())
    }
```

- [ ] **Step 7: Implement for ValkeySessionRepository**

In `crates/adapters/src/valkey/mod.rs`, add to `impl SessionRepository for ValkeySessionRepository`:

```rust
    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        // Valkey/Redis TTL handles expiration automatically.
        // This is a no-op since keys with TTL are deleted by the server.
        Ok(0)
    }
```

- [ ] **Step 8: Implement for LmdbSessionRepository**

In `crates/adapters/src/lmdb/mod.rs`, add to `impl SessionRepository for LmdbSessionRepository`:

```rust
    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> oidc_exchange_core::error::Result<u64> {
        let env = self.env.clone();
        let sessions_db = self.sessions;
        let user_sessions_db = self.user_sessions;

        tokio::task::spawn_blocking(move || {
            let now = Utc::now();

            // Collect expired session keys
            let rtxn = env.read_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            let mut to_delete: Vec<(String, String)> = Vec::new(); // (token_hash, user_id)
            let iter = sessions_db.iter(&rtxn).map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            for result in iter {
                let (key, bytes) = result.map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
                if let Ok(session) = serde_json::from_slice::<Session>(bytes) {
                    if session.expires_at <= now {
                        to_delete.push((key.to_owned(), session.user_id.clone()));
                    }
                }
            }
            drop(rtxn);

            if to_delete.is_empty() {
                return Ok(0);
            }

            let deleted = to_delete.len() as u64;
            let mut wtxn = env.write_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            for (token_hash, user_id) in &to_delete {
                sessions_db
                    .delete(&mut wtxn, token_hash.as_str())
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;

                let index_key = LmdbSessionRepository::user_session_key(user_id, token_hash);
                user_sessions_db
                    .delete(&mut wtxn, index_key.as_str())
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            }

            wtxn.commit().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            Ok(deleted)
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }
```

- [ ] **Step 9: Verify build**

Run: `cargo build`
Expected: Compiles without errors.

- [ ] **Step 10: Write test for cleanup**

Add to `crates/core/tests/refresh.rs` (or a new `crates/core/tests/cleanup.rs`):

```rust
#[tokio::test]
async fn cleanup_expired_sessions_removes_stale_entries() {
    let repo = MockRepository::new();

    let now = chrono::Utc::now();

    // Store an expired session
    let expired_session = Session {
        user_id: "usr_1".to_string(),
        refresh_token_hash: "hash_expired".to_string(),
        provider: "mock".to_string(),
        expires_at: now - chrono::Duration::hours(1),
        device_id: None,
        user_agent: None,
        ip_address: None,
        created_at: now - chrono::Duration::hours(25),
    };
    repo.store_refresh_token(&expired_session).await.unwrap();

    // Store an active session
    let active_session = Session {
        user_id: "usr_2".to_string(),
        refresh_token_hash: "hash_active".to_string(),
        provider: "mock".to_string(),
        expires_at: now + chrono::Duration::hours(24),
        device_id: None,
        user_agent: None,
        ip_address: None,
        created_at: now,
    };
    repo.store_refresh_token(&active_session).await.unwrap();

    assert_eq!(repo.get_all_sessions().await.len(), 2);

    // Cleanup
    let deleted = repo.cleanup_expired_sessions().await.unwrap();
    assert_eq!(deleted, 1, "should delete 1 expired session");

    let remaining = repo.get_all_sessions().await;
    assert_eq!(remaining.len(), 1, "should have 1 active session left");
    assert_eq!(remaining[0].refresh_token_hash, "hash_active");
}
```

- [ ] **Step 11: Run cleanup test**

Run: `cargo nextest run cleanup_expired`
Expected: PASS.

- [ ] **Step 12: Run full test suite**

Run: `cargo nextest run`
Expected: All pass.

- [ ] **Step 13: Commit**

```bash
git add crates/core/src/ports/repository.rs \
       crates/adapters/src/dynamo/mod.rs \
       crates/adapters/src/postgres/mod.rs \
       crates/adapters/src/sqlite/mod.rs \
       crates/adapters/src/valkey/mod.rs \
       crates/adapters/src/lmdb/mod.rs \
       crates/test-utils/src/lib.rs \
       crates/core/tests/cleanup.rs
git commit -m "feat: add cleanup_expired_sessions to SessionRepository trait and all adapters"
```
