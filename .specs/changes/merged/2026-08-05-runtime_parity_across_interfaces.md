# Change: One owned request-normalisation boundary across the five runtime shapes

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** crates/server, crates/ffi, bindings/* (service, bindings)

Move request construction out of the host adapters and into one owned Rust normaliser. Today
the axum server, the Lambda runtime, the napi Node binding, and the PyO3 WSGI and ASGI
adapters each decide independently how a path is decoded, how a prefix is stripped, how
duplicate headers collapse, and how much body to buffer — five answers to one question, four
of them written in a language other than the one the router is written in. This change makes
the FFI accept the most faithful representation each host has (raw path bytes, a separate
query string, ordered header pairs, a bounded body) and construct the request exactly once in
Rust; adds a differential conformance corpus that every shape must satisfy in CI; bounds the
body before it is buffered on every host; and makes the FFI entry points total and async. The
async entry point is a breaking change to two published packages.

---

## Motivation

The scan bundle at
`.security/oidc-exchange/53cbdec9_20260804T102454Z/` records twelve request-shaping findings.
They are not one bug repeated. They are what happens when a control has no owner: the
repository already implements segment-aware prefix stripping correctly in
`crates/server/src/middleware/base_path.rs:148-160`, with a doc comment naming the naive
byte-prefix version as "exactly the bug this helper exists to rule out" — and
`bindings/lambda/src/adapters.ts` reimplements that control three times, once per event
source, each with the naive version. `_wsgi.py` and `_asgi.py` independently make the *same*
three mistakes (decoded-path resplicing, header-dict collapse, unbounded body accumulation),
which is the signature of two authors solving one under-specified problem separately.

Three of the defects are reachable by an unauthenticated request. `handle_request_sync`
asserts `!path.is_empty()` (`bindings/python/src/lib.rs:88-89`); PEP 3333 permits an empty
`PATH_INFO` for an application mounted at a root, so one ordinary GET panics the extension
and takes the worker with it — repeatably, and unrecorded, because `audit.adapter` defaults
to `noop` (`crates/core/src/config.rs:217`). `handleRequest` is synchronous and `block_on`s
the whole request (`bindings/nodejs/src/lib.rs:66-91`), so an anonymous request holds a host
thread for the duration of a ten-second upstream provider call — and that is the shape every
documented Node integration uses. Both Python adapters buffer the entire body with no cap
(`_asgi.py:27-32`, `_wsgi.py:23-24`), so the router's own limit is evaluated only after the
host has already paid the memory cost, which removes the bound rather than deferring it. The
structural fix is the one the hardening proposal recommends
(`hardening/proposals/request-normalisation-boundary.md`, Option 2 with Option 3's corpus):
change what the FFI signature asks for, so the adapters stop having a reason to touch the
request at all.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/bindings/specs/01-ffi-core.md`](../bindings/specs/01-ffi-core.md) | Rewrite the public API and request flow around `WireRequest` and an async, total `handle`; add the normalisation contract and limits |
| [`.specs/bindings/specs/00-overview.md`](../bindings/specs/00-overview.md) | Update the shape diagram; retire the `block_on`-on-a-host-thread assumption; add the conformance-corpus decision |
| [`.specs/bindings/specs/02-nodejs.md`](../bindings/specs/02-nodejs.md) | `handleRequest` returns a `Promise`; `handleRequestSync` is deprecated; request shape gains `rawPath`/`query` |
| [`.specs/bindings/specs/03-python.md`](../bindings/specs/03-python.md) | Ordered header sequence replaces the header dict; raw path and query are separate; typed errors replace the assertions; body cap in both adapters |
| [`.specs/bindings/specs/04-lambda.md`](../bindings/specs/04-lambda.md) | Event adapters lose base-path stripping and query splicing; they become pure event-shape translation |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) | Record the breaking-change version step and the conformance CI job |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | Middleware stack: two-guard panic containment and the body limit layer |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | Sharpen `base_path` (segment-boundary strip, embedded runtimes, load-time normalisation); add `server.max_request_body_bytes` to `[server]` and the Defaults summary |
| [`.specs/canonical-types.schema.json`](../canonical-types.schema.json) | `HttpRequest` becomes the wire request (`rawPath` + `query`); add `NormalisationLimits` |

No new canonical page. The conformance corpus is documented inside
[`01-ffi-core.md`](../bindings/specs/01-ffi-core.md) rather than given a page of its own.

---

## Proposed changes

### `.specs/bindings/specs/01-ffi-core.md` → Responsibilities (Modify)

> - Build an `AppService` and axum `Router` from a TOML config (re-using `crates/server`'s
>   bootstrap), and own the tokio `Runtime` that drives them. Config supplied through `new`
>   (inline string) or `from_file` passes the same load-time validation as the server's
>   `load_config` (role, TTLs, allowlist, internal-API secret —
>   [06-configuration.md](../../service/specs/06-configuration.md) → Validation at load);
>   invalid config is rejected as an `FfiError` at construction, never at request time.
> - **Own request normalisation.** Turn the most faithful representation a host can supply —
>   raw percent-encoded path bytes, the query string separately, an ordered list of header
>   pairs, and a bounded body — into the `http::Request` the router routes. Percent-decoding,
>   base-path stripping, header ordering, and body bounding all happen here and nowhere else.
> - Publish the limits a host must respect before it buffers anything.
> - Be total: no host input produces a panic across the boundary. Shaping failures become the
>   same HTTP response the native server produces for the same wire bytes; only boundary
>   failures with no HTTP meaning surface as `FfiError`.

### `.specs/bindings/specs/01-ffi-core.md` → Public API (Modify)

> ```rust
> pub struct OidcExchange { /* runtime + router + limits */ }
>
> pub struct WireRequest {
>     pub method: String,              // token as received; validated here
>     pub raw_path: Vec<u8>,           // still percent-encoded, no query, no fragment
>     pub query: Option<Vec<u8>>,      // still percent-encoded, no leading '?'
>     pub headers: Vec<(String, String)>,  // ordered; duplicates preserved in wire order
>     pub body: Vec<u8>,               // already bounded by the host against `limits()`
>     pub hints: TransportHints,
> }
>
> pub struct TransportHints {
>     /// False when the host could only supply an already-decoded path (a WSGI server
>     /// exposing neither `RAW_URI` nor `REQUEST_URI`), so its parity claim is qualified.
>     pub path_is_raw: bool,
> }
>
> pub struct NormalisationLimits { pub max_body_bytes: u64 }
>
> impl OidcExchange {
>     pub fn new(config_toml: &str) -> Result<Self, FfiError>;
>     pub fn from_file(path: &str) -> Result<Self, FfiError>;
>
>     pub fn limits(&self) -> NormalisationLimits;
>
>     /// Total. Every `WireRequest` yields a response; `FfiError` is reserved for boundary
>     /// failures with no HTTP meaning (a caught panic, a poisoned runtime).
>     pub async fn handle(&self, request: WireRequest) -> Result<FfiResponse, FfiError>;
> }
>
> pub struct FfiResponse { pub status: u16, pub headers: Vec<(String, String)>, pub body: Vec<u8> }
> pub struct FfiError    { pub code: String, pub message: String }   // impl Error + Display
> ```
>
> `handle_request(method, path, headers, body)` remains as a deprecated synchronous shim that
> splits `path` on the first `?`, marks `path_is_raw: false`, and forwards to `handle` on the
> owned runtime. It is removed one major cycle after this change ships.

### `.specs/bindings/specs/01-ffi-core.md` → Request flow (Modify)

> `handle` normalises in a fixed order, and every step is fallible rather than panicking:
>
> 1. **Method** — parsed with `http::Method::from_str`; an invalid token yields `400`.
> 2. **Path** — `raw_path` is validated as origin-form and left percent-encoded. An empty
>    path normalises to `/` (PEP 3333 permits an empty `PATH_INFO` for a root-mounted
>    application). A path that is not origin-form — no leading `/`, or an authority or scheme
>    where a path belongs — yields `400`, never an absolute-form URI carrying a
>    client-chosen authority.
> 3. **Query** — attached as-is; never concatenated by a host, so an encoded `?` or `#` in a
>    path segment stays a literal octet instead of becoming URI structure.
> 4. **Base path** — stripped by `crates/server`'s `strip_prefix_at_segment_boundary`, the
>    single implementation, at a path-segment boundary. `/authorize` is never mangled by a
>    `/auth` prefix.
> 5. **Headers** — inserted in wire order with `HeaderMap::append`, so duplicates survive and
>    first-wins consumers read what the native server reads. A header name or value the
>    `http` crate rejects is dropped and counted, not fatal.
> 6. **Body** — rejected with `413` above `limits().max_body_bytes`; the router additionally
>    carries `DefaultBodyLimit` set from the same value, so one number bounds all five shapes.
>
> The router is cloned per request and awaited on the owned runtime. Every response body is
> collected into `FfiResponse`.

