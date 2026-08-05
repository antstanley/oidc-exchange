# Change: Audit and throttle authentication failures

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** crates/core, crates/server (service)

Make the record of a failed authentication unsuppressable and the attempt itself bounded.
Split audit emission into a mandatory `SecurityEvent` channel that no threshold may filter
and a best-effort channel that keeps `emit_threshold`, move terminal-outcome emission to a
single exit point per flow so no failure can leave through `?` unrecorded, make a
mandatory-channel write failure fail the operation, and add per-IP / per-subject /
per-provider rate limiting to the public routes keyed on a client address the server
observed rather than one the client asserted.

---

## Motivation

Under the shipped defaults this service keeps no record of anyone attacking it. A failed
refresh emits `ValidationFailed` at `Debug`(7) (`crates/core/src/service/refresh.rs:37`),
`emit_audit` compares it against an `emit_threshold` of `Info`(6) and returns `Ok(())`
before any adapter is consulted (`crates/core/src/service/mod.rs:106-110`) — and the
shipped adapter is `noop` anyway (`config/default.toml:12-15`). A failed ID-token
validation or an unknown provider never reaches emission code at all: both propagate
through `?` at `crates/core/src/service/exchange.rs:66-71`, `:76` and `:92-93`, and
`map_domain_error` writes a `tracing::error!` only for the `server_error` class
(`crates/server/src/error.rs:51-78`), so a 400 leaves no audit event *and* no log line.
There is no access log and no `TraceLayer`. Meanwhile nothing bounds attempt volume: the
router at `crates/server/src/bootstrap.rs:352-360` layers request-id, timeout,
audit-context and catch-panic, and `crates/server/src/middleware/` contains no limiter.
A credential-guessing campaign against `/token` is therefore both unlimited and invisible.

The third defect turns the audit trail into a best-effort one even when an operator has
built a real sink. When `audit.emit` fails, `crates/core/src/service/mod.rs:129` compares
the event's severity against `blocking_threshold` (`warning`, 4) and lets anything less
severe proceed. Five record types sit below that line — `UserCreated`(Notice) at
`exchange.rs:209`, `TokenExchange`(Info) at `exchange.rs:327`, `TokenRefresh`(Info) at
`refresh.rs:110`, `AllSessionsRevoked`(Notice) at `revoke.rs:54`, and
`TokenRevocation`(Info) at `revoke.rs:115` — so a sink outage lets the creation of a new
principal and the destruction of every session for a subject both return 200 with no
durable trail. This change spec reverses a Decision recorded in
[`changes/merged/2026-07-01-wire_audit_event_emission.md`](merged/2026-07-01-wire_audit_event_emission.md),
which put `ValidationFailed` at `debug` behind `emit_threshold` so "the abuse-detection
signal is one config knob away without making the default pipeline noisy". That Decision's
wording is now canonical: [`03-service-flows.md` → Token
refresh](../service/specs/03-service-flows.md) records `ValidationFailed` (debug) as
suppressed by the default `emit_threshold`, and
[`07-telemetry-and-audit.md`](../service/specs/07-telemetry-and-audit.md) documents the
pre-dispatch filter — the Modify blocks below replace exactly that text. The volume
concern was real; the conclusion was wrong. It treated an authentication failure as log
noise and gave the noise knob authority over the security record. The fix is not to raise a
number — it is to stop the noise knob from reaching the security record at all.

It follows Option 2 of
[`observability-contract.md`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/hardening/proposals/observability-contract.md)
(invariants OC3, OC4, OC5, OC8) and takes deliberately the rate-limiting decision that
proposal defers to Option 3 as "a product decision with an explicit written non-goal behind
it". The other half of Option 2 — `Secret<T>` / `TokenHash` / `SessionRef` newtypes that
implement no formatting trait — belongs to a separate change spec
([`2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md`](2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md)):
its findings are adapter-side span exposure in `crates/adapters`, outside this change's
target.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/00-overview.md`](../service/specs/00-overview.md) | Remove rate limiting from Non-goals; flip its Scope-summary row; revise the audit Goal for the mandatory security channel; port count in the Detail-pages and Crate-map tables |
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md) | Add `SecurityEvent` and `ClientAddr`; add `ip_address_source` to `AuditEvent`; add the `ThrottleExceeded` event type |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | Add the `RateLimiter` port (a seventh) and its in-process adapter row; update the six-trait intro |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | Terminal-outcome emission per flow; rewrite the audit emission/blocking section; admin-operations audit-failure wording; replace the "Audit fallback always records" Decision |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | Middleware stack gains client-address resolution and the throttle layer; 429 in error mapping; access log; `into_make_service_with_connect_info` in Bootstrap; revise the `X-Forwarded-For` Assumption |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | `audit.adapter` default `noop`→`stdout`; new `audit.durability`; new `[rate_limit]`; new `server.trusted_proxies` / `trusted_proxy_hops`; Validation-at-load additions; committed-default, Defaults-summary and Assumptions updates |
| [`.specs/service/specs/07-telemetry-and-audit.md`](../service/specs/07-telemetry-and-audit.md) | Mandatory vs best-effort channels; `emit_threshold` scoped to best-effort; overview-table failure-mode row; replace the "Noop audit by default" Decision |
| [`.specs/service/README.md`](../service/README.md) | Index row for 02: six ports → seven |
| [`canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | Add `ClientAddrSource`; add `throttle_exceeded` to `AuditEventType`; add `ip_address_source` to `AuditEvent` |

No new canonical page.

---

## Proposed changes

### `.specs/service/specs/01-domain-model.md` → Entities (Add)

