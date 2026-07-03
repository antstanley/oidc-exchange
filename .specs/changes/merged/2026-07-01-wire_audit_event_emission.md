# Change: Wire audit event emission into the service flows

**Status:** Merged · **Date:** 2026-07-01 · **Merged:** 2026-07-03 · **Owner:** Ant Stanley · **Target:** crates/core + crates/server + crates/adapters (audit)

Call `AppService::emit_audit` from every flow the spec names as audited, plumb the client
context the `AuditContext` middleware already extracts into sessions and audit events, gate
emission on a new `[audit] emit_threshold` severity floor, and harden the stdout and SQS
audit adapters so a backend failure reaches `emit_audit`'s fallback-and-threshold logic
instead of panicking or misfiring.

---

## Motivation

The canonical spec is ahead of the code here. [03-service-flows.md](../service/specs/03-service-flows.md)
names audited events throughout the exchange flow (`UserSuspended`, `RegistrationDenied`,
`UserCreated`) and [07-telemetry-and-audit.md](../service/specs/07-telemetry-and-audit.md)
describes the blocking-threshold semantics as implemented — but `AppService::emit_audit`
(`crates/core/src/service/mod.rs:102`) has zero call sites outside `crates/core/tests/audit.rs`.
No exchange, refresh, revocation, registration denial, or admin operation emits an
`AuditEvent`; the entire compliance pipeline is production-dead code.

The supporting plumbing is dead too. The `AuditContext` middleware extracts
`X-Forwarded-For`/`User-Agent`/`X-Device-Id` that nothing consumes; `exchange.rs` hardcodes
`device_id`/`user_agent`/`ip_address` to `None` in stored sessions, and `create_audit_event`
hardcodes the event's `ip_address`/`user_agent` to `None`. And the two real adapters undermine the design that spec 07 records:
`stdout_audit` uses `println!`/`eprintln!`, which panic on write errors (EPIPE from a restarted
log collector) instead of returning `Err` for the threshold logic to judge, and `sqs_audit`
detects FIFO queues with a substring `contains(".fifo")` where spec 07 says suffix.

---

## Affected spec pages

| Canonical page                                                                                 | Nature of change                                                                                                                                                                                                                                |
| ---------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md)               | Session section: `device_id`/`user_agent`/`ip_address` are populated from the request context, no longer `None` at issuance; removes the Open question about wiring the audit-context values into the stored session                            |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md)             | Page already describes the rejection-branch auditing and blocking algorithm as the target state; merge adds the success-path and admin event names, the client-context wording, and removes the `Unauthorized` vs `UserSuspended` Open question |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md)                       | Middleware stack: note the `AuditContext` extension is consumed by the `/token` and `/revoke` handlers                                                                                                                                          |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md)             | `[audit]` section: add the `emit_threshold` key — the severity floor for emitting events at all, separate from `blocking_threshold` — and add its `info` default to the Defaults summary table                                                 |
| [`.specs/service/specs/07-telemetry-and-audit.md`](../service/specs/07-telemetry-and-audit.md) | Audit section: merge adds the `emit_threshold` pre-dispatch filter, the adapter write-failure sentence, and the FIFO `message_group_id`/deduplication detail                                                                                    |

---

## Proposed changes

### `.specs/service/specs/01-domain-model.md` → Entities → Session (Modify)

> The raw refresh token exists only in memory during issuance and in the response to the
> client. Only the hash is stored. `device_id`, `user_agent`, and `ip_address` are populated
> from the request context: the audit-context middleware captures them at the HTTP edge and
> the exchange flow threads them into the stored session.

### `.specs/service/specs/01-domain-model.md` → Open questions (Remove)

Remove the resolved item:

> - `Session.device_id` / `user_agent` / `ip_address` exist on the entity and in the store
>   schemas but are written as `None` at issuance; wiring the audit-context values into the
>   stored session is unresolved.

### `.specs/service/specs/03-service-flows.md` → Token exchange (Modify)

> `ExchangeRequest` carries the client context (`ip_address`, `user_agent`, `device_id`)
> extracted by the server's audit-context middleware; the stored session records all three,
> and every audit event in the flow records the `ip_address` and `user_agent` (the
> `AuditEvent` shape carries no `device_id`). A suspended user audits `UserSuspended` (warning, failure); the
> registration-policy denials audit `RegistrationDenied` (warning, failure); a created user
> audits `UserCreated` (notice, success); a successful exchange audits `TokenExchange` (info,
> success) after the token response is assembled.