### `.specs/bindings/specs/01-ffi-core.md` → Panic containment (Add)

> Three guards, innermost outward. `CatchPanicLayer` stays nearest the handler so a caught
> handler panic still passes back out through `request_id_layer` and carries its
> `x-request-id`. A second `CatchPanicLayer` wraps the assembled router *and* the base-path
> service, so a panic in the request-id, timeout, audit-context, or base-path layers is
> contained too. `handle` then wraps its own body in `catch_unwind` and converts an escaped
> unwind into `FfiError { code: "PANIC" }`, because the napi trampoline's behaviour under an
> unwind is not established for this build (see Open questions). No `assert!`, `unwrap`, or
> `expect` runs on host-supplied input anywhere on this path.

### `.specs/bindings/specs/01-ffi-core.md` → Conformance corpus (Add)

> `conformance/corpus/` holds transport-agnostic request fixtures and the normalised request
> each must produce. Four runners replay every fixture — the native hyper server, the FFI
> directly, the Node binding (plus the Lambda adapter over synthesised events), and the
> Python WSGI and ASGI adapters under a pinned server — and assert that method, decoded path,
> query, ordered headers, and status agree across all five shapes. The corpus covers `%2F`
> and `%2E%2E` in a path segment, an encoded `?` and `#`, duplicate `X-Forwarded-For` in both
> orders, `/authorize` and `/auth/keys` as siblings of a `/auth` base path, an empty
> `PATH_INFO`, a non-numeric `CONTENT_LENGTH`, a `CONTENT_LENGTH` of `sys.maxsize`, and a
> body one byte over the cap. A fixture whose expectation a shape cannot meet records the
> reason as a declared qualification against that shape's `TransportHints`, not as a skip.
> The `conformance` CI job is a merge gate.

