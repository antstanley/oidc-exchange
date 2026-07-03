# Change: Enforce user lifecycle transitions in admin update

**Status:** Merged · **Date:** 2026-07-01 · **Merged:** 2026-07-03 · **Owner:** Ant Stanley · **Target:** crates/core (user_admin), crates/server error mapping

Make `admin_update_user` enforce the user-status state machine from
[01-domain-model.md](../service/specs/01-domain-model.md): `Deleted` becomes terminal
(reachable from any non-`Deleted` status), a status patch that changes the user's status to
`Suspended` or `Deleted` revokes all the user's sessions (the delete case matching
`admin_delete_user`), and an update, delete, or claims operation on an unknown
user id returns a new `NotFound` domain error (HTTP 404 `not_found`) instead of a
`StoreError` 500.

---

## Motivation

`admin_update_user` (`crates/core/src/service/user_admin.rs:32-61`) passes any
`UserPatch.status` straight to the repository with no transition rules. The canonical
lifecycle gives `Deleted` no outgoing edges and says "the service revokes all the user's
sessions on delete", but today (a) patching `status: Deleted` skips the session revocation
that `admin_delete_user` (`user_admin.rs:66-82`) performs, leaving refresh-token sessions
live for a "deleted" user; and (b) a later patch back to `Active` resurrects the user — and
every old refresh token, including stolen ones the delete was meant to kill, works again.
The spec is ahead of the code here; this change closes the gap.

Separately, `update_user` on an unknown id maps to `StoreError` in all three repository
adapters (`crates/adapters/src/dynamo/mod.rs:121-123`, `postgres/mod.rs:228-230`,
`sqlite/mod.rs:258-260`), which `crates/server/src/error.rs:98-106` renders as a generic
500 — unlike `admin_get_user` (404) and the claims operations, which pre-check and return
`InvalidRequest`. A caller typo should be a 4xx, not an internal error.

---

## Affected spec pages

| Canonical page                                                                     | Nature of change                                                                                                                                                           |
| ---------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md)   | Lifecycle diagram and prose: new `Suspended → Deleted` edge (delete is valid from any non-`Deleted` status); entering `Suspended` now revokes sessions (supersedes the previously *implied* behaviour — today nothing revokes sessions on suspend, so reactivation resumed existing refresh sessions); `Deleted` bullet already ahead of code; same-status patches defined as accepted no-ops except on `Deleted`; rejection of off-diagram transitions made explicit; the `Lifecycles → Session` removal-causes sentence and the *Suspended keeps live tokens valid* Decision updated to match revoke-on-suspend |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | `admin_update_user` row gains transition validation, revoke-on-suspend/delete, and not-found behaviour; claims rows move from `InvalidRequest` to `NotFound` for unknown ids                                                                       |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md)           | Routes table: PATCH/DELETE/claims rows gain the "(404 if absent)" annotation the GET row already carries; Error mapping preamble widened beyond RFC 6749 §5.2; mapping table gains `NotFound` → 404 `not_found`                                    |

---

## Proposed changes

### `.specs/service/specs/01-domain-model.md` → Lifecycles → User status (Modify)

The state diagram gains a `Suspended → Deleted` edge — "admin delete (soft)" is reachable
from both `Active` and `Suspended`, so `admin_delete_user` on a suspended user is valid:

> ```
>         create_user
>             │
>             ▼
>         ┌────────┐  admin suspend   ┌───────────┐
>         │ Active │ ───────────────► │ Suspended │
>         └───┬────┘ ◄─────────────── └─────┬─────┘
>             │        admin reactivate     │
>             │ admin delete (soft)         │ admin delete (soft)
>             ▼                             │
>         ┌─────────┐                       │
>         │ Deleted │ ◄─────────────────────┘
>         └─────────┘  record retained, all sessions revoked
> ```
>
> - **Suspended** — exchange and refresh are rejected (`UserSuspended`); entering `Suspended`
>   revokes all the user's sessions, so reactivation does not restore existing refresh
>   tokens — the user signs in again. Already-issued access JWTs remain valid until they
>   expire (JWTs are not individually revocable).
> - **Deleted** — soft delete via `UserPatch { status: Deleted }` or `admin_delete_user`,
>   permitted from any non-`Deleted` status (`Active` or `Suspended`); the service revokes
>   all the user's sessions on delete. The row is kept. `Deleted` is strictly terminal: any
>   status patch on a deleted user — to any status, including `Deleted` itself — and any
>   other transition not drawn above are rejected with `InvalidRequest`.
>
> A status patch equal to the user's current status is not a transition: it is accepted as a
> no-op with no side effects — in particular, a `Suspended → Suspended` patch does not
> re-trigger session revocation. The one exception is `Deleted`, which admits no status
> patch at all (see above): `Deleted → Deleted` is rejected, not a no-op.

