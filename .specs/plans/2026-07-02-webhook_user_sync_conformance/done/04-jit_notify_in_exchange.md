# Task 04 — JIT `notify_user_created` in the exchange flow

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-jit_notify_in_exchange-certificate.md](04-jit_notify_in_exchange-certificate.md)

**Implements:** [`03-service-flows.md` §Token exchange](../../../service/specs/03-service-flows.md) — exchange step 3 "Otherwise (`mode == "open"`) → `create_user(NewUser{…})` … then `notify_user_created` — awaited but best-effort like the admin flows: the call completes before the response … its result is discarded, and a sync failure is logged and never fails the exchange"
**Depends on:** 01, 02, 03
**Produces:** when the exchange flow JIT-registers a user, it fires exactly one best-effort `notify_user_created` — awaited (not spawned) so a follow-up `user.updated` cannot overtake `user.created`, its result discarded, a failure logged via `tracing` and never failing token issuance
**Pointers:** `crates/core/src/service/exchange.rs:137` (`self.user_repo.create_user(&new_user).await?` in the `None` registration branch — add the notify immediately after); reference pattern `crates/core/src/service/user_admin.rs:16-18` (`admin_create_user`'s log-and-continue notify)

## Steps

- [x] After the successful JIT `create_user` at `exchange.rs:137` (inside the `None` branch only, so existing users are not re-notified), bind the created `user` and call `self.user_sync.notify_user_created(&user).await`, awaited before the flow continues.
- [x] On `Err`, log with `tracing::warn!` (error and `user.id`) and continue — mirror `admin_create_user` so a sync failure never fails the exchange; on `Ok`, discard the result.
- [x] Ensure the notify fires only for the JIT-registration path, not for the found-active-user branch that bypasses registration.
- [x] Add a core test (via `wiremock` behind the webhook adapter, or the test-utils `MockUserSync`) that a JIT exchange fires exactly one `user.created` and that the exchange still returns a `TokenResponse` when the sync fails every attempt (webhook 500s throughout).

## Definition of done

- [x] A JIT-registered user triggers exactly one `notify_user_created`, awaited before the token response is returned (ordering preserved); an existing active user triggers none.
- [x] Negative-space test: when the sync backend fails every attempt, the exchange still returns a token and the failure is logged, never propagated (best-effort).
- [x] Meets the repo definition of done (tests, lint/format, ≥2 assertions per touched function — see plan.md baseline).
- [x] Reviewable: a reviewer runs the JIT-notify test and confirms one `user.created` fires on first login and that a webhook that 500s every attempt still yields a token.