> ### SecurityEvent (`domain/security_event.rs`)
>
> A closed enumeration of the outcomes the service treats as security-relevant. Severity is
> a property of the variant, not an argument at the call site: `SecurityEvent::severity()`
> and `SecurityEvent::event_type()` derive the `AuditSeverity` and `AuditEventType` that the
> resulting `AuditEvent` carries, so a new variant cannot be introduced with a severity that
> makes it invisible.
>
> | Variant | `AuditEventType` | Severity |
> |---|---|---|
> | `AuthenticationSucceeded { kind: Exchange }` | `TokenExchange` | `Info`(6) |
> | `AuthenticationSucceeded { kind: Refresh }` | `TokenRefresh` | `Info`(6) |
> | `AuthenticationFailed` | `ValidationFailed` | `Warning`(4) |
> | `RegistrationDenied` | `RegistrationDenied` | `Warning`(4) |
> | `PrincipalSuspended` | `UserSuspended` | `Warning`(4) |
> | `PrincipalCreated` | `UserCreated` | `Notice`(5) |
> | `SessionRevoked` | `TokenRevocation` | `Info`(6) |
> | `SessionsRevoked` | `AllSessionsRevoked` | `Notice`(5) |
> | `ProviderRejected` | `ProviderError` | `Warning`(4) |
> | `AdminMutation { .. }` | `UserCreated` / `UserUpdated` / `UserDeleted` | `Notice`(5) |
> | `ThrottleExceeded` | `ThrottleExceeded` | `Warning`(4) |
>
> `AuditEvent` remains the durable shape every adapter serializes; `SecurityEvent` is how
> the service names an outcome, and `create_audit_event` renders one into the other.
>
> ### ClientAddr (`domain/client_addr.rs`)
>
> ```rust
> enum ClientAddr {
>     Peer(IpAddr),                    // observed by the server or supplied by the platform
>     Forwarded(IpAddr),               // read from X-Forwarded-For behind a trusted proxy
>     Asserted { value: String },      // client-authored, length-bounded, untrusted
>     Unknown,
> }
> ```
>
> A consumer must acknowledge which kind it holds. `Peer` and `Forwarded` are admissible as
> a rate-limit key; `Asserted` and `Unknown` are not. All three render into
> `AuditEvent.ip_address`, and `AuditEvent.ip_address_source` records which one it was.

### `.specs/service/specs/01-domain-model.md` → Entities → AuditEvent (Modify)

> ```rust
> struct AuditEvent {
>     id: String,                       // ULID
>     timestamp: DateTime<Utc>,
>     severity: AuditSeverity,          // RFC 5424 syslog levels, emergency(0)..debug(7)
>     event_type: AuditEventType,
>     actor: Option<String>,            // user id if known
>     provider: Option<String>,
>     ip_address: Option<String>,
>     ip_address_source: ClientAddrSource, // peer | forwarded | asserted | unknown
>     user_agent: Option<String>,
>     detail: HashMap<String, Value>,
>     outcome: AuditOutcome,            // Success | Failure { reason }
> }
> ```
>
> `ip_address_source` states the provenance of `ip_address`: `peer` and `forwarded` were
> established by the server, `asserted` was copied from a client-controlled header behind no
> trusted proxy. An analyst reading the durable record can tell an observed address from a
> claimed one without knowing the deployment topology.
>
> `AuditEventType` variants: `TokenExchange`, `TokenRefresh`, `TokenRevocation`,
> `SessionRevoked`, `AllSessionsRevoked`, `UserCreated`, `UserUpdated`, `UserSuspended`,
> `UserDeleted`, `ValidationFailed`, `RegistrationDenied`, `ProviderError`, `Unauthorized`,
> `ThrottleExceeded`.
> `AuditOutcome` serializes to `{ "status": "success" }` or `{ "status": "failure", "reason": … }`.

### `.specs/service/specs/02-ports-and-adapters.md` → Introduction (Modify)

> The core declares seven port traits in `crates/core/src/ports/`. Adapters in
> `crates/adapters/`, `crates/providers/`, and — for the in-process rate limiter —
> `crates/server/` implement them. Every method returns the core's `Result<T>`; adapters
> convert native errors into the domain [`Error`](04-http-api.md) at the boundary.

### `.specs/service/specs/02-ports-and-adapters.md` → Port traits (Add)

> ### RateLimiter (`ports/rate_limit.rs`)
>
> ```rust
> async fn check_and_consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision>;
>
> enum RateLimitKey {
>     ClientAddr(IpAddr),
>     Subject { provider: Option<String>, subject_hash: String },
>     Provider(String),
> }
>
> enum RateLimitDecision {
>     Allow,
>     Deny { retry_after_secs: u64 },
> }
> ```
>
> One call consumes one unit against one key and reports whether the caller may proceed.
> `Subject.subject_hash` is a SHA-256 hex of the subject identifier, so limiter state — which
> may live in a shared store — never holds a raw user or provider subject. A limiter that
> cannot answer returns `Err`; callers log it and proceed (see Decisions).

### `.specs/service/specs/02-ports-and-adapters.md` → Adapter inventory (Add)

> | RateLimiter | In-process | `server/middleware/throttle` | fixed window per key, bounded map with expiry eviction; per-process, not global |
> | RateLimiter | Noop | `adapters/noop` | always `Allow`; selected when `rate_limit.enabled = false` |

### `.specs/service/specs/03-service-flows.md` → Token exchange (Modify)

> Emission is terminal and single: `exchange` wraps a fallible inner body and maps its
> outcome to exactly one `SecurityEvent` at the one exit point, so no branch can return
> through `?` unrecorded. `Ok` audits `AuthenticationSucceeded { kind: Exchange }`;
> `Err(UnknownProvider | InvalidRequest | InvalidGrant | InvalidToken)` audits
> `AuthenticationFailed`; `Err(AccessDenied)` audits `RegistrationDenied`;
> `Err(UserSuspended)` audits `PrincipalSuspended`; `Err(ProviderError | ProviderTimeout)`
> audits `ProviderRejected`. The event's `outcome` carries a **fixed classification string**
> — `unknown provider`, `provider token validation failed`, `malformed exchange request`,
> `upstream provider unavailable` — never the error's `Display`, because `ProviderError`'s
> `Display` embeds the upstream response body verbatim and would turn the audit sink into a
> second copy of that leak. `PrincipalCreated` is emitted at the creation site rather than at
> the exit, because it records a state change rather than the request's outcome; the losing
> racer in a JIT-registration conflict still emits none.
>
> The terminal success event is emitted **after** the session write and the access-token
> signature, and a mandatory-channel failure there revokes the session just stored before
> returning the error. Otherwise a failed exchange would leave a live refresh token that the
> caller never received and that no audit record describes.
>
> Two rate-limit checks sit inside the flow, in addition to the per-IP check the HTTP layer
> already applied: a per-provider unit is consumed before the outbound `exchange_code` or
> JWKS-backed validation, bounding the service's use as an amplifier against a provider; and
> a per-subject unit is consumed once `validate_id_token` has produced a subject, bounding
> replay of a valid ID token. A per-subject budget therefore bounds post-validation volume,
> not pre-validation guessing — that is what the per-IP and per-provider budgets bound.
> Exceeding either audits `ThrottleExceeded` and returns `TooManyRequests`.