### `.specs/service/specs/03-service-flows.md` → Token refresh (Modify)

> `RefreshRequest` carries the same client context; audit events in the flow record its
> `ip_address` and `user_agent`. A suspended user audits `UserSuspended`; a
> successful refresh audits `TokenRefresh` (info, success). Unknown or expired tokens return
> `InvalidToken` and audit `ValidationFailed` (debug, failure) — an abuse-detection signal
> that the default `[audit] emit_threshold` of `info` suppresses; lowering the
> threshold to `debug` enables it.

### `.specs/service/specs/03-service-flows.md` → Revocation (Modify)

> `RevokeRequest` carries the same client context; audit events in the flow record its
> `ip_address` and `user_agent`. The access-token path audits
> `AllSessionsRevoked` when signature verification succeeds; the refresh-token path audits
> `TokenRevocation` when a session was actually revoked. Failed verification and unknown
> tokens emit nothing, matching RFC 7009's silence.

### `.specs/service/specs/03-service-flows.md` → Admin operations (Modify)

> Admin mutations are audited: `admin_create_user` → `UserCreated`, `admin_update_user` →
> `UserUpdated` (and `UserSuspended` when the patch sets `status = Suspended`),
> `admin_delete_user` → `UserDeleted`, and the claims mutations → `UserUpdated` with the
> operation in `detail`. Read-only operations (get, list, stats, get-claims) are not audited.
> Audit failures follow `emit_audit`'s blocking rules, unlike best-effort user sync.

### `.specs/service/specs/03-service-flows.md` → Open questions (Remove)

Remove the resolved item:

> - Suspended-user exchange is rejected, but whether an audit `Unauthorized` vs `UserSuspended`
>   event type is emitted in every rejection branch is worth confirming against the handlers.

### `.specs/service/specs/04-http-api.md` → Middleware stack (Modify)

> 2. **Audit context** (`middleware/audit_context.rs`) — extract `X-Forwarded-For`,
>    `User-Agent`, `X-Device-Id` into an `AuditContext` request extension, which the `/token`
>    and `/revoke` handlers pass into the core request structs so the stored session records
>    `ip_address`/`user_agent`/`device_id` and audit events record `ip_address`/`user_agent`.

### `.specs/service/specs/06-configuration.md` → Sections → `[audit]` (Modify)

> `adapter` (`noop` | `stdout` | `sqs`, default `noop`), `blocking_threshold` (syslog severity
> name, default `warning`), `emit_threshold` (syslog severity name, default `info`)
> — events with a severity strictly less severe than the threshold are not emitted at all,
> independently of the blocking decision — optional `[audit.sqs] { queue_url, region }`.

### `.specs/service/specs/06-configuration.md` → Defaults summary (Modify)

> | `audit.adapter` / `blocking_threshold` / `emit_threshold` | `noop` / `warning` / `info` |

### `.specs/service/specs/07-telemetry-and-audit.md` → Audit (Modify)

> Before dispatching to any adapter, `emit_audit` applies the `[audit] emit_threshold` filter:
> events strictly less severe than the configured threshold (default `info`) are
> dropped outright, so `ValidationFailed` (debug) stays silent unless the threshold is
> lowered. The `stdout_audit` adapter writes with locked handles; a write failure (e.g. EPIPE
> from a restarted log collector) returns `AuditError` and flows through `emit_audit`'s
> fallback-and-threshold path rather than panicking. On FIFO queues, `sqs_audit` sets
> `message_group_id` to the event id — each event is its own group, so FIFO ordering never
> serializes throughput — with the event's ULID as the deduplication id.

---

## Type changes

None. `Session` already carries `device_id`/`user_agent`/`ip_address` and `AuditEvent`
already carries `ip_address`/`user_agent` in the canonical schema — `AuditEvent` has no
`device_id` field and this change does not add one; the core request structs are not
canonical entities. No `AuditEventType` variant is added or removed — `SessionRevoked` stays
reserved.

---

## Implementation notes

1. Add `ip_address`/`user_agent`/`device_id: Option<String>` to `ExchangeRequest`
   (`crates/core/src/service/exchange.rs:10-15`), `RefreshRequest`
   (`crates/core/src/service/refresh.rs:8-10`), and `RevokeRequest`
   (`crates/core/src/service/revoke.rs:8-11`).
