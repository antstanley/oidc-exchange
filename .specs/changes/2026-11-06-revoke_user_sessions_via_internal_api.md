# Change: Revoke one or all of a user's sessions through the internal API

**Status:** Proposed · **Date:** 2026-11-06 · **Owner:** Ant Stanley · **Target:** service (crates/core, crates/adapters, crates/server), schemas, docs

Add two internal routes. `DELETE /internal/users/{id}/sessions/{sessionId}` revokes the single session (refresh-token family) whose opaque `ses_…` id `GET /internal/users/{id}/sessions` returned, and answers `204`. `DELETE /internal/users/{id}/sessions` revokes every live session of the user and answers `200 { "sessions_revoked": <n> }`; the user stays active. Both reuse the existing family revocation (`revoke_family`, `revoke_all_user_sessions`), so retired refresh-token records go with the live generation. Both emit an operator-attributed audit event on the mandatory channel. The bulk port method changes its return from `()` to a count so the route can report it. Resolves GitHub issue #76.

---

## Motivation

A host app that renders a "where you're signed in" panel from `GET /internal/users/{id}/sessions` cannot act on it. `POST /revoke` needs the session's own refresh token, which only that device holds, and `DELETE /internal/users/{id}` soft-deletes the whole user. The host backend already holds the internal shared secret and the opaque session ids; the service only has to accept an id and an owner.

