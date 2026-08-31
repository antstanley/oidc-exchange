# Change: Give embedded deployments an operator-visible signal for internal exchange faults

**Status:** Proposed · **Date:** 2026-08-31 · **Owner:** Ant Stanley · **Target:** crates/ffi, crates/server, crates/core, schemas (service, bindings)

Close the two independent gaps behind GitHub issue #47 (affects `@oidc-exchange/*` 0.4.0,
tag `v0.4.0`): in an embedded deployment — the Node/Lambda/Python bindings over
`crates/ffi` — an internal fault on the token-exchange path returns
`500 {"error":"server_error"}` with no log line and no audit event. First, install the
`tracing` subscriber on the embedded entrypoint: `OidcExchange` construction will run the
server's `init_telemetry`, made idempotent and host-respecting via `try_init`, so the 500
error mapping's diagnostic log (`crates/server/src/error.rs:104`) and the FFI
panic-boundary log (`crates/ffi/src/lib.rs:221`) reach the host process's stdout under
`RUST_LOG` control instead of being discarded (G1). Second, stop the exchange flow's
`StoreError` early return from being wholly unaudited: the flow will record one operational
`StoreError` audit event — `Error` severity, best-effort channel, never a `SecurityEvent` —
so a `durability = "observe"` deployment gets an audit-stream record of the infrastructure
fault while the existing semantic that infrastructure faults are not client rejections is
preserved (G2). G1 lands once in `crates/ffi` and reaches the Node, Lambda, and Python
packages with no binding-code changes.

---

## Motivation

Issue #47's incident: a production exchange failure in an embedded deployment surfaced only
as `500 {"error":"server_error","error_description":"internal server error"}`. The server
layer had logged the exact underlying `ValidationException` message — the 500 mapping logs
the full internal `Display` at `crates/server/src/error.rs:103-104`, and the embedded
runtime executes that same router and mapping — but no `tracing` subscriber existed to
receive it. `init_telemetry` is called only by the standalone server binary
(`crates/server/src/main.rs:29`); neither `crates/ffi/src/lib.rs` nor the Node binding
(`bindings/nodejs/src/lib.rs`) installs one, and `tracing-subscriber` is a dev-only
dependency of `crates/ffi` (`crates/ffi/Cargo.toml:29`). Every `tracing::error!` in an
embedded process — the 500 mapping, the panic-boundary record, adapter warnings — is
discarded regardless of `RUST_LOG`; `RUST_BACKTRACE=1` adds nothing because nothing
unwinds uncaught. Diagnosing the fault required reading the source.