2. Populate the session from them at `crates/core/src/service/exchange.rs:158-160`; extend
   `create_audit_event` (`crates/core/src/service/mod.rs:132-151`) to take the context instead
   of hardcoding `None` at `mod.rs:146-147`.
3. Emit at each named point: `exchange.rs:92` (`UserSuspended`), `:105`/`:110`/`:116`/`:126`
   (`RegistrationDenied`), `:137` (`UserCreated`), `:168` (`TokenExchange`); `refresh.rs:22`/
   `:28`/`:38` (`ValidationFailed`, debug — unknown/expired token, unknown user), `:43`
   (`UserSuspended`), `:50` (`TokenRefresh`); `revoke.rs:20` (`AllSessionsRevoked`), `:28`/`:34`
   (`TokenRevocation`); `crates/core/src/service/user_admin.rs:14`, `:33`, `:74`, `:120`,
   `:152`, `:173` (admin mutations).
4. Add `emit_threshold: String` (default `"info"`) to `AuditConfig`
   (`crates/core/src/config.rs:81`); at the top of `AppService::emit_audit`
   (`crates/core/src/service/mod.rs:102`), parse it with the existing `parse_severity` and
   return `Ok(())` before any adapter dispatch when the event's severity is strictly less
   severe than the threshold.
5. Consume the `Extension<AuditContext>` in `crates/server/src/routes/token.rs:23-55` and
   `crates/server/src/routes/revoke.rs:16-29`; the layer is already installed at
   `crates/server/src/bootstrap.rs:135`.
6. `crates/adapters/src/stdout_audit/mod.rs:47-51`: replace `println!`/`eprintln!` with
   `writeln!` to locked stdout/stderr handles, mapping io errors to `Error::AuditError`.
7. `crates/adapters/src/sqs_audit/mod.rs:60`: `ends_with(".fifo")` instead of
   `contains(".fifo")`, matching spec 07 and the `test_fifo_detection` intent. At `mod.rs:70`,
   set `message_group_id` to `&event.id` instead of the serialized event type; the
   deduplication id at `mod.rs:69` is already the event's ULID.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**`.
2. Apply the two Open-question `Remove` blocks: 01-domain-model's session client-context
   wiring item and 03-service-flows' `Unauthorized` vs `UserSuspended` item (both resolved by
   the Decisions below).
3. No schema change.
4. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
5. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- `X-Forwarded-For` is set by a trusted proxy (already assumed in 04-http-api); the service
  records it verbatim without validating the trust chain.
- The committed default audit adapter stays `noop`, so wiring emission changes nothing for
  deployments that have not opted into stdout or SQS auditing.

### Decisions

- _Device id stays on the session._ **Audit events record `ip_address`/`user_agent` only;
  `device_id` is stored on the session.** The canonical `AuditEvent` shape has no `device_id`
  field and this change wires emission, it does not reshape events.
- _Suspended exchange audits `UserSuspended`, not `Unauthorized`._ **Every suspension
  rejection — exchange and refresh — emits the `UserSuspended` event type.** Resolves the
  03-service-flows Open question with one unambiguous event name.
- _Adapter failures are `Err`, never panics._ **Audit adapters return `AuditError` on write or
  send failure.** `emit_audit`'s fallback-then-threshold design is only meaningful if failures
  reach it.
- _Reads are not audited._ **Only mutations emit events.** Keeps volume proportional to
  state change.
- _FIFO groups are per event._ **`sqs_audit` sets `message_group_id` to the event id.** Each
  event forms its own FIFO group, so ordering guarantees never serialize throughput — the
  ULID deduplication id already carries the exactly-once semantics.
- _Failed refreshes audit `ValidationFailed`, behind an emit threshold._ **Unknown/expired
  refresh attempts emit `ValidationFailed` at `debug`, and a new `[audit] emit_threshold`
  (default `info`) filters emission.** The abuse-detection signal is one config knob
  away without making the default pipeline noisy.
- _`SessionRevoked` stays reserved._ **The unused `AuditEventType` variant is kept.**
  Single-session revocation may yet need it, and dropping it churns the schema for no gain.

### Open questions

- (None at this stage.)