### `.specs/service/specs/01-domain-model.md` → Lifecycles → Session (Modify)

The removal-causes sentence gains the new revoke-on-suspend path (currently: "Removed by
explicit revocation (`/revoke`, delete-user, or revoke-all-user-sessions) or by expiry"):

> Created on token exchange with `expires_at = now + refresh_token_ttl`. Removed by explicit
> revocation (`/revoke`, revoke-all-user-sessions, or a status change to `Suspended` or
> `Deleted`) or by expiry — DynamoDB via its TTL attribute, other stores via
> `cleanup_expired_sessions`.

### `.specs/service/specs/01-domain-model.md` → Decisions (Modify)

The *Suspended keeps live tokens valid* bullet is retitled so it cannot be read as keeping
stored sessions alive (its bolded claim about access JWTs stays true):

> - _Suspension keeps outstanding access JWTs valid._ **Suspension revokes the user's stored
>   sessions but not outstanding access JWTs.** Access JWTs are stateless and short-lived;
>   revoking them would require introspection, which is out of scope.

### `.specs/service/specs/03-service-flows.md` → Admin operations (Modify)

> | Method              | Behaviour                                                                                                                                                                                                                                                                                                                    |
> | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
> | `admin_update_user` | load the user (missing → `NotFound`), validate any status change against the [user lifecycle](01-domain-model.md) (`Deleted` is strictly terminal — any status patch on a deleted user, including `Deleted → Deleted`, is rejected; a patch to the current status is otherwise an accepted no-op; invalid transition → `InvalidRequest`), `update_user`, revoke all sessions when the patch changed the status to `Suspended` or `Deleted`, diff changed fields, `notify_user_updated` |
> | `admin_delete_user` | patch `status=Deleted` via the same validated path (valid from `Active` or `Suspended`), `revoke_all_user_sessions`, `notify_user_deleted`; unknown id → `NotFound`                                                                                                                                                                                                  |
> | `admin_get_claims`  | return `user.claims` (missing user → `NotFound`; `admin_set_claims` / `admin_merge_claims` / `admin_clear_claims` return the same on an unknown id)                                                                                                                                                                             |

### `.specs/service/specs/04-http-api.md` → Routes → Internal (Modify)

The unknown-id rows gain the "(404 if absent)" annotation the GET row already carries. Their
Purpose cells currently read "update user (`UserPatch`)", "soft-delete user", "read claims",
"replace claims", "merge claims", and "clear claims"; they become:

> | PATCH | `/internal/users/{id}` | update user (`UserPatch`; 404 if absent) |
> | DELETE | `/internal/users/{id}` | soft-delete user (404 if absent) |
> | GET | `/internal/users/{id}/claims` | read claims (404 if absent) |
> | PUT | `/internal/users/{id}/claims` | replace claims (404 if absent) |
> | PATCH | `/internal/users/{id}/claims` | merge claims (404 if absent) |
> | DELETE | `/internal/users/{id}/claims` | clear claims (404 if absent) |

### `.specs/service/specs/04-http-api.md` → Error mapping (Modify)

The preamble no longer implies every code comes from RFC 6749 §5.2:

> `ApiError` wraps the domain `Error` (plus `UnsupportedGrantType`) and renders
> `{"error": <code>, "error_description": <detail>}` — an OAuth-style error envelope; codes
> beyond RFC 6749 §5.2 (`not_found`) use the same shape:

and the mapping table gains:

> | `NotFound` | 404 | `not_found` |

---

## Type changes

None. `UserStatus` and `UserPatch` keep their shapes; the transition rules are behaviour,
not data. `Error::NotFound` is a new error-enum variant in `crates/core/src/error.rs`, not
a canonical type.

---

## Implementation notes

1. `crates/core/src/service/user_admin.rs:32-61` — in `admin_update_user`, fetch the user
   first (mirror the `admin_get_claims` pre-check at `user_admin.rs:87-97`); missing →
   `NotFound`. Validate `(current, patch.status)` against the lifecycle before calling
   `update_user`; a patch equal to the current status passes validation as a no-op — the
   rest of the patch still applies, but it does not count as entering the status. Call
   `session_repo.revoke_all_user_sessions` after the update only when the status *changed*
   to `Suspended` or `Deleted` (same ordering as `admin_delete_user`, `user_admin.rs:66-82`);
   in particular `Suspended → Suspended` must not re-revoke.
2. Put the transition predicate next to `UserStatus` in `crates/core/src/domain/user.rs`
   (e.g. `UserStatus::can_transition_to`) so the rules live with the type, not the service.
   It allows `Deleted` from any non-`Deleted` status, allows same-status no-ops except on a
   `Deleted` user (`Deleted → Deleted` is rejected — `Deleted` admits no status patch), and
   rejects everything else off the diagram.
3. `admin_delete_user` routes its patch through the same validated path, so deleting an
   already-deleted user becomes `InvalidRequest` rather than a silent second delete, while
   deleting a suspended user stays valid (`Suspended → Deleted` is on the diagram).
4. Add `Error::NotFound { detail }` to `crates/core/src/error.rs` (variants at
   `error.rs:4-50`) and map it to 404 / `not_found` in `map_domain_error`
   (`crates/server/src/error.rs:51-108`), plus any FFI error tables. Switch the claims
   pre-checks (`user_admin.rs:87-97`, `:100-111`, `:127-138`, `:157-164`) from
   `InvalidRequest` to `NotFound` so every unknown-id admin operation agrees with GET. The
   adapters' `StoreError { "user not found" }` branches become unreachable from the admin
   path but stay as a backstop.
5. Tests: patch to `Suspended` or `Deleted` revokes sessions; a reactivated user has no
   surviving refresh sessions; suspend-then-delete succeeds and leaves the user `Deleted`;
   `Suspended → Suspended` patch is accepted and does not call `revoke_all_user_sessions`;
   patch `Deleted → Active` rejected; patch `Deleted → Deleted` and a second `DELETE` on the
   same user rejected with `InvalidRequest`; unknown id on PATCH/DELETE/claims returns 404
   `not_found`, not 500 (extend the server E2E internal-API tests).

---

## Merge plan

1. Apply the 01, 03, and 04 `Proposed changes` blocks to their canonical pages; bump each
   page's `**Date:**`.
2. No schema change.
3. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- The transition check happens in core, not the adapters; the repositories stay dumb writes.
  (The read-then-write race in `update_user` itself is addressed separately in
  [2026-07-01-fix_user_creation_race_and_dynamo_integrity.md](2026-07-01-fix_user_creation_race_and_dynamo_integrity.md).)

### Decisions

- _Deleted is terminal._ **No patch may leave `Deleted`.** Un-deleting silently rearms every
  historical refresh token; if restore is ever wanted it must be a deliberate operation that
  does not resurrect sessions.
- _Status-patch delete equals endpoint delete._ **`PATCH {status: Deleted}` and
  `DELETE /internal/users/{id}` behave identically**, including session revocation and the
  same terminal semantics.
- _Suspend revokes sessions._ **Any transition to `Suspended` revokes all the user's
  sessions, same as `Deleted`.** A suspension that leaves refresh tokens live is not a
  suspension; reactivation therefore does not restore existing sessions — the user signs in
  again.
- _Delete is valid from any non-`Deleted` status._ **The lifecycle gains a
  `Suspended → Deleted` edge; `admin_delete_user` (and `PATCH {status: Deleted}`) on a
  suspended user succeeds.** Without this edge, rejecting off-diagram transitions would make
  a suspended user undeletable — the R1 review flagged the collision; the diagram, not the
  rule, was wrong.
- _Same-status patch is a no-op, except on `Deleted`._ **A status patch equal to the user's
  current status is accepted with no side effects — in particular `Suspended → Suspended`
  does not re-trigger session revocation — except on a `Deleted` user, where any status
  patch (including `Deleted → Deleted`) is rejected with `InvalidRequest`.** Keeps status
  patches idempotent and ties revocation to the transition into a status, while keeping
  `Deleted` strictly terminal so `PATCH {status: Deleted}` and a second `DELETE` fail
  identically (per the equal-behaviour decision above).
- _Unknown id is 404._ **PATCH/DELETE/claims operations on an unknown user id return a new
  `Error::NotFound` mapped to HTTP 404 `not_found`.** Matches what GET already returns and
  keeps 400 for malformed or rule-violating requests.

### Open questions

- (None at this stage.)
