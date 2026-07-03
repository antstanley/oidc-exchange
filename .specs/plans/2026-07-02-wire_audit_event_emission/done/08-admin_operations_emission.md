# Task 08 — admin operations emission

**Plan:** [plan.md](../plan.md) · **Certificate:** [08-admin_operations_emission-certificate.md](08-admin_operations_emission-certificate.md)

**Implements:** [03-service-flows.md](../../../service/specs/03-service-flows.md) §Admin operations (audit the admin mutations; leave reads unaudited)
**Depends on:** 01, 03
**Produces:** admin mutations emit audit events — `admin_create_user` → `UserCreated`, `admin_update_user` → `UserUpdated` (and `UserSuspended` when the patch sets `status = Suspended`), `admin_delete_user` → `UserDeleted`, and the claims mutations → `UserUpdated` with the operation in `detail`; reads emit nothing.
**Pointers:** `crates/core/src/service/user_admin.rs:14` (`admin_create_user`), `:33` (`admin_update_user`), `:74` (`admin_delete_user`), `:120` (`admin_set_claims`), `:152` (`admin_merge_claims`), `:173` (`admin_clear_claims`); `crates/core/src/service/mod.rs:102`/`:132`

## Steps

- [x] Emit `UserCreated` after `admin_create_user` succeeds; `UserDeleted` after `admin_delete_user` completes.
- [x] Emit `UserUpdated` after `admin_update_user`; when the applied patch sets `status = Suspended`, emit `UserSuspended` instead of (or in addition to, per spec wording) `UserUpdated`.
- [x] Emit `UserUpdated` after each claims mutation (`admin_set_claims`, `admin_merge_claims`, `admin_clear_claims`), recording the operation name in the event `detail`.
- [x] Leave the read-only operations (`admin_get_user`, `admin_list_users`, `admin_stats`, `admin_get_claims`) unaudited; build events via `create_audit_event` with `None` ip/ua (admin carries no client context) and apply `emit_audit`'s blocking rules (unlike the best-effort user-sync notify).
- [x] Add tests via `MockAuditLog`: each mutation records its event; a `status = Suspended` update records `UserSuspended`; each read records nothing.

## Definition of done

- [x] Each admin mutation emits its named event (with `UserSuspended` on a suspend patch and the operation in `detail` for claims mutations); read-only operations emit nothing.
- [x] Admin audit failures follow `emit_audit`'s blocking rules (a blocking-threshold failure propagates as `Err`), distinct from the best-effort user-sync notify that only logs a warning.
- [x] Negative-space test: `admin_get_user`/`admin_list_users`/`admin_stats`/`admin_get_claims` each record zero events on `MockAuditLog`.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run `cargo nextest run -p oidc-exchange-core user_admin` and observe each mutation's event (and the suspend-path `UserSuspended`) recorded while reads record nothing.