### `.specs/bindings/specs/00-overview.md` → Shape (Modify)

> ```
> crates/ffi (OidcExchange: new/from_file + limits + async handle → the one normaliser)
>    │      ▲ hosts hand over raw path bytes, query, ordered headers, bounded body
>    │      │ they perform no decoding, stripping, or deduplication
>    ├── bindings/nodejs   (napi-rs)   → @oidc-exchange/node     (class OidcExchange)
>    │       └── bindings/lambda (TS)  → @oidc-exchange/lambda   (createHandler)
>    └── bindings/python   (PyO3)      → oidc-exchange (PyPI)    (class OidcExchange + ASGI/WSGI)
> ```
>
> Each binding translates its host's request shape into a `WireRequest` and the `FfiResponse`
> back. It does not construct a URI, strip a prefix, collapse headers, or decide a body size —
> those are the normaliser's, and the conformance corpus asserts every shape agrees.

### `.specs/bindings/specs/00-overview.md` → Assumptions (Modify)

> - The host calls the async entry point from its own runtime; the FFI never holds a host
>   thread for the duration of network I/O. Callers on the deprecated synchronous entry point
>   still block the calling thread and must not call it from an event-loop thread.

### `.specs/bindings/specs/00-overview.md` → Decisions (Add)

> - *Parity is tested, not assumed.* **A conformance corpus replays shared fixtures through
>   all five runtime shapes in CI and asserts they agree**
>   ([01-ffi-core.md](01-ffi-core.md) → Conformance corpus). Agreement between five
>   implementations is not a property any one of them can establish alone; the corpus is the
>   merge gate that keeps them in step.

### `.specs/bindings/specs/02-nodejs.md` → API (Modify)

> ```typescript
> interface HeaderEntry { name: string; value: string }
> interface HttpRequest  {
>   method: string;
>   rawPath: string;        // still percent-encoded, no query
>   query?: string;         // still percent-encoded, no leading '?'
>   headers: HeaderEntry[]; // ordered; duplicates preserved
>   body?: Buffer;
>   pathIsRaw?: boolean;    // default true; false when the source pre-decoded the path
> }
> interface HttpResponse { status: number; headers: HeaderEntry[]; body: Buffer }
> interface OidcExchangeOptions { config?: string; configString?: string; basePath?: string }
> interface Limits { maxBodyBytes: number }
>
> class OidcExchange {
>   constructor(options: OidcExchangeOptions);
>   limits(): Limits;
>   handleRequest(request: HttpRequest): Promise<HttpResponse>;  // async
>   handleRequestSync(request: HttpRequest): HttpResponse;       // deprecated
>   shutdown(): void;                                            // no-op
> }
> ```
>
> `handleRequest` is **asynchronous**: it returns a `Promise` backed by a napi async task, so
> no host thread is held while the router awaits an upstream provider. `handleRequestSync`
> preserves the previous blocking behaviour for callers with genuinely synchronous
> architectures; it logs a deprecation warning once per process and is removed one major cycle
> after this change ships. Both are total — a malformed request yields an `HttpResponse` with
> the status the native server would return, never a thrown `REQUEST_BUILD_ERROR`. `basePath`
> overrides `server.base_path` in the instance's config at construction, so the segment-aware
> strip runs in the normaliser and no path handling happens in JavaScript.

### `.specs/bindings/specs/02-nodejs.md` → Decisions (Modify)

> - *Asynchronous `handleRequest`.* **The method returns a `Promise` backed by a napi async
>   task.** The previous synchronous surface `block_on`ed the whole request, so a slow
>   upstream call held a host thread — the availability cost fell on every request the host
>   was serving, not on the one that was slow. The sync variant survives, deprecated, for one
>   major cycle.

