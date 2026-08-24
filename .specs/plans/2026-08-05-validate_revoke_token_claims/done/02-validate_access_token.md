# Task 02 — Validate first-party access tokens

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/changes/2026-08-05-validate_revoke_token_claims.md](../../../changes/2026-08-05-validate_revoke_token_claims.md) §`Validate access token` and implementation note 4; its decisions *One validator for first-party tokens*, *Required claims are parse-enforced*, *No `jsonwebtoken` dependency*, and *60 seconds of clock skew*.
**Depends on:** 01 (contract — validator deserializes required `AccessTokenClaims.sid` and pins the `at+jwt` header task 01 mints)
**Produces:** `AppService::validate_access_token(&str) -> Result<AccessTokenClaims, &'static str>` validates a service-minted access JWT in the specified order and returns only fixed, non-attacker-derived rejection reasons.
**Pointers:** `crates/core/src/service/mod.rs:7-100`; `crates/core/src/service/revoke.rs:129-155` (replaceable parsing pattern only; task 03 removes it); `crates/core/src/ports/key_manager.rs`; `crates/core/tests/revoke.rs:250-413`; `crates/test-utils` `MockKeyManager` implementation.

## Steps

- [x] Add named time constants in `service/mod.rs` for the 60-second skew and any declared JWT
  segment bound. Implement `pub(crate) async fn validate_access_token(&self, token: &str) ->
  std::result::Result<AccessTokenClaims, &'static str>` immediately beside the minting helper.
- [x] Validate in the source-spec order without inspecting payload claims before signature success:
  require exactly three non-empty base64url-no-pad-decodable segments; decode a typed header and
  require `alg == keys.algorithm()`, `kid == keys.key_id()`, `typ == "at+jwt"`; then call
  `keys.verify` over the original `header.payload` bytes and reject false or port errors.
- [x] Only after a successful signature, deserialize the payload as `AccessTokenClaims`; reject
  missing/ill-typed required `sub`, `iss`, `aud`, `iat`, `exp`, or `sid` through serde rather than
  optional claim reads. If `nbf` is accepted as an optional payload claim, parse it only after the
  typed required payload succeeds and reject malformed/non-numeric values.
- [x] Pin `iss` to `config.server.issuer` and `aud` to
  `config.token.audience.clone().unwrap_or_default()`. Check `exp`, `iat`, and optional `nbf`
  against one captured `Utc::now()` timestamp with the named 60-second leeway, and reject blank
  `sub` or `sid`. Make the exact comparison direction and edge behavior explicit in tests.
- [x] Keep all rejection strings fixed constants or fixed literals suitable only for the audit
  reason; never include token bytes, decoded header/payload content, key-manager details, or serde
  errors. Preserve core’s no-`jsonwebtoken` dependency and no-infrastructure dependency rule.
- [x] Add focused core tests, preferably alongside revoke integration tests or a new narrow service
  helper suite, for a valid minted token and every negative boundary: wrong segment/base64 shape;
  wrong `alg`, `kid`, and `typ`; bad signature; missing `exp` and `sid`; wrong issuer/audience;
  expired token; future `iat`; future `nbf`; blank `sub`/`sid`; and exact clock-skew boundary.
  Use deterministic signed mutations/helpers, not a real clock sleep.
- [x] Apply the assertion-density rule to the validator and helper functions and keep each under the
  70-line review gate by extracting narrow, typed parsing/checking helpers if necessary.

## Definition of done

- [x] No caller can obtain `AccessTokenClaims` from a first-party JWT until header pinning and
  signature verification succeed; all required registered claims are deserialization-required.
- [x] A correctly minted, currently valid access token is returned with its typed claims; wrong
  type/key metadata, bad signature, issuer/audience mismatch, temporal invalidity, missing claims,
  or blank identifiers each return a fixed rejection and no claims.
- [x] Negative-space tests cover each validator rejection family and the documented 60-second skew
  boundary; they use signed tokens where required so each assertion reaches the intended check.
- [x] Format, clippy, and relevant core tests pass; run and report workspace testing separately,
  preserving the known unrelated `providers.*.adapter` config failures if still present.
- [x] Do not create a done certificate or any `*-certificate.md` file.

## Completion notes (2026-08-22)

- Implemented exactly in the source-spec order: segment shape → typed-header pin (alg/kid/typ,
  all required struct fields so missing ones are parse failures) → `keys.verify` over the
  original serialized bytes → typed `AccessTokenClaims` payload (sub/iss/aud/iat/exp/sid
  required) → issuer/audience pins from config → one captured `Utc::now()` compared with
  `CLOCK_SKEW_SECS` (60) → optional `nbf` parsed only after the required typed claims succeed →
  non-blank `sub`/`sid`.
- Boundary semantics are explicit and tested at both edges: expired iff
  `now > exp + skew` (so `now == exp + skew` is still valid); future-issued iff
  `iat > now + skew`; not-yet-valid iff `nbf > now + skew`; all comparisons saturating in `u64`
  so absurd claim values cannot wrap into acceptance. No clock sleeps — tests sign tokens with
  offsets computed from the captured second.
- Rejection reasons are fixed module constants (`REASON_*`), never derived from token bytes,
  header/payload content, key-manager details, or serde errors. Taxonomy: header-shape failures
  (including a missing `typ`/`kid` field) report "not an access token"; value mismatches against
  the key manager report the wrong-key reason. No `jsonwebtoken` dependency added.
- The focused suite lives in-module (`service::validate_access_token_tests`) because the method
  is `pub(crate)`. It intentionally does NOT use `oidc-exchange-test-utils`: that crate depends
  on core, so its mocks would compile two copies of core into one graph. Ports the validator
  must never touch are panicking stubs (any I/O fails the test), and signing uses a
  deterministic toy key manager (`sign(p) == p`) shared with the service under test.
- Twelve new tests: valid token; malformed shapes (6 shapes + non-base64); wrong typ (+ missing
  typ); wrong kid / wrong alg / missing kid; tampered payload + corrupted signature; missing
  exp / missing sid; wrong iss / wrong aud; expiry skew edges (accept at −60, reject at −61);
  future-iat skew edges; nbf edges + non-numeric nbf + historical nbf; blank sub / blank sid.
- Workspace gates: fmt clean; clippy `-D warnings` clean; `cargo nextest run --workspace` →
  399 passed / 27 skipped (388 after task 01; this suite adds 11 test functions).
- TEMPORARY `#[allow(dead_code)]` annotations (each marked `TEMPORARY(03)`) keep this commit
  green because `/revoke` does not consume the validator until task 03; they are removed in the
  task 03 commit.