The design keeps the opaque id as the sole handle. Ownership is enforced by resolving the id against the named user's own live sessions, so an id belonging to another user is indistinguishable from an unknown one (`404`) and can never cause a revocation. Access tokens already issued stay valid until they expire, exactly as with `POST /revoke`; revocation ends the refresh-token family, not outstanding JWTs.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | `revoke_all_user_sessions` returns the count of live sessions removed; SR5 no longer says the signature is unchanged; conformance suite gains count cases |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | Admin operations table gains `admin_revoke_user_session` and `admin_revoke_user_sessions`; audit paragraph names the new events |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | Internal route table gains two rows; response paragraph |
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md) | `SecurityEvent` / `AdminMutationKind` gain session-revocation variants |
| [`.specs/service/specs/07-telemetry-and-audit.md`](../service/specs/07-telemetry-and-audit.md) | Audit section names the two operator events and their `detail` keys (the page has no event-catalogue table) |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) | DynamoDB roster paragraph: the returned count is families, not hashes |
| [`.specs/service/specs/canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | `SessionsRevokedResponse` |
| `schemas/internal-api.schema.json` | Code-side artifact: two operations, `SessionsRevokedResponse` |
| `schemas/dynamodb/table-design.json` | Code-side artifact: `revoke_all_user_sessions` access-pattern description notes the returned count |
| `README.md`, `apps/website/src/content/docs/guides/api-reference.md` | Route tables and a short section on semantics and token lifetime |
| `.changeset/` | A changeset for the user-facing routes |

---

## Proposed changes

### `02-ports-and-adapters.md` → SessionRepository (Modify)

> ```rust
> async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<u64>;
> ```
>
> Removes every live generation and retained retirement record across all of the user's families (the SR5 guarantee, family-set-wide) and returns the number of **sessions** removed, that is, families that had a live generation (including one whose `expires_at` has passed but that the reaper has not yet swept, and a pre-rotation row with no `family_id`, which counts as one). Retirement records are not counted. Idempotent: a user with no live sessions returns `Ok(0)`. Existing callers that ignore the value (suspend, delete) are unchanged.

### `02-ports-and-adapters.md` → Session-store guarantees, SR5 (Modify)

> In **SR5**, replace "(its `Result<()>` signature is unchanged)" with "and returns the number of live sessions (families with a live generation) removed". The last sentence ("Neither reports success for work it did not do") stands.

### `02-ports-and-adapters.md` → Session-store conformance suite (Modify)

> The suite gains cases every adapter and `MockRepository` must pass: `revoke_all_user_sessions` returns the number of live families removed and not the number of records; a repeated call returns `0`; it counts only the named user's families; a rotated family counts once; a pre-rotation row with no `family_id` counts as one session.

### `03-service-flows.md` → Admin operations (Add rows)

> | `admin_revoke_user_session` | `get_user_by_id`; `NotFound` when the user is absent or soft-deleted. Resolve `session_id` by mapping the user's live sessions (`list_live_sessions(user_id, now, 50)`, skipping rows with an empty `family_id`, as the list does) through `session_id_for_family`; `NotFound` when none matches, including a malformed id. Otherwise `revoke_family(family_id)`; when it removes `0` records (a concurrent revocation won) return `NotFound` and emit nothing, else emit the operator-attributed `SessionRevoked` event. |
> | `admin_revoke_user_sessions` | `get_user_by_id`; `NotFound` when the user is absent or soft-deleted. `revoke_all_user_sessions(user_id)` returns `n`; emit the operator-attributed `AllSessionsRevoked` event with `n`, including when `n = 0`. The user's status is unchanged. |

### `03-service-flows.md` → Admin operations → audit paragraph (Modify)

> Append: `admin_revoke_user_session` → `SessionRevoked` with `detail.session_id`; `admin_revoke_user_sessions` → `AllSessionsRevoked` with `detail.sessions_revoked`. Both are operator-attributed, mandatory-channel, and carry the user id as the subject. Neither `detail` ever holds a token, hash, or family id. A single revocation that finds no matching session emits nothing.
>
> Revocation ends refresh-token families only. Access tokens issued before the call remain valid until they expire, as with `POST /revoke`.

### `04-http-api.md` → Routes → Internal (Add rows)

> | DELETE | `/internal/users/{id}/sessions/{sessionId}` | revoke one live session by the opaque id the list returned; `204`; 404 for an unknown or soft-deleted user, or an id that is not one of that user's live sessions |
> | DELETE | `/internal/users/{id}/sessions` | revoke all of the user's live sessions; `200 { "sessions_revoked": <n> }`; 404 for an unknown or soft-deleted user |
>
> Repeating the single route after success is `404`. Repeating the bulk route returns `{ "sessions_revoked": 0 }`. An id belonging to another user is `404`. Auth and other error behaviour are those of every internal route. The `404` body is the existing `not_found` error.

### `01-domain-model.md` → SecurityEvent and ClientAddr (Modify)

> `AdminMutationKind` gains `SessionRevoked` and `SessionsRevoked`. `SecurityEvent::AdminMutation` with these kinds renders as `AuditEventType::SessionRevoked` and `AuditEventType::AllSessionsRevoked` at `Notice` severity. These are distinct from the existing top-level `SecurityEvent::SessionRevoked` (renders as `TokenRevocation`, `Info`, emitted by `POST /revoke`) and `SecurityEvent::SessionsRevoked` (renders as `AllSessionsRevoked`, `Notice`, currently emitted by no flow); the `AuditEventType` variants themselves already exist.

### `07-telemetry-and-audit.md` → Audit (Modify)

> Append to the first paragraph: the internal session-revocation routes emit on the mandatory channel with the operator principal stamped. `DELETE /internal/users/{id}/sessions/{sessionId}` emits `SessionRevoked` for the subject user id with `detail.session_id`; `DELETE /internal/users/{id}/sessions` emits `AllSessionsRevoked` with `detail.sessions_revoked`. Neither `detail` holds a token, hash, or family id.

### `08-persistence.md` → DynamoDB session roster (Modify)

> In the paragraph ending "delete exactly the hashes it names, and return the count": for `revoke_family` the count is records removed (live plus retired); for `revoke_all_user_sessions` the count is the number of families in the roster that had a live generation, not the number of hashes deleted.

---

## Type changes

```json
{
  "$comment": "Fragment for 2026-11-06-revoke_user_sessions_via_internal_api. Folds into .specs/service/specs/canonical-types.schema.json on merge.",
  "$defs": {
    "SessionsRevokedResponse": {
      "type": "object",
      "additionalProperties": false,
      "required": ["sessions_revoked"],
      "properties": {
        "sessions_revoked": { "type": "integer", "minimum": 0 }
      }
    }
  }
}
```

---

## Implementation notes

1. `crates/core/src/ports/repository.rs:124`: change `revoke_all_user_sessions` to `Result<u64>`. Update every implementer (11): adapters `sqlite/mod.rs:1048`, `postgres/mod.rs:1011`, `dynamo/mod.rs:2069`, `lmdb/mod.rs:839`, `valkey/mod.rs:1101` (all under `crates/adapters/src/`); test doubles `crates/test-utils/src/lib.rs:596`, `crates/core/src/service/mod.rs:818`, `crates/server/src/reaper.rs:485`, `crates/core/tests/refresh.rs:996` and `:1373`, `crates/core/tests/assertion.rs:873`. The call at `crates/core/src/service/user_admin.rs:175` ignores the value and needs no change. Count families with a live generation, not rows or hashes (the DynamoDB roster maps family to hashes).
2. Extend `crates/test-utils/src/session_contract.rs` with the count cases and run them from every adapter.
3. `crates/core/src/domain/audit.rs`: add the two `AdminMutationKind` variants and their `event_type` / severity arms.
4. `crates/core/src/service/user_admin.rs`: add `admin_revoke_user_session` and `admin_revoke_user_sessions` beside `admin_list_user_sessions` (line 108). Share the user-load and soft-delete check with that method; build the audit event through `emit_admin_mutation_audit_event` (line 462) extended with an optional `detail` map, or as `emit_claims_audit_event` (line 407) already does for its `operation` detail. Also update the `revoke_all_user_sessions` entry in `schemas/dynamodb/table-design.json`.
5. `crates/server/src/routes/internal.rs`: two handlers and routes next to line 42. The bulk route and the list route share the `/users/{id}/sessions` path with different methods.
6. `schemas/internal-api.schema.json`: `revokeUserSession` (`204`, `404`, `401`, `429`) and `revokeUserSessions` (`200 SessionsRevokedResponse`, `404`, `401`, `429`). Add the rows to the README internal-routes table and `api-reference.md`, with the access-token-lifetime note, and add a `.changeset/` entry.
7. Tests: `crates/server/tests/internal.rs` for auth parity, `204`/`404` on the single route, ownership (another user's id is `404` and the session survives), idempotency on both routes, soft-deleted user `404`, rotated family revoked and a stale refresh token then rejected, a concurrent-revocation case (`revoke_family` returning `0` gives `404` and no audit event), and a check that audit `detail` holds no token, hash, or family id. Core tests in `crates/core/tests/user_admin.rs` for the audit events, including `n = 0`.
8. Order: port and contract suite, adapters, audit variants, service methods, routes, schema and docs.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**`.
2. Fold the `Type changes` `$defs` into `canonical-types.schema.json`; update `schemas/internal-api.schema.json` in the code change.
3. Flip `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, move this file to `.specs/changes/merged/`.
4. Update `.specs/README.md`: change the row's link and status.

---

## Assumptions and open questions

### Assumptions

- The host backend holds the internal credential and is the only caller; it is trusted to pass ids it received from the list for the same user.
- Operators accept that revocation does not invalidate access tokens already issued.

### Decisions

- *Ownership by resolution, not lookup by id.* **The single route resolves the id among the named user's live sessions.** Adapters cannot compute ids and no reverse index exists; resolving within the user's own sessions makes cross-user ids a `404` by construction and needs no new port method.
- *Single revocation addresses only listed sessions.* **Resolution uses the same 50-session window as the list.** Any id the list returned is revocable; a user with more than 50 live sessions can still clear the older ones with the bulk route.
- *Bulk count means live sessions.* **`revoke_all_user_sessions` returns families with a live generation removed.** This matches the "sessions" a host app counted and keeps the second call at `0`; retirement records are an implementation detail.
- *Port change over a pre-count.* **The port returns the count.** Listing first would be racy and capped at 50; the adapter knows what it removed.
- *Operator-attributed audit reuses existing event types.* **`SessionRevoked` and `AllSessionsRevoked` via new `AdminMutationKind` variants.** Keeps the closed event set, severity, and operator stamping of every other admin write; the `TokenRevocation` event stays for token-holder revocation.
- *Bulk audits a zero result; single audits only success.* **An operator's "sign out everywhere" is recorded even when nothing was live; a `404` mutates and records nothing.**
- *Bulk count includes unswept expired families.* **The count is families with a live generation removed, whether or not `expires_at` has passed.** Adapters do not filter on expiry when revoking; the count can therefore exceed what the list showed, but the repeat call is still `0`.
- *Admin mutation variants over the unused top-level events.* **New `AdminMutationKind` variants rather than `SecurityEvent::SessionsRevoked`.** The admin variants keep one operator-attributed path and the `detail` plumbing of every other admin write; the existing top-level `SessionRevoked` (maps to `TokenRevocation`) must stay as it is for `POST /revoke`.
- *The user stays active.* **No status change.** Suspension and deletion remain `PATCH` and `DELETE /internal/users/{id}`.

### Open questions

- Should the single route resolve beyond the 50-session list window by adding an unbounded port listing, if sign-ins per user can exceed 50 in practice?