### `.specs/bindings/specs/03-python.md` → API (Modify)

> ```python
> class OidcExchange:
>     def __init__(self, *, config: str | None = None, config_string: str | None = None) -> None: ...
>     def limits(self) -> dict[str, int]: ...
>     def handle_request_sync(self, request: dict[str, Any]) -> dict[str, Any]: ...
>     async def handle_request(self, request: dict[str, Any]) -> dict[str, Any]: ...
>     def asgi_app(self) -> Any: ...
>     def wsgi_app(self) -> Any: ...
>     def shutdown(self) -> None: ...
> ```
>
> A request `dict` is
> `{ "method": str, "raw_path": bytes, "query": bytes | None, "headers": Sequence[tuple[str, str]], "body": bytes, "path_is_raw": bool }`.
> `headers` is an **ordered sequence of pairs**, not a mapping: HTTP header names are not
> unique, and a name-keyed dict silently picked last-wins where the router reads first. The
> response `dict` is `{ "status": int, "headers": list[tuple[str, str]], "body": bytes }`.
> Malformed input raises `ValueError`; nothing on this path asserts.

### `.specs/bindings/specs/03-python.md` → Implementation (Modify)

> - **Rust (`src/lib.rs`)** — a `#[pyclass] OidcExchange` whose `#[new]` constructor calls FFI
>   `from_file`/`new`. `handle_request_sync` extracts the method, raw path, query, ordered
>   header pairs, and body from the request mapping, returning `PyValueError` for any missing
>   or ill-typed field — an empty path is valid input and normalises to `/`, not a panic. It
>   releases the GIL (`py.allow_threads`) around the blocking FFI call and re-acquires it to
>   build the result dict. `handle_request` is a native coroutine over the same normaliser via
>   `pyo3-async-runtimes`, so the event loop is not occupied by an executor thread. `shutdown`
>   is a no-op.
> - **Python (`python/oidc_exchange/`)** — `_asgi.py` (`make_asgi_app`) forwards
>   `scope["raw_path"]` when present (falling back to `scope["path"]` with
>   `path_is_raw=False`), keeps `scope["query_string"]` separate, passes `scope["headers"]` as
>   ordered pairs, and accumulates the body into a `bytearray` that stops at
>   `limits()["max_body_bytes"]` — the request is refused with `413` before the host allocates
>   past the cap. `_wsgi.py` (`make_wsgi_app`) prefers `RAW_URI` or `REQUEST_URI` for the raw
>   path and falls back to re-encoding `PATH_INFO` with `path_is_raw=False`; it parses
>   `CONTENT_LENGTH` defensively (a non-numeric value is `400` — the status the native server
>   gives a malformed `Content-Length` — a value beyond the cap is `413`, and neither is ever
>   an unhandled `ValueError` or `OverflowError`) and reads `wsgi.input` in capped
>   chunks. Neither adapter builds a URI, strips a prefix, or collapses a header.

### `.specs/bindings/specs/03-python.md` → Decisions (Modify)

> - *Ordered header pairs, not a dict.* **The request and response header surfaces are
>   sequences of `(name, value)` tuples.** A mapping cannot represent repeated headers, and
>   collapsing them made the embedded shape disagree with the native server on
>   `X-Forwarded-For` and `Set-Cookie`. This is the change that required the PyO3 signature to
>   move off `PyDict`.
> - *Native coroutine, not an executor hop.* **`handle_request` is a real coroutine over the
>   async FFI.** The executor design existed because the FFI was blocking; once it is not,
>   the hop is pure latency.

### `.specs/bindings/specs/04-lambda.md` → Responsibilities (Modify)

> - Detect the incoming event shape (API Gateway REST v1, HTTP API v2 / Function URL, or ALB).
> - Translate it to the Node binding's `HttpRequest` — event field to request field, and
>   nothing more. The adapter performs no path stripping, no query re-encoding, and no header
>   deduplication; those belong to the normaliser, which already implements them once and
>   correctly.

### `.specs/bindings/specs/04-lambda.md` → Event adapters (Modify)

> - **Detection** — `isApiGatewayV1` (`httpMethod` + `resource`, no `version`), `isApiGatewayV2`
>   (`version === "2.0"`), `isAlbEvent` (`requestContext.elb`).
> - **Translation** — `fromApiGatewayV1`, `fromApiGatewayV2`, `fromAlbEvent` each read the
>   rawest path the event carries (`rawPath` for v2, `path` for v1 and ALB, with
>   `pathIsRaw: false` for the two sources that pre-decode), pass the query string through
>   unmodified (`rawQueryString` for v2; `multiValueQueryStringParameters` re-encoded once for
>   v1 and ALB), emit headers as ordered pairs preserving multi-value entries, and base64-decode
>   the body when `isBase64Encoded`.
> - **Base path** — `createHandler`'s `basePath` option is forwarded to the FFI instance and
>   applied by `crates/server`'s segment-aware strip. The adapter no longer contains a strip of
>   its own, so `/authorize` under `basePath: "/auth"` routes exactly as it does on the
>   standalone server: a clean `404`, not a mangled `orize` or a `502` from an uncaught request
>   build error.
> - **Helpers** — `decodeBody(body, isBase64Encoded)` decodes base64 or UTF-8 and refuses a
>   body above `limits().maxBodyBytes` with a `413` before it is handed across the boundary.

