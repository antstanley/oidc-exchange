# Task 08 — Family `sid` and access-token revocation integration

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Build access token / Validate access token / Revocation and sibling re-pointing only.
**Depends on:** 07 · exchange_refresh_rotation_flow; 03 · sql_session_adapters; 04 · lmdb_session_adapter; 05 · valkey_session_adapter; 06 · dynamodb_session_adapter; sibling `2026-08-05-validate_revoke_token_claims.md` merged/available
**Produces:** `family_id` as stable access-token `sid`, family-scoped access-token revocation, and cutover validation using the sibling's claims contract.
**Pointers:** `crates/core/src/service/{mod,exchange,refresh,revoke}.rs`; `crates/core/src/domain/token.rs`; core/server revoke and token tests; sibling change spec (external dependency).

## Steps

- [x] Consume—not recreate—the sibling's validated `AccessTokenClaims`/`sid` seam; change `build_access_token` and call sites to supply the stable family id.
- [x] Enforce `fam_` + lowercase-ULID `sid` validation and fail closed for hash-form pre-rotation tokens with the sibling's one-rejection/audit contract.
- [x] Change access-token revoke from hash/session handling to `revoke_family(claims.sid)`; leave refresh-token revoke semantics distinct.
- [x] Update token/revoke assertions: issued and rotated access tokens carry unchanged family `sid`; valid access-token revoke removes live and retired generations; malformed/hash `sid` revokes nothing.

## Definition of done

- [x] No code in this task duplicates or alters the sibling's JWT typ/signature/registered-claim work; it only uses its completed API.
- [x] Every issuance path provides a family id and the access-token `sid` remains invariant across rotations.
- [x] Invalid/hash `sid` fails before any revocation mutation; valid access-token revocation removes precisely one family's retained state.
- [x] Core/server tests cover valid, malformed, and pre-rotation-hash `sid` cases.
- [x] Done certificates remain intentionally absent.

## Completion notes

- **Vendored seam (deviation, per wave directive).** The task's stated dependency — the sibling `2026-08-05-validate_revoke_token_claims` contract (PR #19) — is *not* merged on this branch. To keep this PR self-contained, only the minimal `sid` slice of that contract is vendored: `AccessTokenClaims.sid: String` (plain field ⇒ required on deserialization ⇒ payloads without it fail closed), populated at both mint sites through `build_access_token(&user, family_id)`. Nothing else from PR #19 was recreated: no `at+jwt` typ-header pinning, no registered-claim validation order, no new audit event type — every vendored site carries a `VENDORED SEAM (task 08)` comment naming PR #19 for merge-time reconciliation. The fail-closed rejection uses a fixed-reason `ValidationFailed` at Debug as this branch's closest analogue to the sibling's `AuthenticationFailed`.
- Mint sites (`service/mod.rs`, called from `exchange.rs` and both refresh paths): exchange passes the family it just minted; enabled rotation passes the replacement's asserted-well-formed family; rotation-disabled mode passes the session's existing family, tolerating the empty-string legacy sentinel because minting there would be rotation work — such tokens fail closed at consumption like any hash-form sid.
- `revoke.rs`: signature verification stays first (failure ⇒ RFC 7009 silence, unchanged); after verification the payload parses into typed claims and `is_valid_family_id(&claims.sid)` gates the mutation. Usable tokens take `revoke_family(sid)` — removing live generation **and** retained retirement records — audited as `TokenRevocation` with detail `{family_id, sessions_revoked}`. Unusable sids revoke nothing and emit exactly one fixed-reason rejection. The refresh-token arm remains hash/session-scoped and untouched.
- `MockKeyManager::sign_payload_jws` added to test-utils so integration tests can present validly-signed tokens carrying claim values the service would never mint (hash-form/blank/malformed/missing sid); mirrors the task-07 backdating hook in being test infrastructure only.
- Tests: `revoke_access_token_revokes_its_family_live_and_retired` (sibling family survives, retired record removed, sibling still redeems), `revoke_valid_access_token_emits_token_revocation_with_family_count`, `unusable_sids_fail_closed_before_any_mutation` (five shapes, one fixed reason, byte-identical store), `forged_signature_stays_silent_under_debug_threshold`, `sid_is_invariant_across_rotation`, plus exchange happy-path assertions that the issued token's sid equals the stored generation-0 family.
- Gates at commit: nextest 454 passed / 50 skipped (+15 vs the 439 baseline); fmt and clippy `-D warnings` clean.