### `.specs/service/specs/03-service-flows.md` → Token refresh (Modify)

> `refresh` uses the same single-exit emission. `Ok` audits
> `AuthenticationSucceeded { kind: Refresh }`; an unknown token, an expired session, and an
> unknown user all audit `AuthenticationFailed` at `Warning`, which the default
> configuration records — the emit threshold no longer reaches the security channel. A
> suspended user audits `PrincipalSuspended`. A per-subject unit is consumed once the
> session lookup resolves `session.user_id`; a refresh token that matches no session yields
> no subject, so pre-lookup guessing is bounded by the per-IP budget alone.

### `.specs/service/specs/03-service-flows.md` → Revocation (Modify)

> The access-token path audits `SessionsRevoked` when signature verification succeeds; the
> refresh-token path audits `SessionRevoked` when a session actually matched. A rejected
> token — failed verification, or a hash matching no session — audits `AuthenticationFailed`
> with a fixed classification `reason`. Every branch emits exactly one event and the response
> is `200` regardless: RFC 7009 §2.2 constrains what the *client* observes, while the record
> travels the mandatory channel to an operator sink.
>
> The symmetry is the control, not the silence. Under `durability = "enforce"` a
> mandatory-channel write failure fails the operation, so a flow that emitted only on success
> would answer `503` for a token that existed and `200` for one that did not — reconstructing,
> during a sink outage, exactly the existence oracle §2.2 forbids. Emitting on both branches
> keeps the two indistinguishable in every mode. Per-IP limiting still applies at the HTTP
> layer and is what bounds the volume of unauthenticated probing.

The `SessionsRevoked` wording above describes the access-token path as it exists at this
spec's merge point; [2026-08-05-validate_revoke_token_claims.md](2026-08-05-validate_revoke_token_claims.md)
merges later, narrows that path to the single session the token names, and its Revocation
rewrite supersedes this block's access-token sentence (the rejected-token branch and the
symmetry argument survive unchanged).

### `.specs/service/specs/03-service-flows.md` → Audit emission and blocking (Modify)

> ## Audit emission (`service/mod.rs`)
>
> Two channels with different guarantees.
>
> ```
> emit_security_event(SecurityEvent)          — mandatory
>    render to AuditEvent (severity derived from the variant)
>    audit.emit(event)
>       Ok  → done
>       Err → write the event to a tracing log with `audit_fallback = true`
>             durability = "enforce" → propagate Err (the operation fails)
>             durability = "observe" → tracing::error! `audit_durability_degraded = true`,
>                                      return Ok
>    No threshold is consulted. `emit_threshold` and `blocking_threshold` do not apply.
>
> emit_audit(AuditEvent)                      — best-effort
>    severity strictly less severe than `emit_threshold` → drop before dispatch
>    audit.emit(event)
>       Err → tracing fallback, then `blocking_threshold` decides as before
> ```
>
> Severity follows RFC 5424 (emergency 0 … debug 7); lower number is more severe. It is
> retained on the mandatory channel because sinks and SIEMs route on it — but it is no
> longer an emission gate there, so no configured threshold can suppress a security record.
> `blocking_threshold` governs only the best-effort channel; on the mandatory channel
> durability is unconditional and `audit.durability` chooses whether a failure is enforced
> or observed.
>
> Every event the shipped flows emit travels the mandatory channel. `emit_audit` remains
> public for operational events an embedder adds through `crates/ffi`, and `emit_threshold`
> governs those.

### `.specs/service/specs/03-service-flows.md` → Admin operations (Modify)

> Admin mutations are audited: `admin_create_user` → `UserCreated`, `admin_update_user` →
> `UserUpdated` (and `UserSuspended` when the patch sets `status = Suspended`),
> `admin_delete_user` → `UserDeleted`, and the claims mutations → `UserUpdated` with the
> operation in `detail`. Read-only operations (get, list, stats, get-claims) are not audited.
> Admin events travel the mandatory channel like every other shipped emission, so a write
> failure is governed by `audit.durability` — unlike best-effort user sync. Admin operations
> carry no client context; their events record `ip_address_source = "unknown"`.

### `.specs/service/specs/03-service-flows.md` → Decisions (Modify)

Replace *Audit fallback always records*:

> - *Audit durability is unconditional.* **A mandatory-channel write failure fails the
>   operation.** A tracing fallback line is still written first, but a record in a stream the
>   operator did not build is not the durable trail they built; letting the request succeed
>   anyway meant `/token` and `/revoke` completed with no evidence.

### `.specs/service/specs/04-http-api.md` → Middleware stack (Modify)