### `.specs/bindings/specs/04-lambda.md` → Decisions (Modify)

> - *No control logic in the adapter.* **Prefix stripping, path decoding, and header handling
>   live in Rust; the adapter only maps event fields.** Three hand-maintained copies of one
>   control drifted from the Rust original that they were copied from; the way to keep them in
>   step is for them not to exist.

### `.specs/bindings/specs/05-distribution.md` → Version parity (Modify)

> One version string must match across `Cargo.toml` `workspace.package.version`,
> `bindings/nodejs/package.json`, and `bindings/python/pyproject.toml`. The `validate` job
> checks this before building. Bumps are manual: edit the three files, commit, tag, push. npm
> and PyPI use the bare `X.Y.Z`; GitHub and Docker use the `v`-prefixed tag. Because the three
> artifacts share one version, a breaking change to the FFI surface bumps all of them together
> — under `0.x` that is a minor bump (`0.2.x` → `0.3.0`), and the release notes name the two
> packages whose API changed (`@oidc-exchange/node`, `oidc-exchange` on PyPI) and the migration
> for each.

### `.specs/bindings/specs/05-distribution.md` → Assumptions (Add)

> - The `conformance` CI job has Rust, Node, and Python toolchains available in one runner; it
>   is a required check on the default branch.

### `.specs/service/specs/04-http-api.md` → Middleware stack (Modify)

> Applied to the router (`routes/mod.rs`), outermost first:
>
> 1. **Outer catch-panic** (`middleware/error_handler.rs`, tower `CatchPanicLayer`) — wraps
>    the base-path service and everything inside it, so a panic in any layer becomes
>    `500 {"error":"server_error","error_description":"internal server error"}` instead of a
>    dropped connection or an unwind into an embedding host. It is the outermost guard, not a
>    move of the inner one: moving the single guard outward would cost a caught handler panic
>    its `x-request-id`, so the stack carries two.
> 2. **Base-path strip** (`middleware/base_path.rs`) — strip `server.base_path` at a
>    path-segment boundary before the routing decision. `base_path` is normalised at config
>    load, so the middleware never sees `""` or `"/"`, and no assertion runs on a request path.
> 3. **Request ID** (`middleware/request_id.rs`) — reuse `X-Request-Id` or generate a UUIDv4;
>    open a per-request `info_span` carrying `request_id` so all downstream logs — including
>    the `server_error` detail log — inherit it; echo in the response header.
> 4. **Request timeout** (`tower_http::timeout::TimeoutLayer`) — abort any request that runs
>    longer than `server.request_timeout` (default `30s`) and respond `408`. Sits inside the
>    request-id layer, so a timeout response still carries the request id, and outside the
>    rest of the stack, so the bound covers the remaining middleware and the handler.
> 5. **Body limit** (`axum::extract::DefaultBodyLimit`) — reject a request body above
>    `server.max_request_body_bytes` with `413`. Embedded hosts enforce the same number before
>    they buffer, so one configured value bounds all five runtime shapes.
> 6. **Audit context** (`middleware/audit_context.rs`) — extract `X-Forwarded-For`,
>    `User-Agent`, `X-Device-Id` into an `AuditContext` request extension, which the `/token`
>    and `/revoke` handlers pass into the core request structs so the stored session records
>    `ip_address`/`user_agent`/`device_id` and audit events record `ip_address`/`user_agent`.
> 7. **Inner catch-panic** (`middleware/error_handler.rs`, tower `CatchPanicLayer`) — nearest
>    the handler, so a caught handler panic still passes back out through the request-id layer
>    and its response carries `x-request-id`.
>
> Internal routes mount only when `internal_api.enabled = true` and the role is `admin` or
> `all`; with the flag false no internal routes are mounted regardless of role. When mounted,
> they additionally pass through **internal auth** (`middleware/internal_auth.rs`):
> `Authorization: Bearer <secret>` compared to `internal_api.shared_secret` in constant time
> (`subtle`); missing/wrong → `401`. A missing or empty secret is rejected at startup, never
> discovered at request time.

### `.specs/service/specs/06-configuration.md` → Sections → `[server]` (Modify)

