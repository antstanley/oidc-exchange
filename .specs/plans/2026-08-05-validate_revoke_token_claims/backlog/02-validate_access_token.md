# Task 02 — Validate first-party access tokens

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/changes/2026-08-05-validate_revoke_token_claims.md](../../../changes/2026-08-05-validate_revoke_token_claims.md) §`Validate access token` and implementation note 4; its decisions *One validator for first-party tokens*, *Required claims are parse-enforced*, *No `jsonwebtoken` dependency*, and *60 seconds of clock skew*.
**Depends on:** 01 (contract — validator deserializes required `AccessTokenClaims.sid` and pins the `at+jwt` header task 01 mints)
**Produces:** `AppService::validate_access_token(&str) -> Result<AccessTokenClaims, &'static str>` validates a service-minted access JWT in the specified order and returns only fixed, non-attacker-derived rejection reasons.
**Pointers:** `crates/core/src/service/mod.rs:7-100`; `crates/core/src/service/revoke.rs:129-155` (replaceable parsing pattern only; task 03 removes it); `crates/core/src/ports/key_manager.rs`; `crates/core/tests/revoke.rs:250-413`; `crates/test-utils` `MockKeyManager` implementation.

## Steps

- [ ] Add named time constants in `service/mod.rs` for the 60-second skew and any declared JWT
  segment bound. Implement `pub(crate) async fn validate_access_token(&self, token: &str) ->
  std::result::Result<AccessTokenClaims, &'static str>` immediately beside the minting helper.
- [ ] Validate in the source-spec order without inspecting payload claims before signature success:
  require exactly three non-empty base64url-no-pad-decodable segments; decode a typed header and
  require `alg == keys.algorithm()`, `kid == keys.key_id()`, `typ == "at+jwt"`; then call
  `keys.verify` over the original `header.payload` bytes and reject false or port errors.
- [ ] Only after a successful signature, deserialize the payload as `AccessTokenClaims`; reject
  missing/ill-typed required `sub`, `iss`, `aud`, `iat`, `exp`, or `sid` through serde rather than
  optional claim reads. If `nbf` is accepted as an optional payload claim, parse it only after the
  typed required payload succeeds and reject malformed/non-numeric values.
- [ ] Pin `iss` to `config.server.issuer` and `aud` to
  `config.token.audience.clone().unwrap_or_default()`. Check `exp`, `iat`, and optional `nbf`
  against one captured `Utc::now()` timestamp with the named 60-second leeway, and reject blank
  `sub` or `sid`. Make the exact comparison direction and edge behavior explicit in tests.
- [ ] Keep all rejection strings fixed constants or fixed literals suitable only for the audit
  reason; never include token bytes, decoded header/payload content, key-manager details, or serde
  errors. Preserve core’s no-`jsonwebtoken` dependency and no-infrastructure dependency rule.
- [ ] Add focused core tests, preferably alongside revoke integration tests or a new narrow service
  helper suite, for a valid minted token and every negative boundary: wrong segment/base64 shape;
  wrong `alg`, `kid`, and `typ`; bad signature; missing `exp` and `sid`; wrong issuer/audience;
  expired token; future `iat`; future `nbf`; blank `sub`/`sid`; and exact clock-skew boundary.
  Use deterministic signed mutations/helpers, not a real clock sleep.
- [ ] Apply the assertion-density rule to the validator and helper functions and keep each under the
  70-line review gate by extracting narrow, typed parsing/checking helpers if necessary.

## Definition of done

- [ ] No caller can obtain `AccessTokenClaims` from a first-party JWT until header pinning and
  signature verification succeed; all required registered claims are deserialization-required.
- [ ] A correctly minted, currently valid access token is returned with its typed claims; wrong
  type/key metadata, bad signature, issuer/audience mismatch, temporal invalidity, missing claims,
  or blank identifiers each return a fixed rejection and no claims.
- [ ] Negative-space tests cover each validator rejection family and the documented 60-second skew
  boundary; they use signed tokens where required so each assertion reaches the intended check.
- [ ] Format, clippy, and relevant core tests pass; run and report workspace testing separately,
  preserving the known unrelated `providers.*.adapter` config failures if still present.
- [ ] Do not create a done certificate or any `*-certificate.md` file.