> 1. **Request ID** (`middleware/request_id.rs`) — unchanged.
> 2. **Client address** (`middleware/client_addr.rs`) — resolve a `ClientAddr` into a request
>    extension. Under hyper the connection peer from `ConnectInfo` is `Peer`; under Lambda
>    the event's request-context source IP (API Gateway v1 `identity.sourceIp`, v2
>    `http.sourceIp`) is `Peer`; under `crates/ffi` there is no peer and the result is
>    `Unknown`. When the peer falls inside `server.trusted_proxies`, the
>    `X-Forwarded-For` element `server.trusted_proxy_hops` from the **right** is parsed as an
>    `IpAddr` and becomes `Forwarded` — counted from the right because appending proxies put
>    the observed peer last, and taking the leftmost element is how this control is usually
>    reimplemented as the bug it replaces. A value that does not parse as an address is
>    discarded. Otherwise `X-Forwarded-For` is recorded as `Asserted` and is never used as a
>    rate-limit key. `User-Agent` and `X-Device-Id` are opaque free text and are truncated at
>    256 characters.
> 3. **Throttle** (`middleware/throttle.rs`) — on the public routes only, consume one
>    per-`ClientAddr` unit against the `RateLimiter` when the address is `Peer` or
>    `Forwarded`. A denial returns `429` and audits `ThrottleExceeded`. An `Asserted` or
>    `Unknown` address is not throttled here, because throttling on a header the client
>    controls only penalises clients that do not rotate it. The layer also carries a global
>    `ConcurrencyLimitLayer` fronted by `LoadShedLayer`, so a distributed flood that keeps
>    every individual key under budget still cannot exhaust the outbound provider and
>    key-manager capacity; a shed request returns `503`.
> 4. **Request timeout** — unchanged.
> 5. **Audit context** (`middleware/audit_context.rs`) — carry the resolved `ClientAddr`,
>    the bounded `User-Agent` and `X-Device-Id` into the `AuditContext` extension, which the
>    `/token` and `/revoke` handlers still pass into the core request structs: the stored
>    session records `ip_address`/`user_agent`/`device_id`, and audit events record the
>    address with its provenance.
> 6. **Access log** (`middleware/access_log.rs`) — one `tracing::info!` per public-route
>    request inside the request span: method, matched path, status, the `error` code where
>    one was rendered, and the `ClientAddr` kind. Never the token, the form body, or the
>    asserted header values. This is what makes a request rejected before the core reaches
>    it — an unknown `grant_type`, a malformed form, a timeout, a throttle denial — visible
>    at all.
> 7. **Catch-panic** — unchanged.

### `.specs/service/specs/04-http-api.md` → Error mapping (Modify)

> | `TooManyRequests` | 429 | `slow_down` |
> | `Overloaded` (load shed) | 503 | `server_error` |
>
> A `429` carries `Retry-After` in seconds (RFC 9110 §10.2.3) set to the time remaining in
> the current window. The `slow_down` code is RFC 8628 §3.5's token-endpoint rate-limit
> error; RFC 6749 §5.2 defines none. `429` means "this key is over its budget"; `503` means
> "the process is at its concurrency bound" — a refusal that is not attributable to the
> caller and so carries no `Retry-After`.

### `.specs/service/specs/04-http-api.md` → Bootstrap (Modify)

> 6. Detect runtime: `AWS_LAMBDA_RUNTIME_API` present → the router is served through
>    `lambda_http::run` as a tower service, accepting API Gateway REST/HTTP-API, Function URL,
>    and ALB events; otherwise bind `server.host:server.port` and serve over hyper via
>    `into_make_service_with_connect_info::<SocketAddr>()` — so the client-address middleware
>    has a real connection peer to classify — with graceful shutdown: SIGTERM or ctrl-c stops
>    accepting connections and drains in-flight requests for up to a 10 s hard deadline, after
>    which stragglers are aborted and the process exits. The middleware stack's request-timeout
>    layer bounds slow clients at `server.request_timeout` (default 30 s). Both paths run the
>    identical router, middleware stack, and `AppState`, and both strip a configured
>    `server.base_path` prefix from incoming request paths before routing
>    ([06-configuration.md](06-configuration.md)) — covering API Gateway stages and mount
>    prefixes.

### `.specs/service/specs/04-http-api.md` → Assumptions (Modify)

> - A reverse proxy or gateway terminates TLS. `X-Forwarded-For` is honoured only when the
>   connection peer is inside `server.trusted_proxies`; with the shipped empty list the
>   header is recorded as client-asserted and is not used for rate limiting or for any
>   authorization decision.
> - The internal API is reachable only from trusted callers (admin UI, scripts) on a private
>   network; the shared secret is its only authentication.
> - In-process rate-limit state is per process. A horizontally scaled deployment's effective
>   budget is the configured budget times the instance count, and under Lambda it is the
>   budget times the number of live execution environments — which the platform scales with
>   load, so under Lambda an in-process limiter bounds burst rate per environment and does
>   not bound a campaign globally. Under Lambda the effective global bound remains the
>   gateway's own usage plan.

### `.specs/service/specs/06-configuration.md` → Validation at load (Add)

> - `audit.adapter` must name a known adapter; an empty string is a `ConfigError`, not a
>   synonym for `noop`. `audit.durability` must be `observe` or `enforce`.
> - Each `server.trusted_proxies` entry must parse as a CIDR; `rate_limit.window` must parse
>   as a duration and `rate_limit.store` must be `in_process` or `none`.

### `.specs/service/specs/06-configuration.md` → Committed default (Modify)

This block is written against the default as
[2026-08-05-fail_closed_across_config_and_adapters.md](2026-08-05-fail_closed_across_config_and_adapters.md)
leaves it — that spec merges first and adds the `issuer` / `audience` placeholders that make
the committed default deliberately non-startable until an operator supplies them. Those two
lines are **carried through unchanged here**; dropping them would silently restore a bootable
default with an empty `iss`/`aud`, which is the defect that sibling exists to close.

> ```toml
> [server]
> host = "0.0.0.0"
> port = 8080
> issuer = "${OIDC_EXCHANGE_ISSUER}"
> trusted_proxies = []
> trusted_proxy_hops = 1
>
> [registration]
> mode = "open"
>
> [token]
> access_token_ttl = "15m"
> refresh_token_ttl = "30d"
> audience = "${OIDC_EXCHANGE_AUDIENCE}"
>
> [audit]
> adapter = "stdout"
> blocking_threshold = "warning"
> emit_threshold = "info"
> durability = "observe"
>
> [rate_limit]
> enabled = true
> store = "in_process"
> window = "1m"
> per_ip = 60
> per_ip_failures = 10
> per_subject = 10
> per_provider = 600
> max_concurrent_requests = 256
>
> [telemetry]
> enabled = false
> exporter = "none"
> ```
>
> The default still carries no providers and no key manager, and — since `issuer` and
> `audience` resolve from the environment or fail startup — it is not bootable as committed.
> What changes here is that it is no longer *silent*: once an operator supplies those two
> values, a fresh boot writes its security record to stdout, where every runtime this service
> targets already collects it.

### `.specs/service/specs/06-configuration.md` → Sections → `[server]` (Modify)