> `host` (`0.0.0.0`), `port` (`8080`), `issuer` (the `iss` claim / discovery issuer, default
> empty), `role` (`all` | `exchange` | `admin`, default `all`), `request_timeout` (humantime
> duration string like the token TTLs, default `"30s"`) — the per-request timeout the server's
> timeout layer enforces; `base_path` (optional, default unset — a leading prefix such as
> `/prod` stripped from incoming request paths at a segment boundary before routing, honored in
> server, Lambda, and every embedded runtime); `max_request_body_bytes` (default `2097152`) —
> the request body ceiling the server's body-limit layer enforces and every binding enforces
> before it buffers.
>
> `base_path` is normalised and validated at config load: an empty string and `"/"` both
> resolve to unset, a value not starting with `/` is a startup error, and a trailing `/` is
> trimmed. Validating once at startup is what lets the per-request path be free of assertions.

### `.specs/service/specs/06-configuration.md` → Defaults summary (Modify)

> | `server.host` / `port` / `role` / `request_timeout` / `max_request_body_bytes` | `0.0.0.0` / `8080` / `all` / `"30s"` / `2097152` |

---

## Type changes

`HttpRequest` becomes the wire request: `path` (a spliced, host-built string) is replaced by
`rawPath` and `query`, which the normaliser combines. `headers` is unchanged — it was already
specified as an ordered array whose "repeated headers appear as repeated entries", which the
Python binding's `dict[str, str]` surface never honoured. `NormalisationLimits` is new.
`HttpResponse` is unchanged.

```json
{
  "$comment": "Fragment for 2026-08-05-runtime_parity_across_interfaces. Folds into .specs/canonical-types.schema.json on merge, replacing the existing HttpRequest $def.",
  "$defs": {
    "HttpRequest": {
      "type": "object",
      "description": "Transport-agnostic wire request handed to the FFI normaliser by every binding. The host performs no percent-decoding, prefix stripping, or header deduplication.",
      "required": ["method", "rawPath", "headers"],
      "properties": {
        "method": { "$ref": "#/$defs/NonEmptyString", "description": "HTTP method token as received." },
        "rawPath": {
          "type": "string",
          "description": "Still percent-encoded origin-form path, without query or fragment. An empty string normalises to '/'."
        },
        "query": {
          "type": "string",
          "description": "Still percent-encoded query string with no leading '?'. Absent when the request carries none; never concatenated onto rawPath by a host."
        },
        "headers": {
          "type": "array",
          "description": "Header name/value pairs in wire order. Repeated headers appear as repeated entries and are preserved through normalisation.",
          "items": {
            "type": "object",
            "required": ["name", "value"],
            "properties": {
              "name": { "type": "string" },
              "value": { "type": "string" }
            }
          }
        },
        "body": {
          "type": "string",
          "contentEncoding": "base64",
          "description": "Raw request body as bytes (modelled as base64 in JSON). Optional. Bounded by NormalisationLimits.maxBodyBytes before the host buffers it."
        },
        "pathIsRaw": {
          "type": "boolean",
          "default": true,
          "description": "False when the host could supply only an already-decoded path, qualifying that shape's parity claim in the conformance corpus."
        }
      }
    },
    "NormalisationLimits": {
      "type": "object",
      "description": "Limits the FFI core publishes so a host can refuse an oversized request before allocating for it.",
      "required": ["maxBodyBytes"],
      "properties": {
        "maxBodyBytes": {
          "type": "integer",
          "minimum": 0,
          "description": "Maximum request body in bytes, from server.max_request_body_bytes."
        }
      }
    }
  }
}
```

---

## Implementation notes

Order matters: steps 1–3 are non-breaking and remove request-triggerable panics from published
packages, so they ship first and independently of the FFI redesign.

