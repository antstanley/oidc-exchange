# Task 03 — enforce lifecycle in admin_update

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-enforce_lifecycle_in_admin_update-certificate.md](03-enforce_lifecycle_in_admin_update-certificate.md)

**Implements:** [03-service-flows.md](../../../service/specs/03-service-flows.md) §Admin operations (`admin_update_user`, `admin_delete_user` rows); [01-domain-model.md](../../../service/specs/01-domain-model.md) §Lifecycles → Session (revoke on suspend or delete) and §Decisions (suspension revokes stored sessions; status-patch delete equals endpoint delete)
**Depends on:** 01, 02
**Produces:** `admin_update_user` that fetches-first, validates the status transition, applies the patch, and revokes all sessions only when the status changed to `Suspended` or `Deleted`; `admin_delete_user` routed through the same validated path
**Pointers:** `crates/core/src/service/user_admin.rs:32-61` (`admin_update_user`), `:66-82` (`admin_delete_user`), `:87-97` (fetch-first pattern to mirror); `crates/core/src/domain/user.rs` (`UserStatus::can_transition_to`, from task 02); `crates/core/src/error.rs` (`Error::NotFound`, from task 01); `crates/core/tests/user_admin.rs` (existing test harness with `MockRepository`/`MockUserSync`)

## Steps

- [x] In `admin_update_user`, fetch the user via `get_user_by_id` before writing; on a miss return `Error::NotFound { detail }` (mirror the claims pre-check pattern).
- [x] When `patch.status` is `Some(target)`, reject with `Error::InvalidRequest` unless `current.can_transition_to(target)`; a same-status target passes as a no-op and the rest of the patch still applies.
- [x] Call `update_user`, then call `session_repo.revoke_all_user_sessions` only when `patch.status` changed the status to `Suspended` or `Deleted` (compare against the fetched current status so `Suspended → Suspended` does not re-revoke); preserve the existing `notify_user_updated` changed-fields diff and ordering.
- [x] Route `admin_delete_user` through the same validated path so a second delete (`Deleted → Deleted`) becomes `InvalidRequest` while deleting a suspended user stays valid; keep its `revoke_all_user_sessions` and `notify_user_deleted` behaviour and return `NotFound` on an unknown id.
- [x] Add core tests: patch to `Suspended`/`Deleted` revokes sessions; a reactivated user has no surviving refresh sessions; suspend-then-delete succeeds leaving the user `Deleted`; `Suspended → Suspended` does not call `revoke_all_user_sessions`; `Deleted → Active` and `Deleted → Deleted` and a second `DELETE` are rejected with `InvalidRequest`; unknown id on update/delete returns `NotFound`.

## Definition of done

- [x] A status patch (or `admin_delete_user`) entering `Suspended` or `Deleted` revokes all the user's sessions; a reactivated user has no surviving refresh sessions — asserted by core tests.
- [x] `Suspended → Suspended` is accepted as a no-op and does **not** call `revoke_all_user_sessions`; `Deleted → Active`, `Deleted → Deleted`, and a second `DELETE` on the same user are rejected with `InvalidRequest` (negative-space).
- [x] `admin_update_user` and `admin_delete_user` on an unknown id return `Error::NotFound`, and every touched function carries at least two meaningful assertions per the guidelines.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the `user_admin` core tests and confirms suspend-then-delete succeeds, `Suspended → Suspended` skips revocation, deleting an already-deleted user is `InvalidRequest`, and an unknown id is `NotFound`.

## Open questions

- None.