> …plus `trusted_proxies` (`Vec<String>` of CIDRs, default empty) — the peers whose
> `X-Forwarded-For` header the client-address middleware honours — and `trusted_proxy_hops`
> (default `1`), how many entries to count in from the right of that header to reach the
> client address. With the shipped empty `trusted_proxies` the hop count is unused.

### `.specs/service/specs/06-configuration.md` → Sections → `[audit]` (Modify)

> `adapter` (`noop` | `stdout` | `sqs`, default `stdout`), `blocking_threshold` (syslog
> severity name, default `warning`) and `emit_threshold` (default `info`) — both apply to
> the best-effort channel only — `durability` (`observe` | `enforce`, default `observe`)
> governing whether a mandatory-channel write failure fails the operation or is logged and
> tolerated, and optional `[audit.sqs] { queue_url, region }`.

### `.specs/service/specs/06-configuration.md` → Sections (Add)

> ### `[rate_limit]`
> `enabled` (bool, default `true`), `store` (`in_process` | `none`, default `in_process`),
> `window` (duration string, default `"1m"`), four per-window budgets — `per_ip` (60),
> `per_ip_failures` (10), `per_subject` (10), `per_provider` (600) — and
> `max_concurrent_requests` (256), the global in-flight bound behind which the service sheds
> rather than queues. Budgets are per window, per key, and **per process**. A zero budget
> disables that scope.
>
> `per_ip_failures` is a second, tighter budget consumed only when a request fails
> authentication, so a client can exhaust its guessing allowance without exhausting the
> allowance a legitimate user needs to log in.
>
> Under Lambda the in-process store is close to inert: each concurrent invocation may run in
> a fresh execution environment with empty counters, and the platform creates environments in
> response to the load an attacker generates. On that shape the effective control is an API
> Gateway usage plan or a WAF rate rule, and this configuration does not substitute for one.

### `.specs/service/specs/06-configuration.md` → Defaults summary (Modify)

> | `server.host` / `port` / `role` / `request_timeout` | `0.0.0.0` / `8080` / `all` / `"30s"` |
> | `server.trusted_proxies` / `trusted_proxy_hops` | `[]` / `1` |
> | `audit.adapter` / `blocking_threshold` / `emit_threshold` / `durability` | `stdout` / `warning` / `info` / `observe` |
> | `rate_limit.enabled` / `store` / `window` / `max_concurrent_requests` | `true` / `in_process` / `"1m"` / `256` |
> | `rate_limit.per_ip` / `per_ip_failures` / `per_subject` / `per_provider` | `60` / `10` / `10` / `600` |

### `.specs/service/specs/06-configuration.md` → Assumptions (Add)

> - Where rate limiting must hold globally rather than per process — every Lambda deployment,
>   and any horizontally scaled server deployment — an edge control (API Gateway usage plan,
>   WAF rate rule, ALB) provides it. The in-process limiter bounds a single process and is a
>   backstop for deployments that have none.

### `.specs/service/specs/07-telemetry-and-audit.md` → Overview table (Modify)

> | Failure mode | best-effort, never blocks | mandatory channel per `audit.durability`; best-effort per configured threshold |

### `.specs/service/specs/07-telemetry-and-audit.md` → Audit (Modify)

> Audit has two channels. The **mandatory** channel carries `SecurityEvent`s
> ([01-domain-model.md](01-domain-model.md)) through `AppService::emit_security_event`. No
> configured threshold filters it: `emit_threshold` is not consulted, and a sink failure is
> governed by `audit.durability` rather than by `blocking_threshold`. The **best-effort**
> channel is the existing `emit_audit`, retaining both thresholds, for operational events;
> no shipped flow uses it, so under the shipped configuration `emit_threshold` has no
> observable effect.
>
> Severity survives on both channels because sinks route and alert on it. What it no longer
> does on the mandatory channel is decide whether an event exists.
>
> Adapters are unchanged: `stdout_audit` (JSON lines; `Auto` sends error-and-above to
> stderr) writes with locked handles — a write failure (e.g. EPIPE from a restarted log
> collector) returns `AuditError` and flows through the emitting channel's failure handling
> rather than panicking; `sqs_audit` (one JSON message per event with a `severity`
> attribute, FIFO detected by a `.fifo` queue suffix) sets `message_group_id` to the event
> id on FIFO queues — each event is its own group, so FIFO ordering never serializes
> throughput — with the event's ULID as the deduplication id; and `noop` (drops events).
> `stdout` is the default.

### `.specs/service/specs/07-telemetry-and-audit.md` → Decisions (Modify)

Replace *Noop audit by default*:

> - *Stdout audit by default.* **The committed default uses the stdout audit adapter.** A
>   service that is a root of trust for every downstream relying party must not ship
>   configured to discard its own security record. `stdout` needs no credentials, no
>   network, and no queue, and every runtime this service targets — CloudWatch under Lambda,
>   the container log driver, journald — already collects it. `noop` remains available and
>   is the right choice for tests and local development; it is no longer the default.

### `.specs/service/specs/00-overview.md` → Non-goals (Modify)

> - Hosting login pages, managing passwords, or running a full OAuth 2.0 authorization
>   server. Authentication is delegated entirely to upstream providers.
> - Config hot-reload (restart to apply) and a token introspection endpoint (downstream
>   verifies via JWKS).
> - Multi-tenancy, RBAC beyond a single admin claim check, or SCIM provisioning.
> - A globally coordinated rate limit. The service bounds attempts per process; a shared
>   budget across instances needs the `RateLimiter` port backed by a shared store, or an
>   edge gateway.

### `.specs/service/specs/00-overview.md` → Scope summary (Modify)

> | Rate limiting (per-IP / per-subject / per-provider, in-process) | Yes | `crates/server/src/middleware/throttle.rs`; per process, not global |
> | Key rotation, config hot-reload, introspection | No | out of scope (see Non-goals) |

### `.specs/service/specs/00-overview.md` → Goals (Modify)

> - Emit structured audit events with syslog severities: security outcomes on a mandatory
>   channel no configured threshold can filter, operational events on a best-effort channel
>   behind `emit_threshold` and `blocking_threshold` — plus OpenTelemetry-style tracing.

