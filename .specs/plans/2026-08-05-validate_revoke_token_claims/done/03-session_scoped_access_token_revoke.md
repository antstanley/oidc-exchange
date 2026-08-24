# Task 03 — Session-scoped access-token revocation

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/changes/2026-08-05-validate_revoke_token_claims.md](../../../changes/2026-08-05-validate_revoke_token_claims.md) §Revocation (`revoke.rs`) and implementation notes 5–8; its decisions *A credential revokes only its own session* and *Failed revocation is recorded, not silent*.
**Depends on:** 01 (build — valid access tokens carry the session `sid`); 02 (build — only `validate_access_token` may release typed claims)
**Produces:** an access-token revoke validates once, revokes only `claims.sid`, emits the required success/failure audit outcome, returns 200 for token-state failures, and still propagates repository/audit infrastructure failures.
**Pointers:** `crates/core/src/service/revoke.rs:28-155`; `crates/core/src/service/mod.rs:102-138`; `crates/core/src/domain/audit.rs:37-59`; `crates/core/tests/revoke.rs:128-440`; `crates/server/tests/routes.rs:307-380`.

## Steps

- [x] Delete `verify_and_extract_sub` and its now-unused base64/`serde_json` imports. The
  access-token arm must call `validate_access_token` and never decode/read an untyped payload or
  call `revoke_all_user_sessions`.
- [x] Extract or reshape the existing refresh-token lookup → one-session revoke → success-audit
  sequence into a helper that accepts a session hash and request context. Preserve the current
  idempotent missing-session behavior: lookup `None` returns `Ok(())` and emits no success event;
  lookup/revoke backend errors propagate.
- [x] For a valid access token, pass `claims.sid` to that one-session helper. Assert the validator
  postconditions needed at the core-to-adapter boundary (non-empty session identifier and stored
  session user id where an audit actor is emitted). A valid access token must not inspect or use
  `claims.sub` for revocation authority.
- [x] For every validation rejection, leave sessions untouched, emit exactly one fixed-reason
  failure audit event, and return `Ok(())` to preserve RFC 7009’s indistinguishable client result.
  Resolve the external audit dependency before coding: if the audit/throttling sibling has merged,
  use its canonical security-event/channel API; otherwise use the current `ValidationFailed`
  surface and do not invent `AuthenticationFailed`, durability config, or a rate limiter. Keep
  success and failure emission symmetric with respect to the available durability behavior.
- [x] Rework existing revoke tests: a valid access token from one of two same-user exchanges
  removes its own session only and emits `TokenRevocation`; its sibling session remains. Convert
  forged/failed-verification tests from “emits nothing” to one failure event, still 200 and no
  session mutation. Add the source-spec claim/header negatives (expired `exp`, future `nbf`, wrong
  `iss`, wrong `aud`, wrong `alg`, wrong `typ`, missing `exp`, missing `sid`) with assertions for
  one failure event and unchanged sessions.
- [x] Retain coverage for refresh/unknown-hint behavior, idempotent unknown refresh tokens, and
  store failure propagation. Update server route coverage only where needed to show the access
  validation failure still returns 200 and never reaches a failing store; do not change generic
  server error mapping.
- [x] Keep every modified/new function bounded and assertion-rich, propagate errors with `?`, and
  make audit reasons fixed/non-secret. Do not add store methods, migrations, or an account-wide
  logout endpoint.

## Definition of done

- [x] `/revoke` with a valid access token removes exactly the session named by `sid`, audits one
  `TokenRevocation`, and leaves other sessions for the same subject intact.
- [x] Every malformed, header-invalid, signature-invalid, claim-missing, issuer/audience-invalid,
  or time-invalid access token removes nothing, emits exactly one compatible fixed-reason failure
  event, and yields 200 at the HTTP route.
- [x] A repository failure remains observable as an error/503, while token-state rejection never
  queries/revokes through the session store; audit failures follow the applicable branch’s existing
  durability contract symmetrically.
- [x] Refresh-token revocation semantics and admin-only `revoke_all_user_sessions` behavior remain
  unchanged; no persistence/port API changes appear in the diff.
- [x] Core revoke and affected server tests, format, and clippy pass; report the full workspace
  test result without fixing the documented unrelated three config failures.
- [x] Do not create a done certificate or any `*-certificate.md` file.

## Completion notes (2026-08-22)

- `verify_and_extract_sub` is deleted along with its base64/serde_json imports; the access-token
  arm calls `validate_access_token` and nothing else — no untyped decode, no
  `revoke_all_user_sessions` reachable from `/revoke`.
- The refresh lookup → single-session revoke → success-audit sequence is now one helper,
  `revoke_one_session(&session_hash, &RevokeRequest)`, shared by both arms: lookup `None`
  returns `Ok(())` silently (idempotent RFC 7009 delete), backend errors propagate (→ 503).
  Boundary assertions live there: non-empty stored `user_id`, and a hash-keyed lookup must
  return the matching row.
- Rejection handling is `emit_rejection`: exactly one `ValidationFailed` event at **Info**
  severity with the validator's fixed reason, actor `None` (no claim of an unvalidated token is
  recorded), then `Ok(())`. Severity choice rationale: identical to the success-path
  `TokenRevocation` emission, so both branches share durability semantics under any audit
  config — neither can become an existence oracle when the sink degrades — and Info survives the
  default `emit_threshold`, so the attempt is actually visible to operators.
- External audit dependency resolved per plan decision: the audit/throttling sibling has NOT
  merged here, so the current `ValidationFailed` surface is used; no `AuthenticationFailed`,
  durability config, or rate limiter was invented.
- Core tests reworked/added: sibling-session survival (`...removes_only_the_session_its_sid_names`),
  valid-token single `TokenRevocation` (`revoke_valid_access_token_emits_token_revocation`,
  replacing the `AllSessionsRevoked` variant), forged token now emits exactly one failure event
  while sessions survive, an eight-case claim/header negative matrix (expired exp, future nbf,
  wrong iss, wrong aud, wrong alg, wrong typ, missing exp, missing sid) asserting Ok + one
  failure event + untouched store per case, and store-failure propagation for both the mutating
  and lookup paths. Refresh/default-hint/idempotent-unknown coverage retained unchanged.
- Server routes: added `revoke_valid_access_token_removes_only_its_own_session` proving the HTTP
  contract end-to-end (revoked session's refresh dies 401, same-user sibling still refreshes
  200); existing 503-on-store-failure and 200-on-validation-failure route tests pass unchanged.
- Workspace gates after this task: fmt clean; clippy `-D warnings` clean (the TEMPORARY(03)
  dead-code allowances from task 02 are removed); `cargo nextest run --workspace` → 401 passed /
  27 skipped (399 before this task: +3 new, −1 converted into the matrix).
