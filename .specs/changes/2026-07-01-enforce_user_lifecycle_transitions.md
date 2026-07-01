# Change: Enforce user lifecycle transitions in admin update

**Status:** Proposed · **Date:** 2026-07-01 · **Owner:** Ant Stanley · **Target:** crates/core (user_admin), crates/server error mapping

Make `admin_update_user` enforce the user-status state machine from
[01-domain-model.md](../service/specs/01-domain-model.md): `Deleted` becomes terminal, a
status patch to `Suspended` or `Deleted` revokes all the user's sessions (the delete case
matching `admin_delete_user`), and an update, delete, or claims operation on an unknown
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
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md)   | Lifecycle prose: entering `Suspended` now revokes sessions (supersedes the current wording under which reactivation resumes existing refresh sessions); `Deleted` bullet already ahead of code; rejection of off-diagram transitions made explicit |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | `admin_update_user` row gains transition validation, revoke-on-suspend/delete, and not-found behaviour; claims rows move from `InvalidRequest` to `NotFound` for unknown ids                                                                       |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md)           | Error mapping table gains `NotFound` → 404 `not_found`                                                                                                                                                                                             |

---

## Proposed changes

### `.specs/service/specs/01-domain-model.md` → Lifecycles → User status (Modify)

> - **Suspended** — exchange and refresh are rejected (`UserSuspended`); entering `Suspended`
>   revokes all the user's sessions, so reactivation does not restore existing refresh
>   tokens — the user signs in again. Already-issued access JWTs remain valid until they
>   expire (JWTs are not individually revocable).
> - **Deleted** — soft delete via `UserPatch { status: Deleted }`; the service revokes all the
>   user's sessions on delete. The row is kept. `Deleted` is terminal: a status patch out of
>   `Deleted` (or any transition not drawn above) is rejected with `InvalidRequest`.

### `.specs/service/specs/03-service-flows.md` → Admin operations (Modify)

> | Method              | Behaviour                                                                                                                                                                                                                                                                                                                    |
> | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
> | `admin_update_user` | load the user (missing → `NotFound`), validate any status change against the [user lifecycle](01-domain-model.md) (`Deleted` is terminal; invalid transition → `InvalidRequest`), `update_user`, revoke all sessions when the patch set status to `Suspended` or `Deleted`, diff changed fields, `notify_user_updated`          |
> | `admin_delete_user` | patch `status=Deleted` via the same validated path, `revoke_all_user_sessions`, `notify_user_deleted`; unknown id → `NotFound`                                                                                                                                                                                                  |
> | `admin_get_claims`  | return `user.claims` (missing user → `NotFound`; `admin_set_claims` / `admin_merge_claims` / `admin_clear_claims` return the same on an unknown id)                                                                                                                                                                             |

### `.specs/service/specs/04-http-api.md` → Error mapping (Modify)

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
   `update_user`; on `Some(UserStatus::Suspended)` or `Some(UserStatus::Deleted)` call
   `session_repo.revoke_all_user_sessions` after the update (same ordering as
   `admin_delete_user`, `user_admin.rs:66-82`).
2. Put the transition predicate next to `UserStatus` in `crates/core/src/domain/user.rs`
   (e.g. `UserStatus::can_transition_to`) so the rules live with the type, not the service.
3. `admin_delete_user` routes its patch through the same validated path, so deleting an
   already-deleted user becomes `InvalidRequest` rather than a silent second delete.
4. Add `Error::NotFound { detail }` to `crates/core/src/error.rs` (variants at
   `error.rs:4-50`) and map it to 404 / `not_found` in `map_domain_error`
   (`crates/server/src/error.rs:51-108`), plus any FFI error tables. Switch the claims
   pre-checks (`user_admin.rs:87-97`, `:100-111`, `:127-138`, `:157-164`) from
   `InvalidRequest` to `NotFound` so every unknown-id admin operation agrees with GET. The
   adapters' `StoreError { "user not found" }` branches become unreachable from the admin
   path but stay as a backstop.
5. Tests: patch to `Suspended` or `Deleted` revokes sessions; a reactivated user has no
   surviving refresh sessions; patch `Deleted → Active` rejected; unknown id on
   PATCH/DELETE/claims returns 404 `not_found`, not 500 (extend the server E2E internal-API
   tests).

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
- _Unknown id is 404._ **PATCH/DELETE/claims operations on an unknown user id return a new
  `Error::NotFound` mapped to HTTP 404 `not_found`.** Matches what GET already returns and
  keeps 400 for malformed or rule-violating requests.

### Open questions

- (None at this stage.)