### `.specs/service/specs/00-overview.md` → Detail pages (Modify)

> | [02-ports-and-adapters.md](02-ports-and-adapters.md) | The seven port traits and every adapter that implements them |

### `.specs/service/specs/00-overview.md` → Crate map (Modify)

> | `crates/core` | Domain types, the seven port traits, `AppService` orchestration, config, errors |

### `.specs/service/README.md` → Pages (Modify)

> | [specs/02-ports-and-adapters.md](specs/02-ports-and-adapters.md) | the seven ports and every adapter |

---

## Type changes

```json
{
  "$comment": "Fragment for 2026-08-05-audit_and_throttle_authentication_failures. Folds into .specs/service/specs/canonical-types.schema.json on merge. AuditEvent is shown with its new shape; ip_address_source is the added property. AuditEventType gains throttle_exceeded.",
  "$defs": {
    "ClientAddrSource": {
      "type": "string",
      "enum": ["peer", "forwarded", "asserted", "unknown"],
      "description": "Provenance of AuditEvent.ip_address. peer: observed by the server or supplied by the runtime platform. forwarded: read from X-Forwarded-For behind a peer in server.trusted_proxies. asserted: copied from a client-controlled header with no trusted proxy established; not admissible as a rate-limit key. unknown: no address available."
    },
    "AuditEventType": {
      "type": "string",
      "enum": [
        "token_exchange", "token_refresh", "token_revocation", "session_revoked",
        "all_sessions_revoked", "user_created", "user_updated", "user_suspended",
        "user_deleted", "validation_failed", "registration_denied", "provider_error",
        "unauthorized", "throttle_exceeded"
      ]
    },
    "AuditEvent": {
      "type": "object",
      "required": ["id", "timestamp", "severity", "event_type", "ip_address_source", "detail", "outcome"],
      "properties": {
        "id": { "$ref": "../../canonical-types.schema.json#/$defs/Ulid" },
        "timestamp": { "$ref": "../../canonical-types.schema.json#/$defs/Timestamp" },
        "severity": { "$ref": "#/$defs/AuditSeverity" },
        "event_type": { "$ref": "#/$defs/AuditEventType" },
        "actor": { "type": ["string", "null"], "description": "User id if known." },
        "provider": { "type": ["string", "null"] },
        "ip_address": { "type": ["string", "null"] },
        "ip_address_source": { "$ref": "#/$defs/ClientAddrSource" },
        "user_agent": { "type": ["string", "null"] },
        "detail": { "type": "object", "additionalProperties": true },
        "outcome": { "$ref": "#/$defs/AuditOutcome" }
      }
    }
  }
}
```

`SecurityEvent`, `ClientAddr`, `RateLimitKey` and `RateLimitDecision` are in-process types
that never cross a persistence or wire boundary, so they get no `$def`. `Session` is
unchanged: it keeps `ip_address` with no provenance field, because adding one is a schema
migration across five storage adapters for a value the audit trail already qualifies.

---

## Implementation notes

Land in this order; steps 1–2 are independently shippable and close the default blind spot
before any of the structural work.

1. **Default sink.** `config/default.toml:13` and `AuditConfig::default`
   (`crates/core/src/config.rs:214-223`): `adapter` becomes `"stdout"`. Note the bootstrap
   match arm is `"noop" | "" =>`, so a blanked key silently selects the bit bucket too;
   make the empty string a config-validation error rather than a synonym for `noop`. Also
   fix `examples/container/config/production.toml:21-22`, which hard-codes `noop` in a file
   named `production.toml`, and `docs/deployment/aws-lambda.md:88`, which recommends
   `blocking_threshold = "error"` — a value that puts every event the service routinely
   emits on the silent side. `emit_threshold` appears nowhere in `docs/guides/configuration.md`
   and needs adding.
2. **`ValidationFailed` severity.** `crates/core/src/service/refresh.rs:37`, `:56`, `:76`:
   `AuditSeverity::Debug` → `Warning`. This is the interim cover until step 4 lands and is
   what makes a failed refresh visible under the shipped `emit_threshold`.
   `crates/core/tests/audit.rs:184-221`
   (`audit_debug_event_under_default_emit_threshold_is_suppressed`) currently asserts the
   suppression as intended design and must be **updated to use a non-security event type**,
   not deleted — leaving it as-is pins the defect into the test suite.
3. **`SecurityEvent`.** New `crates/core/src/domain/security_event.rs` with the variant
   table above, `severity()`, `event_type()`, and `into_audit_event(ctx)`. Add
   `ThrottleExceeded` to `AuditEventType` (`crates/core/src/domain/audit.rs:39-53`) and
   `ip_address_source` to `AuditEvent` (`:8-21`); `create_audit_event`
   (`crates/core/src/service/mod.rs:146-167`) takes a `ClientAddr` instead of
   `Option<String>`.
4. **Channel split.** Add `AppService::emit_security_event` beside `emit_audit`
   (`crates/core/src/service/mod.rs:102`). It skips the `emit_threshold` early return
   (`:106-110`) entirely and replaces the `blocking_threshold` comparison (`:129`) with the
   `audit.durability` decision. Add `durability: String` (default `"observe"`) to
   `AuditConfig` (`crates/core/src/config.rs:203-212`) and validate it in
   `AppConfig::validate`. Increment an `audit_sink_failures_total` counter on the fallback
   path and have `health_handler` (`crates/server/src/routes/health.rs`) report degraded
   instead of a constant `{"status":"ok"}` after consecutive failures — under `observe` that
   counter is the only signal the trail has a hole in it.
   `crates/core/tests/audit.rs:49-74`
   (`non_blocking_audit_failure_info_event_warning_threshold`) asserts today's fail-open
   behaviour and must be updated alongside.