The audit stream could not fill the gap either, by design that predates the embedded
deployment shape: `AppService::exchange` emits a terminal security event for every outcome
except `ExchangeFlowError::Other(Error::StoreError)`, which returns early with no emission
(`crates/core/src/service/exchange.rs:158-162` — "Infrastructure ≠ client fault"). That
classification is right — a store fault is not an authentication outcome — but with
`durability = "observe"` and no subscriber, "not a security outcome" degraded into "no
signal at all". Because the stdout audit adapter writes via `writeln!` on locked handles
(`crates/adapters/src/stdout_audit/mod.rs`), bypassing `tracing`, provider-rejection audit
events were visible while internal faults left nothing — the asymmetry that masked a
downstream integration bug for hours. G1 restores the shared log line for every 5xx in
every flow; G2 adds the audit-stream record for the one outcome the exchange flow
deliberately does not emit.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/07-telemetry-and-audit.md`](../service/specs/07-telemetry-and-audit.md) | Modify: the subscriber-install paragraph names both entrypoints and the idempotent, host-respecting install; the `RUST_LOG` paragraph names the embedded log destination; the Audit section's channel sentence records the exchange flow's operational `StoreError` event on the best-effort channel |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | Modify: the exchange intro's emission paragraph records the operational `StoreError` event; step 3's store-failure paragraph points at it; the Audit-emission closing paragraph stops describing `emit_audit` as embedder-only |
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md) | Modify: the `AuditEventType` variant list gains `StoreError` (and, per the republished-list completeness rule, the three shipped operator-auth variants it predates) |
| [`.specs/service/specs/canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | Modify: the `AuditEventType` `$def` enum — the only entity this change alters (the sidecar's `AuditOutcome.reason` is an untyped string and needs no edit) |
| [`.specs/bindings/specs/01-ffi-core.md`](../bindings/specs/01-ffi-core.md) | Add: a telemetry-install responsibility. Modify: the "no global state" sentence names the one deliberate piece of process-global state. Add: a constructor-installed-telemetry Decision |
| [`.specs/bindings/specs/02-nodejs.md`](../bindings/specs/02-nodejs.md) | None — the binding marshals only; telemetry install is inherited from the FFI core it already defers to, with no API or code change |
| [`.specs/bindings/specs/04-lambda.md`](../bindings/specs/04-lambda.md) | None — pure-TS adapter over the Node binding; the host process's stdout (CloudWatch) receives the inherited JSON lines |
| [`.specs/bindings/specs/03-python.md`](../bindings/specs/03-python.md) | None — same inheritance through `crates/ffi` |
| [`.specs/service/specs/00-overview.md`](../service/specs/00-overview.md) | None — its audit goal (security outcomes mandatory, "operational events on a best-effort channel") becomes true of a shipped in-tree operational event exactly as written |

`schemas/datamodel.schema.json` is a code-side artifact, not a canonical page; its delta is
in [The delta → G2](#g2--record-the-exchange-flows-infrastructure-store-fault) below.

---

## The delta

### G1 — Install the telemetry subscriber on the embedded entrypoint

One telemetry init, two entrypoints — the same shape as the config pipeline's "one resolve,
differing sources" rule (`01-ffi-core.md`):

- `init_telemetry` (`crates/server/src/telemetry.rs:22-47`) becomes idempotent and
  host-respecting: the `tracing_subscriber::fmt()` builder's `.init()` call at `telemetry.rs:37-40`
  (which panics if a global dispatcher is already set) becomes `try_init()`. On
  `Err` — a global dispatcher already installed, whether by an earlier `OidcExchange`
  construction or by a host application that owns its own subscriber — the function
  returns `Ok(())`, emits a `tracing::debug!` through the existing dispatcher noting the
  installed subscriber is retained, and skips the exporter fallback warning (the warning
  describes a subscriber this call installed; on the retained path nothing was installed).
  The standalone server's behaviour is unchanged: `main` runs before any subscriber can
  exist, so its call still installs and still warns for `otlp`/`xray`/`prometheus`.
- `OidcExchange::new_with_base_path` (`crates/ffi/src/lib.rs:102-184`) calls
  `oidc_exchange::telemetry::init_telemetry(&config.telemetry)` immediately after config
  resolution and the base-path override (after `lib.rs:126`), before the runtime is
  created and `build_service` runs, so bootstrap-time warnings (e.g. the single-plane
  `role = "all"` warning) are captured too. A returned error — practically unreachable
  once already-set maps to `Ok` — propagates as `FfiError { code: "SERVICE_ERROR" }`,
  the same class as a `build_service` failure. Config-parse failures precede the install
  and keep reaching the host as `CONFIG_ERROR` values; nothing is lost there because the
  error itself crosses the boundary.
- No dependency changes. The FFI already depends on the server crate
  (`crates/ffi/Cargo.toml:11`), which carries `tracing-subscriber` with the
  `env-filter`/`json` features (`crates/server/Cargo.toml:20`); the issue's suggested
  promotion of `tracing-subscriber` into `crates/ffi` is unnecessary and rejected (see
  Decisions). The existing dev-dependency stays for the panic-boundary capture tests.
- No binding changes. The napi constructor (`bindings/nodejs/src/lib.rs:78-100`), the
  Lambda TS adapter, and the Python binding all construct through
  `new`/`new_with_base_path`, so the install reaches every published embedded package from
  the one FFI call site.

The result in an embedded process: JSON log lines on the host's stdout, filtered by
`RUST_LOG` (default `info`), exactly as the standalone server emits — including the 500
mapping's `internal error mapped to error response` line carrying the full internal
`Display` (`error.rs:103-104`) and the `panic contained at FFI request boundary` record
(`lib.rs:221`).

Tests:

- `crates/ffi/tests/telemetry_install.rs` (its own binary — a global dispatcher is
  process-wide, so each scenario needs its own process): constructing an instance over the
  minimal admin-role SQLite config (the `embedder_tests` fixture shape,
  `crates/ffi/src/lib.rs:628-670`) leaves `tracing::dispatcher::has_been_set()` true, and
  constructing a second instance neither panics nor fails — it serves `/health` — pinning
  `try_init` idempotency across instances.
- `crates/ffi/tests/telemetry_host_respect.rs`: a capturing subscriber installed via
  `tracing::subscriber::set_global_default` *before* construction; construction succeeds,
  and a request carrying an invalid header name is answered while the host's subscriber
  captures the deterministic `invalid request headers dropped at FFI boundary` warning
  (`lib.rs:311-315`) — pinning both host-respect and the end-to-end operator signal.
- A server-side companion (`crates/server/tests/` integration binary): `init_telemetry`
  called twice returns `Ok` both times — the double-init panic is unrepresentable.
- The FFI's in-crate unit tests are unaffected: they construct via `with_router_for_test`,
  which bypasses `new` and installs nothing, so their scoped `set_default` captures stay
  valid. The existing embedder test will now install the process-global subscriber for its
  test binary; that is harmless (JSON lines on test stdout) and noted here so a reviewer
  is not surprised.

### G2 — Record the exchange flow's infrastructure store fault

The `StoreError` early return keeps its classification — no `SecurityEvent`, no client
attribution, the same 5xx to the caller — and gains one operational audit record on the
best-effort channel:

- `AuditEventType` (`crates/core/src/domain/audit.rs:56-81`) gains `StoreError`
  (serialized `store_error`); `AuditFailure` (`audit.rs:360-376`) gains `StoreError`
  (`store_error`) so the outcome can name the reason. Neither joins `SecurityEvent`
  (`audit.rs:166-190`): the closed security-outcome set — the set `00-overview.md`'s
  mandatory-channel goal quantifies over — is deliberately unchanged.
- The arm at `crates/core/src/service/exchange.rs:158-162` stops returning silently.
  Before returning the error it assembles one event via `create_audit_event`
  (`crates/core/src/service/mod.rs:368-391`): `event_type: StoreError`,
  `severity: AuditSeverity::Error`, `outcome: Failure(AuditFailure::StoreError)`,
  `actor: None` (the arm sees only the typed error; identity may not have been
  established), `provider: Some(request.provider)`, the request's `client_addr` and
  `user_agent`, and `detail.store_detail` carrying the `StoreError`'s `detail` string —
  the same diagnostic the 500 mapping logs, and precisely the string whose absence
  motivated issue #47. It emits through best-effort `emit_audit` (`mod.rs:255-274`) and
  **discards the emission result**: on a sink failure `emit_audit` has already logged the
  serialized event through `log_audit_fallback` (`mod.rs:348`), and the original
  `StoreError` — not an `AuditError` — is what the caller must receive. Both `Other`
  routes into this arm are covered: direct store failures and
  `AssertionBindError::Store` (`exchange.rs:567-579`).
- Channel semantics are the best-effort channel's own, unmodified: `Error` (3) survives
  the committed `emit_threshold = "info"` (`config/default.toml:35`) and is dropped only
  by an operator raising the threshold past it; `blocking_threshold` and
  `audit.durability` never govern this event (the discard above, and the mandatory
  channel not being used, respectively). The wire shape is the standard `AuditEvent`.
- `schemas/datamodel.schema.json` — the cross-adapter source of truth — gains
  `store_error` in the `event_type` enum (line 69) and the `outcome.reason` enum
  (line 85). The S6 mirror guard (`crates/core/tests/datamodel_schema_mirror.rs`) makes
  this mechanical: the new variants are first a compile error in its exhaustive builders
  (`all_event_types`, `all_failures`), then an equality failure until the schema is
  updated.

Refresh and revoke also propagate store faults without a terminal emission; they are
deliberately out of scope here (see Decisions) — G1 already restores the shared 500/503
mapping log for every flow, and extending the operational store-fault record is an Open
question for a follow-up.

Tests (beside `crates/core/tests/exchange.rs`, reusing its failing-store fixtures):

- An exchange against a failing session store returns `StoreError` and the recording sink
  observed exactly one `store_error` event — severity `error`, outcome
  `failure`/`store_error`, the provider named, `actor` absent, `ip_address`/`user_agent`
  from the request, `detail.store_detail` non-empty — and no terminal `SecurityEvent`.
- With `emit_threshold` raised above `Error` (e.g. `critical`), the event is not emitted
  (best-effort semantics pinned) and the response is unchanged.
- Failing store **and** failing audit sink: the returned error is still the `StoreError`,
  never an `AuditError` — the discard is pinned.
- With `audit.durability = "enforce"` and a failing sink: still the `StoreError`; the
  mandatory-channel durability contract does not govern this event.
- The existing terminal-emission suite (`crates/core/tests/exchange_mandatory_outcomes.rs`)
  keeps passing unchanged: client-fault outcomes gain no store-fault event.

---

## Proposed changes

### `.specs/service/specs/07-telemetry-and-audit.md` → Telemetry (`telemetry::init_telemetry`) (Modify)

The section's opening paragraph becomes:

> Instrumentation uses the `tracing` ecosystem: `crates/core` and the adapters emit spans
> and events with no OTEL awareness; only entry points install the subscriber. The server
> binary installs it at startup, before any other work; the FFI core installs the same
> subscriber during `OidcExchange` construction, after config resolution and before any
> adapter is built, so both runtimes capture all subsequent spans. `init_telemetry` is
> idempotent and host-respecting: it installs through `try_init`, so an already-set global
> dispatcher — a second embedded instance, or a host application that installed its own
> subscriber — is retained rather than fought over, and the call reports success without
> installing (skipping the exporter fallback warning, which describes only a subscriber
> the call itself installed).

and the paragraph after the exporter list becomes:

> The env filter honours `RUST_LOG`, defaulting to `info`. In Lambda these JSON lines are
> captured by CloudWatch Logs; in containers by the log driver; in embedded deployments
> (the Node, Python, and Lambda bindings over `crates/ffi`) they appear on the host
> process's stdout — which is what makes the 500 error mapping's internal diagnostic and
> the FFI panic-boundary record operator-visible there.

(The exporter-behaviour list between them is untouched by this change; two pending change
specs hold pens on that list, and the 2026-06-24 exporter pen extends past it to a
trailing instrumentation sentence — see the Merge plan.)

### `.specs/service/specs/07-telemetry-and-audit.md` → Audit (Modify)

The paragraph's closing sentence — "Shipped flows use the mandatory channel." — becomes:

> Shipped flows emit their security outcomes on the mandatory channel. The best-effort
> channel is not embedder-only: the exchange flow records its infrastructure store fault
> there as an operational `StoreError` event at `Error` severity — an operational record
> of a 5xx condition, deliberately not a `SecurityEvent`
> ([03-service-flows.md](03-service-flows.md)).

(The pending [`2026-08-25-close_r2_audit_code_divergences.md`](2026-08-25-close_r2_audit_code_divergences.md)
rewrites the same sentence to name the refresh flow's Debug-level `ValidationFailed`
exception; whichever merges second composes both qualifications — see the Merge plan.)

### `.specs/service/specs/03-service-flows.md` → Token exchange (`exchange.rs`) (Modify)

The intro's emission paragraph becomes:

> Emission is terminal and single: the flow maps its result to exactly one
> `SecurityEvent`, with fixed classification strings rather than upstream error text (an
> assertion-binding rejection's detail-enriched `ValidationFailed` record is that terminal
> event). Infrastructure store failures remain 5xx conditions and are never recorded as
> authentication outcomes; instead of returning silently, the flow records one operational
> `StoreError` audit event for them — `Error` severity, outcome `failure`/`store_error`,
> `detail.store_detail` carrying the store's diagnostic — through best-effort
> `emit_audit`, whose own failure is discarded so the original `StoreError` always
> reaches the caller. Success is emitted after storing the session and signing the access
> token; under `audit.durability = "enforce"`, a failed terminal emit revokes that
> just-stored session before returning the error. Principal creation is a separate
> state-change event, so a losing JIT-registration racer emits none.

### `.specs/service/specs/03-service-flows.md` → Token exchange, step 3 closing paragraph (Modify)

> Store failures during the two atomic operations propagate as typed infrastructure
> errors (`StoreError` → 5xx), never disguised as client-fault rejections; no rejection
> audit is emitted for them — the exchange wrapper records them as the flow's single
> operational `StoreError` event instead.

### `.specs/service/specs/03-service-flows.md` → Audit emission (`service/mod.rs`) (Modify)

The section's closing paragraph becomes (the two-channel algorithm block above it is
unchanged):

> Severity follows RFC 5424 (emergency 0 … debug 7); lower is more severe. Every shipped
> flow emits its security outcomes on the mandatory channel. The HTTP public per-IP
> throttle also emits `ThrottleExceeded` through this same API before returning its
> terminal `429`; the middleware logs an enforce-mode emission error but preserves the
> `429`, so audit-sink behavior cannot make the denial unsafe. `emit_audit` carries
> operational events — the exchange flow's infrastructure `StoreError` record, and events
> supplied by embedders — and only that best-effort channel is governed by
> `emit_threshold` and `blocking_threshold`; the exchange wrapper discards a store-fault
> emission failure (already logged by the fallback) so an audit-sink error can never
> displace the `StoreError` the caller must receive.

(The pending R2 change spec rewrites this same paragraph for the refresh flow's retained
`ValidationFailed` exception; composition is a Merge-plan obligation.)

### `.specs/service/specs/01-domain-model.md` → AuditEvent (`domain/audit.rs`) (Modify)

The variant list becomes:

> `AuditEventType` variants: `TokenExchange`, `TokenRefresh`, `TokenRevocation`,
> `SessionRevoked`, `AllSessionsRevoked`, `UserCreated`, `UserUpdated`, `UserSuspended`,
> `UserDeleted`, `ValidationFailed`, `RegistrationDenied`, `ProviderError`,
> `Unauthorized`, `ThrottleExceeded`, `RefreshTokenReuse`, `MissingCredential`,
> `InvalidCredential`, `NotConfigured`, `StoreError`.

(`StoreError` is this change's addition. The three operator-auth variants have shipped
since the admin-plane hardening and the list this change republishes predates them — a
republished exhaustive list must be true on merge, so they fold in here rather than wait
for the deferred sidecar/doc backlog; see the *Republished-list completeness* Decision.
The paragraph's remaining sentences — `ip_address_source`, `AuditOutcome` serialization —
stand unchanged.)

### `.specs/bindings/specs/01-ffi-core.md` → Responsibilities (Add)

A fourth bullet joins the list:

> - Install the process-wide `tracing` subscriber at construction — the same
>   `init_telemetry` the server binary runs at startup
>   ([07-telemetry-and-audit.md](../../service/specs/07-telemetry-and-audit.md)) — after
>   config resolution and before any adapter is built, so internal diagnostics (the 500
>   error mapping's log line, the panic-boundary record, adapter warnings) reach the
>   embedder's stdout under `RUST_LOG` control. The install is idempotent and
>   host-respecting: an already-set global dispatcher is retained untouched.

### `.specs/bindings/specs/01-ffi-core.md` → Request flow (Modify)

The paragraph's closing sentence — "Multiple `OidcExchange` instances with different
configs can coexist; there is no global state." — becomes:

> Multiple `OidcExchange` instances with different configs can coexist; the one deliberate
> piece of process-global state is the `tracing` dispatcher — the first construction (or
> the host) installs it, and every later instance observes it unchanged, so the
> `[telemetry]` table of any instance after the first installer does not re-route logs.

### `.specs/bindings/specs/01-ffi-core.md` → Assumptions and open questions → Decisions (Add)

> - *Constructor-installed telemetry, not a host-called hook.* **`OidcExchange`
>   construction installs the subscriber itself, through the server's idempotent
>   `init_telemetry`.** An exported `initTelemetry()` that Node, Lambda, and Python hosts
>   must remember to call at cold start recreates the silent-discard bug for every host
>   that forgets; installing at construction is fail-safe, and `try_init` keeps it correct
>   when the host already owns a subscriber. Reusing the server's function keeps one
>   telemetry pipeline for both entrypoints and adds no `tracing-subscriber` dependency to
>   `crates/ffi` — the install rides the existing server-crate dependency.

(This page's `## Runtime parity update` appendix is deliberately untouched; merging it into
the body remains the deferred S15 pass.)

---

## Type changes

Fragment for `.specs/service/specs/canonical-types.schema.json`. One entity is altered:
`AuditEventType`, shown complete in its post-merge shape so the fold is a wholesale `$def`
replacement. The sidecar's `AuditOutcome.reason` is `["string","null"]` — no enum — so the
new `AuditFailure` variant needs no sidecar edit; the enum-carrying artifact is the
code-side `schemas/datamodel.schema.json`, specified in
[The delta → G2](#g2--record-the-exchange-flows-infrastructure-store-fault). The sidecar's
other known staleness (`AuditEvent.operator`, `UserPage`/`OperatorPrincipal`) remains with
the deferred doc pass and is not folded in here.

```json
{
  "$comment": "Fragment for 2026-08-31-embedded_telemetry_and_store_fault_audit. AuditEventType is a modified entity shown complete in its post-merge shape (replace the sidecar's $def wholesale). Diff vs the current sidecar: store_error is this change's addition; missing_credential, invalid_credential, and not_configured are shipped values the republished enum picks up under the republished-list completeness rule.",
  "$defs": {
    "AuditEventType": {
      "type": "string",
      "enum": [
        "token_exchange",
        "token_refresh",
        "token_revocation",
        "session_revoked",
        "all_sessions_revoked",
        "user_created",
        "user_updated",
        "user_suspended",
        "user_deleted",
        "validation_failed",
        "registration_denied",
        "provider_error",
        "unauthorized",
        "throttle_exceeded",
        "refresh_token_reuse",
        "missing_credential",
        "invalid_credential",
        "not_configured",
        "store_error"
      ]
    }
  }
}
```

---

## Implementation notes

The two deltas are independent; either order works. G1 first maximises diagnostic value
while G2 is built.

```
1. G1  crates/server/src/telemetry.rs:22-47 — .init() (37-40) → .try_init(); already-set
       → Ok(()) + tracing::debug!, skipping the fallback warning on that path. Companion
       double-init integration test in crates/server/tests/.
2. G1  crates/ffi/src/lib.rs:102-184 — init_telemetry(&config.telemetry) after the
       base-path override (~line 126), before Runtime::new/build_service; residual error →
       FfiError { code: "SERVICE_ERROR" }. New per-process integration tests
       crates/ffi/tests/{telemetry_install.rs, telemetry_host_respect.rs}.
3. G2  crates/core/src/domain/audit.rs:56-81 (+StoreError on AuditEventType), 360-376
       (+StoreError on AuditFailure).
4. G2  crates/core/src/service/exchange.rs:158-162 — assemble via create_audit_event
       (service/mod.rs:368-391) with detail.store_detail, emit via emit_audit
       (mod.rs:255-274), discard the emission result, return the original error.
5. G2  schemas/datamodel.schema.json:69,85 (+store_error in both enums);
       crates/core/tests/datamodel_schema_mirror.rs (add the variant to all_event_types
       and all_failures — compile-enforced). New flow tests beside
       crates/core/tests/exchange.rs.
```

References: the exchange wrapper's terminal-emission pattern
(`crates/core/src/service/exchange.rs:123-221`) bounds where the new emission sits; the
best-effort channel contract is `emit_audit` (`crates/core/src/service/mod.rs:255-274`)
with `log_audit_fallback` (`mod.rs:348-355`); the embedded fixture shape for the FFI tests
is `crates/ffi/src/lib.rs:628-670`; the napi constructor confirming no binding change is
needed is `bindings/nodejs/src/lib.rs:78-100`. GitHub issue #47 is the incident record.

---

## Compatibility and migration

- **G1 changes what embedded processes write to stdout.** JSON `tracing` lines (default
  `info` filter, `RUST_LOG`-controlled) now interleave with the stdout audit adapter's
  JSON lines on the host's stdout. Both are line-delimited JSON and the audit event shape
  is unchanged, but a consumer that assumed every stdout line is an audit event must key
  on the audit shape (e.g. the `event_type` field) rather than on line position.
  Operators wanting the old silence can set `RUST_LOG=off` — silence becomes an explicit
  choice instead of a defect.
- **G1 ordering note for Rust hosts.** A host embedding `oidc-exchange-ffi` directly that
  wants its own global subscriber must install it *before* constructing `OidcExchange`;
  afterwards the FFI's subscriber is already the global one. Node/Python/Lambda hosts
  cannot install a Rust global subscriber, so this affects only Rust embedders.
- **G1 changes no API surface.** No binding signatures, no new exports, no npm/PyPI
  packaging changes beyond rebuilt binaries.
- **G2 adds one enum value to the published audit vocabulary.** `store_error` joins
  `event_type` and `outcome.reason` in `schemas/datamodel.schema.json`; SIEM consumers
  validating against the old enums must take the updated schema. Existing event shapes
  and values are untouched, and no event moves channels.
- **G2 emits only on a path that previously emitted nothing**, so no alerting rule keyed
  on existing events changes behaviour; new alerting can key on
  `event_type == "store_error"`.

---

## Merge plan

1. Apply the nine `Proposed changes` blocks to `07-telemetry-and-audit.md`,
   `03-service-flows.md`, `01-domain-model.md`, and `01-ffi-core.md`; bump each page's
   `**Date:**` to the merge date.
2. Coordinate the shared pens before applying mechanically:
   - `03-service-flows.md` → Audit emission and `07-telemetry-and-audit.md` → Audit are
     also rewritten by the pending
     [`2026-08-25-close_r2_audit_code_divergences.md`](2026-08-25-close_r2_audit_code_divergences.md)
     (the refresh flow's Debug-level `ValidationFailed` exception). Whichever spec merges
     second must re-verify both paragraphs against `service/mod.rs` and compose the two
     qualifications — the refresh exception and the exchange `StoreError` operational
     record — rather than overwrite one with the other.
   - `07-telemetry-and-audit.md` → Telemetry's exporter list is rewritten by the pending
     [`2026-06-24-complete_telemetry_exporters.md`](2026-06-24-complete_telemetry_exporters.md)
     and gains a `prometheus` row from the R2 spec; this change's two Telemetry blocks
     touch only the flanking paragraphs, but the merger must confirm the list between
     them still matches `init_telemetry` after all pens have landed. The 2026-06-24 pen
     is wider than the list: its quoted block ends with a trailing sentence —
     "Instrumentation continues to use `tracing`; the server bridges spans to OTEL with
     a `tracing-opentelemetry` layer installed at startup." — whose server-only,
     at-startup framing the second merger must compose with this change's
     dual-entrypoint opening paragraph (and reconcile against the FFI-flush Open
     question) rather than apply verbatim.
3. Fold the `Type changes` fragment into
   `.specs/service/specs/canonical-types.schema.json`: replace the `AuditEventType` `$def`
   wholesale with the fragment's; drop the change-tracking `$comment` on the way in.
4. Verify the None rows of `Affected spec pages` hold against the code (the bindings pages
   still describe marshalling only; `00-overview.md`'s best-effort operational goal reads
   true of the shipped `StoreError` event).
5. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, move it to
   `.specs/changes/merged/`, and update `.specs/README.md` (move this spec's row from
   pending to merged).

---

## Assumptions and open questions

### Assumptions

- Verified on this branch: `init_telemetry` has exactly one caller
  (`crates/server/src/main.rs:29`); no subscriber install exists in `crates/ffi` or any
  binding; `tracing-subscriber` is dev-only in `crates/ffi/Cargo.toml`; the exchange
  `StoreError` arm (`exchange.rs:162`) emits nothing; the embedded runtime executes the
  same router and error mapping as the server, so `error.rs:103-104` fires (into the
  void) on every embedded 5xx today.
- `tracing::dispatcher::has_been_set()` is available on the pinned `tracing` 0.1 for the
  install test, and `tracing_subscriber`'s `try_init` reports an already-set global
  dispatcher as its error case.
- The pending R2 change spec (2026-08-25) is implemented in code but not yet merged into
  the canonical pages; this spec's `Proposed changes` blocks are written against the
  canonical pages as committed, with composition handled in the Merge plan.
- `StoreError.detail` strings are adapter-composed diagnostics (store/SDK errors), not
  client credentials — the same strings `error.rs:104` already logs — so carrying one in
  an operator-facing audit event introduces no new exposure class; the client-facing body
  remains `internal server error`.

### Decisions

- *Install at construction, not a host-called hook.* **The FFI constructor installs the
  subscriber.** The issue offered both designs; a hook that hosts must call at cold start
  recreates the silent-discard failure for every host that forgets, and there is no
  correct moment other than construction. `try_init` keeps constructor-install safe for
  hosts that already own a subscriber.
- *Reuse `init_telemetry`; no dependency promotion.* **The FFI calls the server crate's
  function; `tracing-subscriber` is not added to `crates/ffi`.** The issue suggested
  promoting the dependency, but the FFI already depends on the server crate that owns the
  telemetry pipeline; a second install path is exactly the divergence the config
  pipeline's "one resolve" rule exists to prevent.
- *`try_init`, first-wins, host-respecting.* **An already-set dispatcher is retained and
  the call reports success.** Fighting the host (or panicking, as `.init()` would) turns
  a diagnostic improvement into a crash; first-wins matches `tracing`'s own global-default
  model. Consequence: with multiple instances, the first constructor's `[telemetry]`
  config governs — recorded on `01-ffi-core.md` as the one piece of process-global state.
- *Exporter warning only on actual install.* **The retained-dispatcher path skips the
  fallback warning.** The warning claims "falling back to stdout JSON"; when a host's own
  subscriber is retained, that claim would be false — the host's pipeline, not the
  fallback, receives the events.
- *Store fault goes best-effort operational, never mandatory.* **`emit_audit`, not
  `emit_security_event`.** Three reasons: the failing store may be the audit dependency
  itself (the original comment's insight — a mandatory emission would then predictably
  fail too); under `durability = "enforce"` the mandatory path would replace the
  `StoreError` with `SecurityAuditDurability`, masking the root cause behind a different
  infrastructure error; and `SecurityEvent` is spec'd as the closed set of
  security-relevant outcomes — admitting an infrastructure fault would dilute the
  mandatory-channel invariant `00-overview.md` quantifies over. `Error` severity survives
  the committed `emit_threshold = "info"`, so the default deployment always records it.
- *Emission failure is discarded.* **The original `StoreError` always reaches the
  caller.** `emit_audit`'s `blocking_threshold` contract exists to keep security-relevant
  operational events from succeeding unaudited; this request is already failing, and its
  diagnostic value lives in the error it returns. The discard is safe because
  `emit_audit` has already logged the serialized event through `log_audit_fallback`
  before returning the error — with G1, that fallback line is visible in embedded
  deployments too.
- *`detail.store_detail` carries the diagnostic.* **The `StoreError` `detail` string is
  attached to the event.** The motivating incident's missing signal *was* this string; an
  event that says only "a store failed" would send the operator back to the logs the
  original bug proved might not exist. The key is namespaced (`store_detail`, not
  `detail`) so the JSON does not read `"detail":{"detail":…}`.
- *`actor` is `None`.* **No principal is named on the store-fault event.** The wrapper arm
  sees only the typed error; a store fault can precede identity establishment, and
  threading a sometimes-known user id through `ExchangeFlowError::Other` would make the
  field's meaning depend on where the fault occurred. `provider`, `client_addr`, and
  `user_agent` carry the correlation.
- *Scope: the exchange flow only.* **Refresh and revoke store faults stay un-audited in
  this change.** The issue's subject is the exchange path's deliberate unaudited early
  return — the only place a comment justifies silence — and G1 already restores the
  shared 500/503 mapping log for every flow in every deployment shape. Extending the
  operational record is mechanical once this shape exists, and doing it here would triple
  the test surface of a targeted fix; it is raised as an Open question instead.
- *Republished-list completeness.* **The `AuditEventType` prose list and sidecar `$def`
  this change republishes also pick up `MissingCredential`, `InvalidCredential`, and
  `NotConfigured`; nothing else of the sidecar backlog folds in.** The same rule the R2
  spec applied to `ExchangeRequest`: these variants have shipped, this change has the pen
  on exactly that list and that enum, and re-omitting them would republish a known
  falsehood. The rest of the backlog (`AuditEvent.operator`, `OperatorPrincipal`) stays
  with the deferred doc pass.
- *Bindings pages untouched.* **`02-nodejs.md`, `03-python.md`, and `04-lambda.md` get no
  blocks.** The binding layers change no code and their pages claim nothing the install
  falsifies; the canonical home for the embedded telemetry contract is `01-ffi-core.md`
  plus `07-telemetry-and-audit.md`, which the binding pages already defer to.

### Open questions

- Should the operational `StoreError` record extend to the refresh and revoke flows (and
  the nonce mint's collision `StoreError`), now that the event type exists?
- When the OTLP/X-Ray exporters land
  ([`2026-06-24-complete_telemetry_exporters.md`](2026-06-24-complete_telemetry_exporters.md)),
  embedded Lambda deployments (the TS binding over `crates/ffi`) will have no
  per-invocation `flush_telemetry` seam — the server binary's Lambda mode flushes via
  `run_lambda`, but the FFI exposes no flush. Does the FFI need one before a buffering
  exporter ships?
