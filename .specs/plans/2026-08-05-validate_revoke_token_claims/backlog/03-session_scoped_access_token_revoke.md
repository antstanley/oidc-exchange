# Task 03 — Session-scoped access-token revocation

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/changes/2026-08-05-validate_revoke_token_claims.md](../../../changes/2026-08-05-validate_revoke_token_claims.md) §Revocation (`revoke.rs`) and implementation notes 5–8; its decisions *A credential revokes only its own session* and *Failed revocation is recorded, not silent*.
**Depends on:** 01 (build — valid access tokens carry the session `sid`); 02 (build — only `validate_access_token` may release typed claims)
**Produces:** an access-token revoke validates once, revokes only `claims.sid`, emits the required success/failure audit outcome, returns 200 for token-state failures, and still propagates repository/audit infrastructure failures.
**Pointers:** `crates/core/src/service/revoke.rs:28-155`; `crates/core/src/service/mod.rs:102-138`; `crates/core/src/domain/audit.rs:37-59`; `crates/core/tests/revoke.rs:128-440`; `crates/server/tests/routes.rs:307-380`.

## Steps

- [ ] Delete `verify_and_extract_sub` and its now-unused base64/`serde_json` imports. The
  access-token arm must call `validate_access_token` and never decode/read an untyped payload or
  call `revoke_all_user_sessions`.
- [ ] Extract or reshape the existing refresh-token lookup → one-session revoke → success-audit
  sequence into a helper that accepts a session hash and request context. Preserve the current
  idempotent missing-session behavior: lookup `None` returns `Ok(())` and emits no success event;
  lookup/revoke backend errors propagate.
- [ ] For a valid access token, pass `claims.sid` to that one-session helper. Assert the validator
  postconditions needed at the core-to-adapter boundary (non-empty session identifier and stored
  session user id where an audit actor is emitted). A valid access token must not inspect or use
  `claims.sub` for revocation authority.
- [ ] For every validation rejection, leave sessions untouched, emit exactly one fixed-reason
  failure audit event, and return `Ok(())` to preserve RFC 7009’s indistinguishable client result.
  Resolve the external audit dependency before coding: if the audit/throttling sibling has merged,
  use its canonical security-event/channel API; otherwise use the current `ValidationFailed`
  surface and do not invent `AuthenticationFailed`, durability config, or a rate limiter. Keep
  success and failure emission symmetric with respect to the available durability behavior.
- [ ] Rework existing revoke tests: a valid access token from one of two same-user exchanges
  removes its own session only and emits `TokenRevocation`; its sibling session remains. Convert
  forged/failed-verification tests from “emits nothing” to one failure event, still 200 and no
  session mutation. Add the source-spec claim/header negatives (expired `exp`, future `nbf`, wrong
  `iss`, wrong `aud`, wrong `alg`, wrong `typ`, missing `exp`, missing `sid`) with assertions for
  one failure event and unchanged sessions.
- [ ] Retain coverage for refresh/unknown-hint behavior, idempotent unknown refresh tokens, and
  store failure propagation. Update server route coverage only where needed to show the access
  validation failure still returns 200 and never reaches a failing store; do not change generic
  server error mapping.
- [ ] Keep every modified/new function bounded and assertion-rich, propagate errors with `?`, and
  make audit reasons fixed/non-secret. Do not add store methods, migrations, or an account-wide
  logout endpoint.

## Definition of done

- [ ] `/revoke` with a valid access token removes exactly the session named by `sid`, audits one
  `TokenRevocation`, and leaves other sessions for the same subject intact.
- [ ] Every malformed, header-invalid, signature-invalid, claim-missing, issuer/audience-invalid,
  or time-invalid access token removes nothing, emits exactly one compatible fixed-reason failure
  event, and yields 200 at the HTTP route.
- [ ] A repository failure remains observable as an error/503, while token-state rejection never
  queries/revokes through the session store; audit failures follow the applicable branch’s existing
  durability contract symmetrically.
- [ ] Refresh-token revocation semantics and admin-only `revoke_all_user_sessions` behavior remain
  unchanged; no persistence/port API changes appear in the diff.
- [ ] Core revoke and affected server tests, format, and clippy pass; report the full workspace
  test result without fixing the documented unrelated three config failures.
- [ ] Do not create a done certificate or any `*-certificate.md` file.