```
1. Panic removal, non-breaking:
   - bindings/python/src/lib.rs:88-89 — delete both assert! calls; an empty path is valid
     input. Return PyValueError for a genuinely missing field (the get_item/ok_or_else
     path at :51-59 already does).
   - crates/server/src/middleware/base_path.rs:114-125,128-142 — replace the two panic!
     and two assert! postconditions with a fallible path that leaves the request unmodified
     and logs; :90's `filter(|p| !p.is_empty())` guard also folds in "/" .
   - crates/core/src/config.rs:149 — normalise base_path at load ("" and "/" -> None,
     trailing "/" trimmed, missing leading "/" -> startup error).
2. Two-guard containment:
   - crates/server/src/bootstrap.rs:352-362 — keep CatchPanicLayer as the first .layer()
     call (innermost, preserves x-request-id per the finding's correction #16); wrap the
     with_base_path_strip(...) result at :362 in a second CatchPanicLayer::custom(panic_handler).
   - Mirror the change in the test-only router at bootstrap.rs:1569-1575.
3. Body bounds before buffering:
   - crates/core/src/config.rs — add server.max_request_body_bytes (default 2 MiB, matching
     axum's implicit extractor default so nothing regresses).
   - crates/server/src/bootstrap.rs — layer DefaultBodyLimit::max(..) from that value.
   - bindings/python/python/oidc_exchange/_wsgi.py:23-24 — parse CONTENT_LENGTH inside
     try/except (ValueError, OverflowError): non-numeric -> 400 (native-server parity),
     beyond the cap -> 413; read wsgi.input in capped chunks.
   - bindings/python/python/oidc_exchange/_asgi.py:27-32 — accumulate into a bytearray and
     stop at the cap -> 413.
4. Conformance corpus, reporting mode:
   - conformance/corpus/*.json fixtures + expected normalised output; runners under
     crates/ffi/tests/, bindings/nodejs/__tests__/, bindings/lambda/__tests__/,
     bindings/python/tests/. Add a `conformance` job to .github/workflows/ci.yml alongside
     lint/test/nodejs-test/python-test/web-apps. It fails on day one — that is its purpose;
     record the baseline disagreements.
5. The normaliser (breaking):
   - crates/ffi/src/lib.rs:84-148 — add WireRequest, TransportHints, NormalisationLimits,
     limits(), and `async fn handle`. Reuse crates/server's strip_prefix_at_segment_boundary
     (export it as `pub` from crates/server, or lift it to a shared module) rather than
     writing a second strip. Wrap the body in catch_unwind. Keep handle_request as a
     deprecated shim.
   - Shaping failures return an FfiResponse carrying the same status the native server gives
     for the same wire bytes — this is what the corpus asserts, and it disposes of the
     REQUEST_BUILD_ERROR-as-thrown-exception class the Lambda findings surfaced.
6. Bindings, one at a time (ASGI first — scope["raw_path"] makes it cleanest):
   - bindings/nodejs/src/lib.rs:66-91 — handleRequest becomes an AsyncTask returning a
     Promise; add handleRequestSync (deprecated) and limits(). The napi dependency
     (bindings/nodejs/Cargo.toml) already enables "async"; evaluate adding "catch-unwind".
   - bindings/python/src/lib.rs:61-70 — replace the PyDict downcast with an ordered-sequence
     extraction; add a native coroutine via pyo3-async-runtimes.
   - bindings/python/python/oidc_exchange/{_asgi,_wsgi}.py — forward raw path + separate
     query + ordered headers; delete the f"{path}?{query}" splices at _asgi.py:38-41 and
     _wsgi.py:36-39.
   - bindings/lambda/src/adapters.ts:23-27,61-64,101-105 — delete all three strips; forward
     basePath to the FFI instance in src/index.ts:58-62.
7. Delete the shim-side stripping and decoding so the correct implementation is provably
   the only one; update the five examples/nodejs/* apps that call handleRequest
   (express, fastify, hono, nextjs, sveltekit) in the same release as the async change.
8. Promote the corpus to a merge gate; remove the deprecated entry points one major cycle later.
```

Evidence: finding write-ups under
`.security/oidc-exchange/53cbdec9_20260804T102454Z/findings/` —
`g3-pyo3-empty-path-assert-panic`, `g3-napi-sync-handle-request-event-loop-block`,
`g3-wsgi-unbounded-body-content-length`, `g3-asgi-unbounded-body-buffering`,
`g3-{wsgi,asgi}-decoded-path-uri-reconstruction`, `g3-{wsgi,asgi}-duplicate-header-collapse`,
`g3-lambda-basepath-boundary-{apigw-v1,apigw-v2,alb}`, `g2-catch-panic-layer-innermost`; and
`hardening/proposals/request-normalisation-boundary.md` for the option analysis this change
implements.

---

## Migration

The FFI signature change is breaking for two published packages. All three artifacts share
one version string ([05-distribution.md](../bindings/specs/05-distribution.md) → Version
parity), so they move together: `0.2.x` → `0.3.0`.

| Package | Break | Migration |
|---|---|---|
| `@oidc-exchange/node` | `handleRequest` returns `Promise<HttpResponse>`; `HttpRequest.path` becomes `rawPath` + `query` | `await` the call, or switch to `handleRequestSync` for one major cycle; split the path on the first `?` |
| `oidc-exchange` (PyPI) | request/response `headers` become ordered pair sequences; `path` becomes `raw_path` + `query` | Callers using `asgi_app()`/`wsgi_app()` are unaffected — the shipped adapters migrate with the release. Direct `handle_request_sync` callers convert their dicts |
| `@oidc-exchange/lambda` | none at the API surface | `createHandler({ basePath })` keeps working; the strip now happens in Rust, so sibling paths such as `/authorize` stop being mangled — a behaviour change operators should be told about in the release notes |