5. **Single-exit emission.** Rename the body of `exchange`
   (`crates/core/src/service/exchange.rs:64`) to `exchange_inner`; the new `exchange` awaits
   it, maps the `Result` to one `SecurityEvent`, emits, and returns. Delete the per-branch
   failure emissions at `:104`, `:132`, `:148`, `:166`, `:186`, `:252` and the terminal
   success emission at `:327` — the wrapper now covers all of them. Keep `PrincipalCreated`
   at `:209`. Do the same for `refresh` (`refresh.rs:22`), deleting `:35`, `:54`, `:74`,
   `:91` and `:110`. `revoke` (`revoke.rs:29`) takes the same wrapper treatment: its two
   existing call sites keep emitting on success and the wrapper adds the rejection event, so
   both branches emit exactly once while the handler still returns `200` either way — the
   emission must not become conditional on the outcome, or `durability = "enforce"` turns a
   sink outage into an existence oracle. Because the wrapper lives in `crates/core`, the
   guarantee holds for the hyper, Lambda and `crates/ffi` entry points alike.
   Two details are load-bearing. The mapped `reason` is a fixed classification string, never
   `e.to_string()` — `Error::ProviderError`'s `Display` embeds the upstream response body
   (`crates/adapters/src/shared/token_endpoint.rs:44-58`), so passing it through would trade
   a logging gap for a data leak. And when the terminal success emit fails under
   `durability = "enforce"`, the wrapper calls `session_repo.revoke_session(hash)` for the
   session written at `exchange.rs:313` before returning the error; without that, a failed
   exchange leaves a live refresh token the caller never received and no record describes.
6. **`ClientAddr` + trusted proxies.** Add `into_make_service_with_connect_info::<SocketAddr>()`
   at `crates/server/src/main.rs:67` — a middleware-only fix has no peer address to fall back
   on without it. Add `server.trusted_proxies` (CIDR list) and `server.trusted_proxy_hops`;
   add `crates/server/src/middleware/client_addr.rs` selecting the hop from the **right** of
   the chain and parsing it as an `IpAddr`. Truncate `User-Agent` and `X-Device-Id` at 256
   characters in `crates/server/src/middleware/audit_context.rs:25-41`. In Lambda mode read
   the source IP from the event request context before consulting any header; under
   `crates/ffi` there is no peer, so record `Unknown` rather than a header value. Revisit
   `docs/deployment/linux-server.md:144-150`, whose nginx block appends to `X-Forwarded-For`
   (putting the client's value leftmost, where an investigator looks first) and sets an
   `X-Real-IP` the service ignores.
7. **`RateLimiter`.** New `crates/core/src/ports/rate_limit.rs`; in-process fixed-window
   implementation with a bounded, expiry-evicted map in
   `crates/server/src/middleware/throttle.rs`; `NoopRateLimiter` in `crates/adapters/noop`.
   Wire the layer into `build_router` (`crates/server/src/bootstrap.rs:352-360`) between the
   client-address and timeout layers, on `routes::public_routes()` only
   (`crates/server/src/routes/mod.rs:13-23`), together with
   `tower::load_shed::LoadShedLayer` in front of
   `tower::limit::ConcurrencyLimitLayer::new(config.rate_limit.max_concurrent_requests)`.
   Add `Error::TooManyRequests { retry_after_secs }` and its 429 arm in
   `map_domain_error_inner` (`crates/server/src/error.rs:80`). Consume the per-provider unit
   before the outbound call and the per-subject unit after validation inside `exchange_inner`
   and `refresh`; consume `per_ip_failures` from the throttle layer when the response is an
   authentication failure.
8. **Access log.** `crates/server/src/middleware/access_log.rs`, on the public routes,
   inside the request span so it inherits `request_id`.

Tests to add: exactly one `SecurityEvent` per public-route failure class, asserted as a
property over the outcome space rather than three examples (the direct test for the
single-exit guarantee); `emit_threshold = "emergency"` still delivers every security event;
a failing `MockAuditLog` under `durability = "enforce"` makes `POST /token` fail **and
leaves no session behind**; with the same failing sink under `enforce`, `POST /revoke`
returns the **same status for a token that exists and one that does not** — the property
that keeps the enforce path from becoming an existence oracle, and the reason revocation
emits on both branches rather than only on success; two bursts from one peer carrying different `X-Forwarded-For`
values share one budget, and the forged header is recorded with
`ip_address_source = "asserted"`; a request from a configured trusted proxy records the hop
selected by `trusted_proxy_hops`, not the leftmost element; with no `X-Forwarded-For` at all
the peer address is recorded — the case that catches a regression in the `main.rs` plumbing,
which no middleware-only test can see; the 61st request in a window returns 429 with
`Retry-After` **and the mock provider received no further outbound call**, so a limiter that
refuses only after doing the work still fails.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**`.
2. Fold the `Type changes` `$defs` into
   `.specs/service/specs/canonical-types.schema.json` — `ClientAddrSource` is new,
   `AuditEventType` and `AuditEvent` replace their current definitions.
3. Remove the resolved Open question from
   [`02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) only if the
   audit-sink question is settled by then; it is untouched by this change.
4. No new canonical page.
5. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, move it to
   `.specs/changes/merged/`.
6. Update `.specs/README.md`'s Change specs section.

---

## Assumptions and open questions

### Assumptions

- Every runtime this service targets collects process stdout: CloudWatch Logs under Lambda,
  the container log driver, journald under systemd. The stdout default is only an
  improvement where that holds.
- An operator who configures `[audit.sqs]` has accepted that the sink is on the request
  path; the stdout default means the durability coupling reaches no one who did not choose a
  remote sink.
- The edge gateway or ALB in front of a production deployment remains the coarse volume
  control. The in-process limiter is a backstop for the deployments that copy a shipped
  example without one, not a replacement.

### Decisions

- *The threshold cannot reach the security record.* **`emit_threshold` applies to the
  best-effort channel only.** Raising `ValidationFailed` from `Debug` to `Warning` would fix
  today's symptom and leave the mechanism: a knob whose purpose is controlling log noise
  would still have authority over whether an authentication failure is recorded. Separating
  the channels means the next event someone adds cannot be given an invisible severity,
  because there is no severity that is invisible.
- *Authentication failures are `Warning`(4).* **`AuthenticationFailed`,
  `RegistrationDenied`, `PrincipalSuspended`, `ProviderRejected` and `ThrottleExceeded` all
  sit at `Warning`.** RFC 5424 defines `warning` as a condition that is not an error but
  should be handled specially, which is what a failed authentication against a root of trust
  is — it is expected traffic individually and a signal in aggregate. `Warning` is also
  where the shipped `blocking_threshold` sits, so these events remain the most severe class
  a downstream SIEM sees from normal operation without a second config change. `Notice` and
  `Info` are reserved for state changes and successes, which is what makes a severity-sorted
  view of the sink useful.
