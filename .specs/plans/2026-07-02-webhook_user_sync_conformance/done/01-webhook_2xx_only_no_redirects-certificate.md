# Done Certificate — Task 01: Webhook 2xx-only success and no redirects

**Task:** [01-webhook_2xx_only_no_redirects.md](01-webhook_2xx_only_no_redirects.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The webhook adapter treats only a 2xx as delivery success, a 3xx as a non-retried rejection, and follows no redirects, so the HMAC-signed body is never re-POSTed to an unconfigured host.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing 2xx-success, 5xx-retry, 4xx-no-retry, and timeout/connect-retry behaviour of `send_webhook` (`webhook/mod.rs:64-97`); only the redirect handling and the success predicate change.

## Obligations

- **O1 — A 2xx is the only delivery success; a 3xx is a non-retried failure.**
  - *Claim:* the success test in `send_webhook` is `status.is_success()` alone; a 3xx response returns a non-retried `Error::SyncError`.
  - *Evidence to collect:* read `crates/adapters/src/webhook/mod.rs` around line 66 — confirm the disjunct `|| status.is_redirection()` is gone. Run a `wiremock` test mounting a single `302` handler; assert `notify_user_created` returns `Err` and the server received exactly one request (no retry).
  - *Checks:* trace a 3xx status through the match arms — confirm it is neither `is_success()` nor `is_server_error()`, so it falls to the rejection `return Err(...)` branch, not the 5xx `continue`.
  - *Status:* ☑ SATISFIED — `mod.rs:67` reads `if status.is_success() {` alone; the `|| status.is_redirection()` disjunct is gone (grep for `is_redirection` in the file returns no match). Trace of a 302: `is_success()` (200–299) → false, `is_server_error()` (500–599) → false, so control reaches the `return Err(Error::SyncError { detail: "webhook rejected: HTTP {status}" })` branch at `mod.rs:75-77` — a non-retried rejection, not the 5xx `continue`. Test `test_redirect_is_not_followed_and_is_not_retried` mounts a single 302 handler, asserts `notify_user_created` returns `Err` and the server received exactly 1 request → PASS.

- **O2 — The client follows no redirects (signed body never re-sent).**
  - *Claim:* the `reqwest::Client` is built with `redirect::Policy::none()`; a 302 whose `Location` points at a different host contacts only the configured host.
  - *Evidence to collect:* read `WebhookUserSync::new` (`webhook/mod.rs:19-24`) — confirm `.redirect(reqwest::redirect::Policy::none())` on the builder. Run the redirect test whose `302` `Location` targets a second, unmounted host; assert only the mounted host received a request and the redirect target host was never contacted.
  - *Checks:* confirm the `Policy::none()` is on the same builder used by `send_webhook` (the struct's `client` field), not a different client.
  - *Status:* ☑ SATISFIED — `WebhookUserSync::new` (`mod.rs:20-24`) builds the client with `.redirect(reqwest::redirect::Policy::none())` (line 22) and assigns it to the struct's single `client` field (`mod.rs:15,30`); `send_webhook` posts via that same `self.client` (`mod.rs:57`) — there is no second client. Test uses a second, unmounted `MockServer` as the redirect target and asserts `redirect_target.received_requests().len() == 0` → PASS: the redirect host is never contacted, so the signed body is never re-sent.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, the touched functions keep ≥2 meaningful assertions.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-adapters` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` produced no diff (clean); `cargo clippy --workspace -- -D warnings` finished with no warnings; `cargo nextest run -p oidc-exchange-adapters webhook` → 4/4 passed; full `cargo nextest run --workspace` → 338/338 passed. Touched-function assertions: the new test `test_redirect_is_not_followed_and_is_not_retried` carries 3 meaningful assertions (`result.is_err()`, configured host `== 1` request, redirect host `== 0` requests); `send_webhook`/`new` remain covered by the pre-existing 2xx/5xx/4xx tests.

- **O4 — Reviewable: a 302 is a single-attempt failure with no second host, and `Policy::none()` is set.**
  - *Claim:* a reviewer can confirm a 302 is a non-retried failure that contacts no redirect target, and that redirects are disabled on the client.
  - *Evidence to collect:* run the new redirect test and observe one request to the configured host, an `Err` result, and no request to the redirect target; grep `webhook/mod.rs` for `Policy::none` and confirm the success predicate no longer references `is_redirection`.
  - *Status:* ☑ SATISFIED — ran `test_redirect_is_not_followed_and_is_not_retried` in isolation → PASS: `Err` result, exactly 1 request to the configured host (no retry), 0 requests to the redirect target. `grep` of `webhook/mod.rs`: `Policy::none()` at line 22, `is_redirection` absent, success predicate at line 67 is `status.is_success()` alone. A reviewer re-running the test observes a single-attempt failure with no second host contacted.

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `WebhookUserSync::notify_user_created`/`notify_user_updated`/`notify_user_deleted` still succeed on a 200 and still retry on 500 → expect the existing `test_successful_delivery_with_correct_hmac`, `test_retry_on_5xx`, and `test_4xx_no_retry` tests still pass : ☑ PRESERVED — all three pass (200 → `is_success()` → `Ok`; 500 → `is_server_error()` → `continue`/retry, 3 requests; 400 → rejection, 1 request). Only the success predicate and the redirect policy changed; the 2xx-success, 5xx-retry, 4xx-no-retry, and timeout/connect-retry arms are untouched.

## Residue

- The backoff-cap change and the config `retries` clamp are out of scope here — they land in tasks 02 and 03. Not obligations of Task 01.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O4 all SATISFIED with evidence — the success predicate is `status.is_success()` alone (a 302 falls to the non-retried rejection branch) and the client is built with `redirect::Policy::none()`; the new wiremock test confirms a 302 is a single-attempt failure that never contacts the redirect target; fmt/clippy clean and 338/338 workspace tests pass; the 2xx/5xx/4xx regression callers are PRESERVED.