The deprecated synchronous entry points stay for one major cycle, keeping the current
behaviour (and the current blocking cost) for callers who do not migrate. The five Node
examples that call `handleRequest` update in the same release, so the documented pattern never
lags the recommended one.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page (`00-overview.md`,
   `01-ffi-core.md`, `02-nodejs.md`, `03-python.md`, `04-lambda.md`, `05-distribution.md`
   under `.specs/bindings/specs/`; `04-http-api.md` and `06-configuration.md` under
   `.specs/service/specs/`); bump each page's `**Date:**` to the merge date.
2. Fold the `Type changes` `$defs` into `.specs/canonical-types.schema.json`, replacing the
   existing `HttpRequest` and adding `NormalisationLimits`.
3. Remove the `01-ffi-core.md` Open question about `shutdown` only if the normaliser work
   settles it; otherwise leave it.
4. Flip this file's `**Status:**` to `Merged`, stamp `**Merged:** YYYY-MM-DD`, move it to
   `.specs/changes/merged/`.
5. Update `.specs/README.md`'s Change specs table.

---

## Assumptions and open questions

### Assumptions

- `pyo3-async-runtimes` supports the abi3-py310 build the wheels target
  ([03-python.md](../bindings/specs/03-python.md) → Distribution). If it does not, the
  executor hop stays and only the header/path/body changes land on the Python side.
- ASGI servers populate `scope["raw_path"]`; the ASGI spec makes it optional, so the adapter
  degrades to `scope["path"]` with `path_is_raw: false` rather than failing.
- No Python callbacks are invoked from inside the FFI call, so releasing the GIL across it
  remains sound.

### Decisions

- *One owned normaliser, not five careful adapters.* **Hosts hand over the rawest
  representation they have; Rust constructs the request.** Fixing each shim leaves three
  copies of the prefix strip and two of the path handling, with nothing testing that they
  agree — and the duplicate-header fix is not even representable without the signature change,
  because `bindings/python/src/lib.rs:62` downcasts `headers` to `PyDict`. The cost is paid
  either way; this pays it once and removes the place divergence can live.
- *A conformance corpus, not a review checklist.* **Parity is asserted by replaying shared
  fixtures through all five shapes in CI.** Agreement between five implementations is not a
  property any one of them can establish. The scan found a `502`-versus-`404` divergence that
  four discovery passes had not enumerated; a differential test finds that class on the first
  run.
- *Two panic guards, not one moved guard.* **`CatchPanicLayer` stays innermost and a second
  wraps the base-path service.** Moving the single guard outermost would strip `x-request-id`
  from a caught handler panic's response, trading one containment gap for a correlation
  regression.
- *Shaping failures are responses, not errors.* **A malformed path or an oversized body
  produces the status the native server produces, not an `FfiError` the host must interpret.**
  Parity is the point: an embedded host that returns `502` where the server returns `404` has
  the divergence this change exists to remove.
- *Limits published, not assumed.* **`limits()` exposes `max_request_body_bytes` so a host can
  refuse before it buffers.** A cap enforced only inside the router bounds the router's memory
  and not the host's, which is where the allocation actually happened.
- *A bounded buffer, not a streaming body handle.* **`WireRequest.body` is plain bytes the
  host has already capped against `limits()`.** The hardening proposal sketched a body handle
  read incrementally under the cap; publishing the limit and enforcing it at each host's
  accumulation point gives the same property — no allocation past the cap in any shape —
  without streaming plumbing across two FFI surfaces. Streaming remains available as a later,
  additive change.
- *Sync entry points deprecated, not deleted.* **Both survive one major cycle behind a
  warning.** Callers with genuinely synchronous architectures exist; removing their entry
  point in the same release as the async one turns a migration into a breakage.

### Open questions

- Does a Rust panic unwinding across the napi `extern "C"` trampoline abort the Node process,
  raise a JS exception, or is it undefined for this build? `bindings/nodejs/Cargo.toml` does
  not enable napi's `catch-unwind` feature, and no attacker-reachable panic outside
  `CatchPanicLayer` was demonstrated on the FFI path
  (`coverage.json` → `deferred_napi_panic_containment`). Closing it needs a forced outer-layer
  panic driven through the rebuilt addon with host exit behaviour recorded. The FFI-level
  `catch_unwind` narrows the question rather than answering it; whether to enable
  `catch-unwind` unconditionally depends on its measured overhead.
- Which WSGI and ASGI servers host the Python binding in practice, and do they drop underscore
  headers? `HTTP_*` environ construction collapses `X-Forwarded-For` and `X_Forwarded_For` onto
  one key, and the repository pins no server — gunicorn, waitress, and nginx drop underscore
  headers while permissive servers do not (`coverage.json` →
  `deferred_wsgi_header_collision`). The same answer determines which servers expose `RAW_URI`
  or `REQUEST_URI`, and therefore which shapes can make an unqualified parity claim. Pinning a
  supported set turns two deferred items into answerable ones; until then the WSGI shape's
  corpus results carry a `path_is_raw: false` qualification.
