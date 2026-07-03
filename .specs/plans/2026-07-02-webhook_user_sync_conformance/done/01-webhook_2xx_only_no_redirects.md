# Task 01 — Webhook 2xx-only success and no redirects

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-webhook_2xx_only_no_redirects-certificate.md](01-webhook_2xx_only_no_redirects-certificate.md)

**Implements:** [`02-ports-and-adapters.md` §Webhook adapter contract](../../../service/specs/02-ports-and-adapters.md) — "Only a 2xx response is success; the client does not follow redirects (a 3xx is a rejection — the signed body is never re-sent to a location the operator did not configure)"
**Depends on:** —
**Produces:** the webhook adapter treats only a 2xx as delivery success; a 3xx falls through to the non-retried rejection branch; and the `reqwest::Client` is built with redirects disabled, so on a 307/308 the HMAC-signed body is never re-POSTed to an unconfigured host
**Pointers:** `crates/adapters/src/webhook/mod.rs:66` (`status.is_success() || status.is_redirection()` — drop the redirection disjunct); `crates/adapters/src/webhook/mod.rs:20-23` (`Client::builder().timeout(timeout)` — add `.redirect(reqwest::redirect::Policy::none())`); success/rejection branch structure at `mod.rs:64-77`

## Steps

- [x] In `WebhookUserSync::new`, add `.redirect(reqwest::redirect::Policy::none())` to the `reqwest::Client::builder()` chain alongside the existing `.timeout(timeout)`.
- [x] In `send_webhook`, change the success test from `status.is_success() || status.is_redirection()` to `status.is_success()` so a 3xx no longer counts as success.
- [x] Confirm a 3xx now falls through to the existing non-`is_server_error` branch and returns a non-retried `Error::SyncError` (a 3xx is neither 2xx nor 5xx), matching the 4xx rejection path.
- [x] Add a `wiremock` test: mount a single handler returning `302` (with a `Location` header pointing at a second, unmounted host) and assert `notify_user_created` returns `Err`, that the mounted server received exactly one request, and that the redirect target host was never contacted.

## Definition of done

- [x] A 2xx is the only delivery success; a `wiremock` 302 (or 301/307/308) response makes `notify_user_created` return `Err` and is not retried.
- [x] The client follows no redirects: with a 302 whose `Location` points at a different host, only the configured host receives a request and the signed body is never re-sent (negative-space test).
- [x] Meets the repo definition of done (tests, lint/format, ≥2 assertions per touched function — see plan.md baseline).
- [x] Reviewable: a reviewer runs the new redirect test and confirms a 302 is a single-attempt failure with no second host contacted, and that `Policy::none()` is set on the client builder.