- *This reverses the `debug` decision from 2026-07-01.* **`ValidationFailed` is no longer a
  debug-level signal one config knob away.** That change spec's reasoning — keep the default
  pipeline quiet — was sound about volume and wrong about placement. Volume belongs on the
  best-effort channel and at the sink; the security record is not the place to economise.
- *Audit durability fails the operation, and the cost is an availability coupling.*
  **`durability = "enforce"` is the intended end state: a mandatory-channel write failure
  fails `/token` and `/revoke`.** This is a deliberate reduction in availability. A
  deployment whose SQS queue is unreachable will stop issuing tokens. For most services that
  trade is wrong; for a service that is the authentication root of trust for every
  downstream relying party it is right, because the alternative is authentications that
  happened with no evidence they happened — and a security control whose failure mode is
  silence is not a control. The escape hatch is explicit (`durability = "observe"`), never
  implicit.
- *`observe` ships first.* **The release that introduces `durability` defaults to `observe`;
  the following release changes the default to `enforce`.** Enforcing a new availability
  coupling on operators who have never seen their own sink's reliability data would convert
  an unknown into an incident. `observe` logs the failure with `audit_durability_degraded =
  true` so that data exists before the default changes.
- *The throttle fails open; the audit channel fails closed.* **A `RateLimiter` error is
  logged and the request proceeds.** The two controls fail in opposite directions on
  purpose: an audit channel that fails open destroys the evidence it exists to produce,
  while a throttle that fails open costs only the rate bound — a defence-in-depth layer
  behind an edge gateway, whose loss is bounded and visible in the access log.
- *Per-IP limiting keys only on an address the server established.* **`ClientAddr::Asserted`
  is never a rate-limit key.** `X-Forwarded-For` is client-controlled here — there is no
  trusted-proxy model, `crates/server/src/main.rs:67` calls `axum::serve` without
  `ConnectInfo` so no peer address exists to fall back on, and the repository's own Linux
  deployment guide ships an *appending* proxy directive, which puts the client's value
  leftmost. Throttling on it would penalise exactly the clients that do not rotate it while
  leaving an attacker who does entirely unbounded.
- *Three scopes, three different jobs.* **Per-IP bounds volume from one source, per-provider
  bounds this service's use as an amplifier against an upstream, per-subject bounds replay
  of one valid credential.** No single key covers all three: per-IP collapses under NAT and
  under Lambda, per-subject is only knowable after validation has already been paid for, and
  per-provider is global so it cannot isolate one attacker.
- *Failures get their own, tighter budget.* **`per_ip_failures` is consumed only by requests
  that fail authentication.** A single budget shared by successes and failures means a
  guessing campaign from a NAT gateway exhausts the allowance the legitimate users behind
  that gateway need to log in — the throttle would deliver the denial of service it exists to
  prevent. Separating them lets an attacker spend their guessing allowance without spending
  anyone else's login allowance.
- *In-process, not shared.* **The shipped limiter's state is per process.** A shared store on
  `/token` is a new dependency on the hot path and should be measured before it is added;
  the `RateLimiter` port exists so that a Valkey-backed implementation slots in without
  touching the middleware. The honest consequence is stated in the canonical Assumptions:
  under Lambda, where the platform scales execution environments with load, an in-process
  limiter bounds per-environment burst rate and does not bound a campaign globally.
- *429 uses `slow_down`.* **RFC 8628 §3.5, with `Retry-After` per RFC 9110 §10.2.3.**
  RFC 6749 §5.2 defines no rate-limit error code, and `slow_down` is the registered
  token-endpoint code for this condition.
- *Revocation records both outcomes.* **`/revoke` emits one event whether the token was
  revoked or rejected, and returns `200` either way.** RFC 7009 §2.2 governs the
  client-visible channel; the audit record is operator-facing, so recording a rejection
  discloses nothing to the caller. Under `durability = "enforce"` it is the asymmetric
  alternative that leaks — emitting only on success answers `503` for a token that existed
  and `200` for one that did not, whenever the sink is down. There is therefore no exception
  to "every public-route failure that reaches the core produces exactly one event"; the
  volume of unauthenticated probing is bounded by the per-IP limiter, not by silence.

### Open questions

- Does the mandatory channel need a bounded local durable buffer so `enforce` tolerates a
  transient sink outage without failing requests? It removes most of the availability
  objection and adds a durability surface of its own. The `observe`-release telemetry should
  decide it, for the SQS adapter specifically.
- `emit_threshold` governs an empty set once every shipped flow uses the mandatory channel.
  Retaining a key with no observable effect is itself a trap; whether to remove it outright
  should be settled once it is confirmed that no `crates/ffi` embedder depends on
  `emit_audit`.
- What is the right per-IP budget behind a large NAT? 60 per minute is a guess calibrated to
  interactive login traffic. It should be re-derived from an observe-only run that counts
  what would have been throttled before enforcement is trusted.
- `Session.ip_address` still carries no provenance field while `AuditEvent` gains one.
  Whether the asymmetry is worth a five-adapter storage migration is open.
- Should `AuditEvent` record the resolved address *and* the raw forwarded chain as two
  separate fields? Operators behind a CDN genuinely need the client hint, and keeping the two
  distinct lets an investigator see at a glance which is trustworthy. This change records one
  address plus its provenance, which is the smaller step.
- Should the internal API's shared-secret failures also emit `AuthenticationFailed` and
  consume a rate-limit budget? The gap is real (`/internal/*` has neither), but the
  credential redesign belongs to the admin-plane hardening proposal, not here.
- The single-exit wrapper lives in `crates/core`, where it covers the hyper, Lambda and
  `crates/ffi` entry points alike — a deliberate divergence from the hardening proposal's
  suggestion to emit at response rendering. The cost: a request rejected before the core is
  reached (an unknown `grant_type`, a malformed form) produces an access-log line but no
  `SecurityEvent`. Whether those handler-level 400s deserve an event of their own or the
  access log suffices is open.
