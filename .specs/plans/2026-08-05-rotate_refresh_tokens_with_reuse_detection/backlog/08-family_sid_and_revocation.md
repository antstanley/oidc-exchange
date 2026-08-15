# Task 08 — Family `sid` and access-token revocation integration

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Build access token / Validate access token / Revocation and sibling re-pointing only.
**Depends on:** 07 · exchange_refresh_rotation_flow; 03 · sql_session_adapters; 04 · lmdb_session_adapter; 05 · valkey_session_adapter; 06 · dynamodb_session_adapter; sibling `2026-08-05-validate_revoke_token_claims.md` merged/available
**Produces:** `family_id` as stable access-token `sid`, family-scoped access-token revocation, and cutover validation using the sibling’s claims contract.
**Pointers:** `crates/core/src/service/{mod,exchange,refresh,revoke}.rs`; `crates/core/src/domain/token.rs`; core/server revoke and token tests; sibling change spec (external dependency).

## Steps

- [ ] Consume—not recreate—the sibling’s validated `AccessTokenClaims`/`sid` seam; change `build_access_token` and call sites to supply the stable family id.
- [ ] Enforce `fam_` + lowercase-ULID `sid` validation and fail closed for hash-form pre-rotation tokens with the sibling’s one-rejection/audit contract.
- [ ] Change access-token revoke from hash/session handling to `revoke_family(claims.sid)`; leave refresh-token revoke semantics distinct.
- [ ] Update token/revoke assertions: issued and rotated access tokens carry unchanged family `sid`; valid access-token revoke removes live and retired generations; malformed/hash `sid` revokes nothing.

## Definition of done

- [ ] No code in this task duplicates or alters the sibling’s JWT typ/signature/registered-claim work; it only uses its completed API.
- [ ] Every issuance path provides a family id and the access-token `sid` remains invariant across rotations.
- [ ] Invalid/hash `sid` fails before any revocation mutation; valid access-token revocation removes precisely one family’s retained state.
- [ ] Core/server tests cover valid, malformed, and pre-rotation-hash `sid` cases.
- [ ] Done certificates remain intentionally absent.
