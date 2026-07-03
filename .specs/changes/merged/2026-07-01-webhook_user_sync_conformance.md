# Change: Webhook user-sync conformance (JIT notify, 2xx-only, bounded backoff)

**Status:** Merged · **Date:** 2026-07-01 · **Merged:** 2026-07-03 · **Owner:** Ant Stanley · **Target:** crates/core (exchange), crates/adapters (webhook)

Fire the best-effort `notify_user_created` (awaited, result ignored — not spawned) when the
exchange flow JIT-registers a user;
make the webhook adapter treat only 2xx as delivery success and stop following redirects
(no re-POSTing the HMAC-signed body cross-host); and bound the retry backoff so a
misconfigured `retries` cannot sleep for hours inside a request or overflow the shift.

---

## Motivation

[03-service-flows.md](../service/specs/03-service-flows.md) decides that "sync notifications
never fail an admin **or exchange** operation", implying the exchange flow sends them — but
JIT-registered users never trigger the user-sync webhook: `create_user` in
`crates/core/src/service/exchange.rs:137` has no notify, and only `user_admin.rs` calls the
`UserSync` port. Downstream systems miss exactly the users who self-register.

The adapter also diverges from the
[webhook contract](../service/specs/02-ports-and-adapters.md) ("Any 2xx is success; 5xx or
timeout retries…; 4xx is not retried"): `crates/adapters/src/webhook/mod.rs:66` counts 3xx
as success (`is_success() || is_redirection()`), while the default `reqwest` client follows
up to 10 redirects — on a 307/308 the signed body is re-POSTed to wherever the target
points, cross-host. And the backoff at `webhook/mod.rs:51` (`100 * (1 << (attempt - 1))`)
is uncapped with no bound on `retries` in config (`crates/core/src/config.rs:184`):
`retries = 20` yields a ~14-hour cumulative sleep inside an admin request, and
`retries >= 33` overflows the shift (a panic in debug builds).

---

## Affected spec pages

| Canonical page                                                                               | Nature of change                                                                                                           |
| -------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md)           | Exchange step 3 gains the JIT `notify_user_created`; the best-effort decision already covers exchange — spec ahead of code |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | Contract already says 2xx-only — spec ahead of code. Add redirect policy and backoff cap                                   |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md)           | `[user_sync.webhook].retries` validated against an upper bound                                                             |

---

## Proposed changes

### `.specs/service/specs/03-service-flows.md` → Token exchange (Modify)

> 3. … Otherwise (`mode == "open"`) → `create_user(NewUser{…})` (audited `UserCreated`),
>    then `notify_user_created` — awaited but best-effort like the admin flows: the call
>    completes before the response (a fast follow-up `user.updated` cannot overtake
>    `user.created`), its result is discarded, and a sync failure is logged and never fails
>    the exchange.

### `.specs/service/specs/02-ports-and-adapters.md` → Webhook adapter contract (Modify)

> `POST` `application/json`, body `{ "event": "user.created"|"user.updated"|"user.deleted",
"timestamp": <RFC3339>, "data": <User> }`, authenticated by `X-Signature-256` carrying the
> hex HMAC-SHA256 of the raw body under the configured secret. Only a 2xx response is
> success; the client does not follow redirects (a 3xx is a rejection — the signed body is
> never re-sent to a location the operator did not configure); 5xx or timeout retries up to
> `retries` with exponential backoff capped at 5s per attempt; 4xx is not retried.

### `.specs/service/specs/06-configuration.md` → `[user_sync]` (Modify)

> `enabled` (bool), `adapter` (`webhook`), `[user_sync.webhook] { url, secret, timeout?,
retries? }`. `retries` is clamped at config validation (maximum 10); the `secret` is
> redacted in `Debug`.

---

## Type changes

None. `WebhookConfig` keeps its shape; only its validation tightens.

---

## Implementation notes

1. `crates/core/src/service/exchange.rs:137` — after a successful JIT `create_user`, call
   `self.user_sync.notify_user_created(&user).await` and log-and-continue on error (mirror
   `admin_create_user`, `crates/core/src/service/user_admin.rs:16-18`). The call is awaited
   in the request, not spawned onto a task; the capped backoff (note 4) and the `retries`
   clamp (note 5) bound the worst-case `/token` latency this adds.
2. `crates/adapters/src/webhook/mod.rs:66` — drop `|| status.is_redirection()`; a 3xx falls
   through to the non-retried rejection branch.
3. `crates/adapters/src/webhook/mod.rs:20-23` — build the client with
   `.redirect(reqwest::redirect::Policy::none())` alongside the existing timeout.
4. `crates/adapters/src/webhook/mod.rs:51` — cap the delay:
   `100 * (1u64 << (attempt - 1).min(6))` clamped to 5 000 ms (removes both the unbounded
   sleep and the `1 << 32` overflow).
5. Enforce `retries <= 10` in config validation (`crates/core/src/config.rs`, alongside the
   existing `WebhookConfig` at `config.rs:179-185`), erroring or clamping with a warning at
   load time.
6. Tests (wiremock): JIT exchange fires exactly one `user.created` and still returns a token
   when the webhook 500s every attempt; a 302 response is a failure and no second host is
   contacted; delay cap unit-tested with `retries = 20`.

---

## Merge plan

1. Apply the 03, 02, and 06 blocks to their canonical pages; bump each page's `**Date:**`.
2. No schema change.
3. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- No deployed webhook target relies on redirect-following or a 3xx-as-ack contract; the
  operator points `url` at the final endpoint.
- The extra best-effort webhook call on first login is acceptable exchange-path latency:
  with the 5 s per-attempt cap and `retries <= 10`, the worst case adds roughly 26 s of
  cumulative backoff plus one webhook `timeout` per attempt to `/token`; failures never
  block token issuance.

### Decisions

- _No redirects, ever._ **The webhook client uses `Policy::none()`.** Re-signing semantics
  across hosts are undefined and forwarding an HMAC-signed body to an unconfigured host is a
  credential-adjacent leak; operators configure the final URL.
- _Cap in the adapter and in config._ **Per-attempt delay is capped at 5s and `retries` at 10.** Defence in depth: a bad config cannot turn a synchronous admin call into an
  hours-long hang even if one bound is bypassed.
- _JIT notify is awaited, not spawned._ **The exchange flow awaits `notify_user_created` and
  discards the result, exactly like the admin flows.** Spawning would drop the
  `user.created` → `user.updated` ordering guarantee; the latency cost is bounded by the
  capped backoff (≤ 5 s per attempt, ~26 s worst-case cumulative sleep at `retries = 10`).
- _Durable delivery is a future SQS adapter._ **Queue-backed delivery arrives as a separate
  `UserSync` SQS adapter behind the existing port, proposed in its own change spec.** The
  port boundary already isolates the swap, so the HTTP webhook adapter stays request-scoped
  in this change.

### Open questions

- (None at this stage.)
