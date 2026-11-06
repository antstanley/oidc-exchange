---
"@oidc-exchange/node": minor
---

Revoke a user's sessions through the internal API

Two new internal routes let a host backend act on the list `GET /internal/users/{id}/sessions` returns:

- `DELETE /internal/users/{id}/sessions/{sessionId}` revokes the one session whose opaque `ses_…` id the list returned, and answers `204`. An unknown id, another user's id, or an already-revoked id is `404`.
- `DELETE /internal/users/{id}/sessions` revokes every live session of the user and answers `200 { "sessions_revoked": <n> }`. The user stays active.

Both record an operator-attributed audit event (`SessionRevoked` / `AllSessionsRevoked`). Revocation ends refresh-token families only: access tokens already issued stay valid until they expire, as with `POST /revoke`.

The `SessionRepository::revoke_all_user_sessions` port method now returns the number of sessions removed (`u64`) instead of `()`.
