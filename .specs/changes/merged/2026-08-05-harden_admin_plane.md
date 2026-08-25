# Change: Harden and separate the administrative plane

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** crates/server, apps/admin-ui (service, admin-ui)

Give the internal admin API a named operator principal in place of the anonymous shared
secret, bind it to its own listener so admin reachability is a deployment decision rather
than a consequence of the default role, throttle and record failed internal-auth attempts,
replace the five-name reserved-claim denylist with a closed set enforced at the write path,
bound the DynamoDB admin reads with cursor pagination and a maximum page size, and make the
console encode its route parameters and speak the service's own spelling of `UserStatus`.
This is Option 2 of
[`hardening/proposals/admin-plane-separation.md`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/hardening/proposals/admin-plane-separation.md);
Option 1's console fixes ship first and independently.

---

## Motivation

`/internal/*` is the privilege-assignment primitive of this service: `PUT`/`PATCH
/internal/users/{id}/claims` writes claims that `resolve_custom_claims` flattens into every
access token the service subsequently signs. Today the entire surface is guarded by
`internal_auth_layer` (`crates/server/src/middleware/internal_auth.rs:14-52`), which compares
one `Authorization: Bearer` value against one configured string. The comparison itself is
correct — `subtle::ConstantTimeEq` at `internal_auth.rs:55-63`, with an explicit empty-secret
rejection — but nothing surrounds it: no attempt counter, no lockout, a failure arm that
emits neither a `tracing` line nor an audit event, so guessing is unlimited and silent, and
no strength requirement on the string being guessed — `AppConfig::validate`
(`crates/core/src/config.rs:74-90`) requires only that the secret be non-empty, so a
one-character secret starts the service
(`g1-internal-shared-secret-brute-forceable`). The
`AuditEventType::Unauthorized` variant exists (`crates/core/src/domain/audit.rs:52`) and is
never constructed anywhere in the workspace. There is also no caller identity, so an admin
mutation records *that* someone knew the secret and never *who* acted. `build_router`
(`crates/server/src/bootstrap.rs:336-341`) merges `internal_routes` into the same router as
`public_routes` whenever `(role == "admin" || role == "all") && internal_api.enabled`, and
`server.role` defaults to `"all"` (`crates/core/src/config.rs:158`) — so turning the internal
API on without also changing the role serves the privilege-assignment primitive on the same
socket as `/token`.

Four smaller defects sit on the same surface and are cheapest to fix while it is open.
`RESERVED_CLAIMS` (`crates/core/src/service/claims.rs:8`) names five claims, so every other
registered protocol name — `nbf`, `jti`, `azp`, `scope`, `roles` — passes the filter and lands
in a signed token; the filter also runs only at token-build time, so a denylisted name is
still persisted on the user record and remains re-exportable through a `{{ user.claims.KEY }}`
template (`claims.rs:116-119`). `DynamoRepository::list_users`
(`crates/adapters/src/dynamo/mod.rs:593-633`) scans the whole table into memory and sorts it
before slicing out the requested window, and `count_by_status` (`dynamo/mod.rs:553-…`) plus
`count_active_sessions` (`dynamo/mod.rs:686-…`) drive two more full-table walks on every
`GET /internal/stats`, which is what a dashboard calls on each page load. The console
interpolates `params.id` into the request path unencoded on the read, mutate, and delete paths
(`apps/admin-ui/src/lib/api.ts:6-15` with the call sites at `:30`, `:39`, `:48`, `:53`, `:59`,
`:67`, `:75`, driven from `apps/admin-ui/src/routes/(app)/users/[id]/+page.server.ts:6,8,26,37,45`),
so a percent-encoded separator in the browser's URL changes which internal endpoint the
credentialed request addresses. And `apps/admin-ui/src/lib/types.ts:9` declares
`status: "Active" | "Suspended" | "Deleted"` where `UserStatus` carries
`#[serde(rename_all = "snake_case")]` (`crates/core/src/domain/user.rs:34-42`), so the badge
comparisons at `(app)/users/[id]/+page.svelte:13-19`, `(app)/users/+page.svelte:30-36` and
`(app)/+page.svelte:56-62` never match and the edit `<select>` at
`(app)/users/[id]/+page.svelte:62-65` submits a value the API rejects.

---

## Prerequisites and sequencing

Two sibling change specs own deltas this one depends on and does not restate.

[`2026-08-05-verify_admin_ui_session_jwt.md`](2026-08-05-verify_admin_ui_session_jwt.md) covers
the console's session gate — `hooks.server.ts` treating an unverified JWT as authenticated
identity, and the `POST /login` action minting a session cookie from caller-supplied JSON. It is
a prerequisite: an operator principal is worth nothing while the component that presents it
decides who an operator is by parsing JSON.

[`2026-08-05-audit_and_throttle_authentication_failures.md`](2026-08-05-audit_and_throttle_authentication_failures.md)
introduces the `RateLimiter` port, the `ClientAddr` provenance type, the mandatory
`SecurityEvent` audit channel, and the `TooManyRequests` → 429 (`slow_down`) error mapping,
and explicitly defers the internal API's failures to this spec.
This spec's throttle-and-audit step builds on that machinery: it adds one `SecurityEvent`
variant and one `RateLimitKey` variant rather than standing up a second limiter and a second
event channel for one route. If that spec has not merged, this one's step 2 waits for it.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | Internal-auth section rewritten (operator principal, throttle, audit, retained constant-time compare); Routes → Internal gains cursor pagination and `429`; Service roles becomes a listener table with the new default; Bootstrap gains the two-listener bind and the single-surface-runtime rule; the "shared secret is its only authentication" Assumption replaced (the 429 error-mapping row arrives via the sibling audit spec) |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | `[server].role` default becomes `exchange`; `[internal_api]` gains listener, auth-method, operator-token, mTLS, throttle and stats-cache keys and a minimum shared-secret length; defaults summary rows updated; the Validation-at-load internal-API bullet defers to the new `[internal_api]` rules |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | `UserRepository::list_users` becomes cursor-paginated returning `UserPage`; `RateLimitKey` gains an `OperatorAuth` variant; a note that `OperatorAuthenticator` is a server-layer trait, not a core port |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) | DynamoDB access-pattern rows for `list_users` and the stats counts; the user-counter item; closes the standing scan Open question |
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md) | `OperatorPrincipal` and `UserPage` entities; `AuditEvent` gains `operator`; `SecurityEvent` gains `OperatorAuthenticationFailed`; the `User.claims` reserved-name rule; the list-users query-pattern row follows the cursor signature |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | Custom claims: the closed reserved set enforced at write and on the template path; Admin operations table gains principal attribution, reserved-name rejection, and the cursor signature |
| [`.specs/service/specs/07-telemetry-and-audit.md`](../service/specs/07-telemetry-and-audit.md) | The admin plane's two security events and admin attribution |
| [`.specs/service/specs/00-overview.md`](../service/specs/00-overview.md) | System shape's operator arrow; Non-goals' rate-limiting bullet qualified; scope-summary rows for admin auth and roles |
| [`.specs/service/specs/canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | Add `OperatorPrincipal`, `OperatorAuthMechanism`, `UserPage`; add `operator` to `AuditEvent` |
| [`.specs/admin-ui/specs/00-overview.md`](../admin-ui/specs/00-overview.md) | Generated API client, wire-format status, operator credential, cursor pagination, environment keys |

No new canonical page. Adds `schemas/internal-api.schema.json` as the published contract the
console's client is generated from (documented within `.specs/admin-ui/specs/00-overview.md`).

---

## Proposed changes

### `.specs/service/specs/04-http-api.md` → Middleware stack, internal auth (Modify)

Replaces the middleware stack's closing internal-auth paragraph — the mount-gate sentence
(that rule now lives in Service roles), the shared-secret check, and the startup-rejection
sentence:

> Internal routes additionally pass through **internal auth**
> (`middleware/internal_auth.rs`), which authenticates a named `OperatorPrincipal`
> ([01-domain-model.md](01-domain-model.md)) and inserts it as a request extension. Enabled
> mechanisms come from `internal_api.auth_methods`, tried in order:
>
> | Mechanism | Credential | Principal id |
> |---|---|---|
> | `operator_token` | `Authorization: Bearer <jwt>`, verified against the service's own `KeyManager` with `iss = server.issuer`, `aud = internal_api.token_audience`, an unexpired window, and `internal_api.required_claim == required_value` | the token's `sub` |
> | `mtls` | client certificate subject, read from the terminating proxy's `internal_api.mtls.subject_header` on the admin listener only | the certificate subject |
> | `shared_secret` | `Authorization: Bearer <secret>` compared to `internal_api.shared_secret` in constant time (`subtle`) | the literal `unattributed` |
>
> `shared_secret` is the compatibility mechanism: it authenticates a request without
> identifying anyone, so its principal carries `mechanism = "shared_secret"` and the reserved
> id `unattributed` rather than a name it does not have. An audit reader can therefore
> distinguish an attributed action from an unattributed one without inspecting configuration.
>
> Every failed attempt — missing credential, unknown mechanism, bad credential, or an
> internal API with no mechanism configured — emits one `OperatorAuthenticationFailed`
> security event ([01-domain-model.md](01-domain-model.md)) on the mandatory audit channel,
> carrying the request id, the peer address, the route, and a `reason` of
> `missing_credential` | `invalid_credential` | `not_configured`, plus one `tracing::warn!`
> inside the request span. The presented credential is never recorded.
>
> Before any credential is evaluated, the layer consults the
> `RateLimitKey::OperatorAuth(peer)` budget ([02-ports-and-adapters.md](02-ports-and-adapters.md));
> a `Deny` — the lockout — short-circuits to `429` with `Retry-After` and emits
> `ThrottleExceeded`. A unit is consumed only by a failed attempt, so a working operator
> credential never draws down the budget. The key is the connection's peer address — a `ClientAddr::Peer`, never a
> forwarded or asserted one — because the admin listener sits behind no untrusted proxy.
> Admin-plane budgets (`internal_api.max_auth_failures` per `internal_api.auth_failure_window`,
> locked out for `internal_api.auth_lockout`) are configured separately from the exchange
> plane's and are far tighter: an operator does not retry a credential sixty times a minute.
> The in-process limiter is per process, so a fleet of *n* instances behind one address
> multiplies the budget by *n*; throttling bounds guessing and the audit record makes it
> visible, and neither prevents it.
>
> The constant-time comparison is retained for every secret that remains — the shared secret
> and the opaque portion of any operator credential compared as a string — including its
> length-branch handling, so no mechanism reintroduces a timing oracle.

### `.specs/service/specs/04-http-api.md` → Routes → Internal (Modify)

The section heading, the mount-gate paragraph beneath it, and the list-users row change;
every other row — the `404 if absent` annotations included — is unchanged:

> ### Internal (mounted on the admin listener only, behind operator auth)
>
> These routes exist only when `internal_api.enabled = true` and the role binds the admin
> listener; with the flag false (the default) no internal routes exist regardless of role.
> When they exist they are mounted on the admin listener only — never merged into the
> public router — behind operator auth (see Middleware stack).
>
> | Method | Path | Purpose |
> |---|---|---|
> | GET | `/internal/users` | list users, query `cursor`/`limit` → `UserPage` |
>
> `limit` defaults to 50 and is clamped to `MAX_ADMIN_PAGE_SIZE` (200) in the core, not the
> handler, so every caller of `list_users` is bounded. `cursor` is the opaque
> `UserPage.next_cursor` from the previous page; an absent cursor starts at the first page,
> and a `next_cursor` of `null` means the listing is exhausted. A page may return fewer rows
> than `limit` while still carrying a non-null `next_cursor` — on DynamoDB the scan `Limit`
> applies before the `sk = PROFILE` filter — so a caller pages until `next_cursor` is null,
> never until a short page.
>
> Any `/internal/*` request rejected by the auth-failure throttle returns `429` with
> `Retry-After`.

### `.specs/service/specs/04-http-api.md` → Service roles (Modify)

> `server.role` ∈ `{ all, exchange, admin }` (default **`exchange`**) selects which listeners
> the process binds and which adapters the bootstrap builds:
>
> | Role | Public listener (`server.host:port`) | Admin listener (`internal_api.host:port`) | Adapters built |
> |---|---|---|---|
> | `exchange` | public routes + `/health` | not bound | user repo, session repo, key manager, providers, audit; user-sync → noop |
> | `admin` | not bound | `/internal/*` + `/health` | user repo, session repo, audit, user-sync, key manager (required when `operator_token` is enabled) |
> | `all` | public routes + `/health` | `/internal/*` + `/health` | all |
>
> `/internal/*` is never merged into the public router. `role = "all"` binds two sockets
> rather than composing two route sets onto one, so network policy can reach the admin plane
> without reaching `/token` and can firewall the admin plane without firewalling `/token`.
> The admin listener defaults to `127.0.0.1`; publishing it is an explicit configuration act.
>
> The default role serves only the exchange plane. Serving the admin plane requires both
> `internal_api.enabled = true` and a role that binds the admin listener, and each is visible
> in configuration.

### `.specs/service/specs/04-http-api.md` → Bootstrap (Modify)

Step 5 and step 6 become:

> 5. `bootstrap::build_routers` — build the axum router(s) the role requires: a public router,
>    an admin router, or both. Each carries the same middleware stack and shared state; only
>    the admin router carries the internal-auth layer.
> 6. Detect runtime. Under hyper, bind one socket per router the role produced — each served
>    through the connect-info make-service, so the client-address middleware keeps a real
>    peer on both planes — and serve them concurrently under one graceful-shutdown signal,
>    the existing drain deadline and `server.base_path` stripping applying to both; a bind
>    failure on either socket fails startup rather than silently serving half the configured
>    surface. Under Lambda and
>    `crates/ffi` there is one request surface and no second socket to bind, so the process
>    serves exactly one plane: `exchange` and `admin` serve theirs, and `all` serves the
>    public plane while logging a startup warning that names the unmounted internal routes.
>    Plane separation on those runtimes is expressed by deploying a second function or
>    instance with `role = "admin"`.

### `.specs/service/specs/04-http-api.md` → Assumptions (Modify)

The second Assumption is replaced:

> - The admin listener is reachable only from an operator network. Its authentication is a
>   named principal, and the shared secret remains accepted only for deployments that have not
>   yet migrated.

### `.specs/service/specs/06-configuration.md` → Sections → `[server]` and `[internal_api]` (Modify)

In `[server]`, only the `role` clause changes — `base_path`, `request_timeout`, and whatever
the sibling audit spec has added stay as the page carries them:

> `role` (`all` | `exchange` | `admin`, default `exchange`)

The `[internal_api]` section is replaced in full:

> ### `[internal_api]`
> `enabled` (false), `host` (`127.0.0.1`) and `port` (`8081`) — the admin listener, bound
> separately from `server.host`/`server.port` — and `auth_methods` (`Vec<String>` over
> `operator_token` | `mtls` | `shared_secret`, default `["shared_secret"]`, tried in the order
> given). The singular `auth_method` key is still accepted and read as a one-element list.
>
> Per mechanism: `shared_secret` (redacted in `Debug`); `token_audience` (`"internal"`),
> `required_claim` (`"role"`) and `required_value` (`"admin"`) for `operator_token`;
> `[internal_api.mtls] { subject_header }` for `mtls`.
>
> Throttle and cache: `max_auth_failures` (`5`), `auth_failure_window` (`"1m"`),
> `auth_lockout` (`"5m"`), `stats_cache_ttl` (`"60s"`).
>
> `AppConfig::validate` requires, whenever the role binds the admin listener and
> `enabled = true`: a non-empty `auth_methods`; a `shared_secret` of at least 32 bytes if
> `shared_secret` is among them — non-empty is not sufficient for the string that is the
> plane's entire authentication, and today's validation accepts a one-character value; a
> non-empty `server.issuer` and a non-noop key manager if
> `operator_token` is among them; and an admin listener that does not collide with
> `server.host:server.port`. It emits a startup warning when `shared_secret` is the only
> enabled mechanism.

### `.specs/service/specs/06-configuration.md` → Validation at load (Modify)

The internal-API bullet — "`internal_api.shared_secret` must be present and non-empty"
whenever the internal API is served — no longer holds under `auth_methods` and is replaced:

> - When the internal API will be served (the role binds the admin listener and
>   `internal_api.enabled = true`), the `[internal_api]` requirements apply: a non-empty
>   `auth_methods`, the per-mechanism secret and key-manager requirements — including the
>   shared secret's 32-byte minimum length — and the
>   listener-collision check (see `[internal_api]`).

The same section's closed-domain table row must move with it. The sibling
[2026-08-05-fail_closed_across_config_and_adapters.md](2026-08-05-fail_closed_across_config_and_adapters.md)
merges first and adds a row pinning the scalar `internal_api.auth_method` to the single value
`shared_secret` via an `InternalAuthMethod` newtype. That key no longer exists once this
change lands, so the row is replaced rather than left beside its successor:

> | `internal_api.auth_methods` | `Vec<InternalAuthMethod>` | `operator_token` \| `mtls` \| `shared_secret` (non-empty, no duplicates) |

### `.specs/service/specs/06-configuration.md` → Defaults summary (Modify)

> | `server.host` / `port` / `role` / `request_timeout` | `0.0.0.0` / `8080` / `exchange` / `"30s"` |
> | `internal_api.host` / `port` / `auth_methods` | `127.0.0.1` / `8081` / `["shared_secret"]` |
> | `internal_api.max_auth_failures` / `auth_failure_window` / `auth_lockout` | `5` / `"1m"` / `"5m"` |

### `.specs/service/specs/02-ports-and-adapters.md` → UserRepository (Modify)

> ```rust
> async fn list_users(&self, cursor: Option<&str>, limit: u32) -> Result<UserPage>;
> ```
>
> `list_users` is cursor-paginated: `limit` is clamped to `MAX_ADMIN_PAGE_SIZE` (200) before
> it reaches an adapter, and every adapter pushes that bound into the store rather than
> applying it to an in-memory result. The cursor is opaque to the caller and adapter-defined:
> the SQL adapters encode the last row's `(created_at, id)` keyset, the DynamoDB adapter
> encodes the scan's `LastEvaluatedKey`. A cursor is only valid against the adapter that
> issued it; an unparseable cursor is `InvalidRequest`.

### `.specs/service/specs/02-ports-and-adapters.md` → RateLimiter (Modify)

`RateLimitKey` gains one variant, so the admin plane draws on its own budget rather than
sharing the exchange plane's:

> ```rust
> enum RateLimitKey {
>     ClientAddr(IpAddr),
>     OperatorAuth(IpAddr),
>     Subject { provider: Option<String>, subject_hash: String },
>     Provider(String),
> }
> ```
>
> `OperatorAuth` is consumed only by a *failed* `/internal/*` authentication. Keeping it
> distinct from `ClientAddr` means a burst of anonymous exchange traffic cannot exhaust the
> operator budget and lock an administrator out of the plane they would use to respond to it.

### `.specs/service/specs/02-ports-and-adapters.md` → Port traits (Add, after the port list)

> `OperatorAuthenticator` — the trait behind `internal_api.auth_methods` — is **not** a core
> port. It authenticates HTTP request parts, which `crates/core` does not model, so it lives
> in `crates/server/src/middleware/` alongside the layer that drives it. The inward-dependency
> rule is unaffected: the core owns `OperatorPrincipal`, the server owns the authentication of
> one.

### `.specs/service/specs/08-persistence.md` → DynamoDB access patterns (Modify)

The item table gains the counter row; in the access-pattern table, the combined
`count_by_status` / `count_active_sessions` scan row is replaced by two and a `list_users`
row is added:

> | Item | pk | sk | GSI1pk | GSI1sk |
> |---|---|---|---|---|
> | User counters | `STATS#USERS` | `COUNTS` | — | — |
>
> | Operation | DynamoDB call |
> |---|---|
> | `list_users` | `Scan` with `Limit` = the requested page size and `ExclusiveStartKey` = the decoded cursor; one call per page, no in-memory sort |
> | `count_by_status` | `GetItem` on the `STATS#USERS` / `COUNTS` counter item |
> | `count_active_sessions` | a `Scan` refreshed at most once per `internal_api.stats_cache_ttl` per process |
>
> The counter item holds one numeric attribute per `UserStatus` value and is updated in the
> same `TransactWriteItems` that creates, status-patches, or deletes a user, so it moves with
> the write that changes a status and cannot record a state no write produced.
>
> Session counts are not maintained the same way: DynamoDB's TTL reaper deletes session items
> without passing through the adapter, so a session counter would drift downward silently.
> The active-session count therefore stays a scan, but a cached one — at most one walk per
> `stats_cache_ttl` per process, however many `GET /internal/stats` calls arrive.
>
> A `Scan` orders items by the store's internal key distribution, not by `created_at`, so
> `list_users` returns pages in scan order on DynamoDB and in `created_at DESC` order on the
> SQL adapters. Cursor paging is stable and complete on both; only the ordering differs.

### `.specs/service/specs/08-persistence.md` → Open questions (Remove)

The standing entry — "`count_by_status` / `count_active_sessions` on DynamoDB rely on scans;
at large table sizes a maintained counter item or a stream-fed aggregate may be needed. Not
yet addressed." — is resolved by the counter item and the cached session aggregate above and
is removed.

### `.specs/service/specs/01-domain-model.md` → Entities (Add)

> ### OperatorPrincipal (`domain/operator.rs`)
>
> ```rust
> struct OperatorPrincipal {
>     id: String,                       // certificate subject, token `sub`, or "unattributed"
>     mechanism: OperatorAuthMechanism, // MutualTls | OperatorToken | SharedSecret
> }
> ```
>
> The authenticated identity behind an `/internal/*` request. Every admin service method
> receives one and every audit event an admin operation emits carries it. `SharedSecret`
> always pairs with the reserved id `unattributed`: that mechanism proves possession of a
> string and identifies nobody, and the event says so rather than naming a principal it does
> not have.
>
> ### UserPage (`domain/user.rs`)
>
> ```rust
> struct UserPage {
>     users: Vec<User>,
>     next_cursor: Option<String>,  // opaque; None means the listing is exhausted
> }
> ```

### `.specs/service/specs/01-domain-model.md` → Entities → User (Modify)

The `claims` sentence gains the reserved-name rule:

> `metadata` is operator/sync data and is readable by claim templates; `claims` is the set of
> private claims injected into the access token, managed through the internal API. No key in
> `claims` may be a reserved protocol claim name ([03-service-flows.md](03-service-flows.md));
> the internal API rejects such a write rather than accepting and later filtering it, so the
> stored map and the signed token agree.

### `.specs/service/specs/01-domain-model.md` → AuditEvent (Modify)

One field is added to the `AuditEvent` struct, wherever
[`2026-08-05-audit_and_throttle_authentication_failures.md`](2026-08-05-audit_and_throttle_authentication_failures.md)
has left it — that spec adds `ip_address_source` to the same struct and the two additions
fold together:

> ```rust
>     operator: Option<OperatorPrincipal>, // who performed it, on `/internal/*` operations
> ```
>
> `actor` and `operator` answer different questions and are both present on an admin
> mutation: `actor` is the user the action was performed *on*, `operator` is the principal
> that performed it. `operator` is `None` on the exchange plane, where there is no operator.
> The existing `actor` comment narrows to "user id if known — the subject of the action".

### `.specs/service/specs/01-domain-model.md` → SecurityEvent (Modify)

The variant table gains one row. `Unauthorized` is one of the `AuditEventType` variants the
codebase declares and never constructs; this is what constructs it:

> | Variant | `AuditEventType` | Severity |
> |---|---|---|
> | `OperatorAuthenticationFailed` | `Unauthorized` | `Warning`(4) |
>
> `OperatorAuthenticationFailed` is distinct from `AuthenticationFailed`: the latter is an
> end user failing to authenticate on the exchange plane, the former is someone failing to
> authenticate as an operator. Collapsing them would make a guessing campaign against the
> privilege-assignment primitive indistinguishable, in a query, from a user mistyping a
> password at their identity provider.

### `.specs/service/specs/01-domain-model.md` → Required query patterns (Modify)

The list-users row follows the port change:

> | List users (paged) | `UserRepository::list_users(cursor, limit)` → `UserPage` |

### `.specs/service/specs/03-service-flows.md` → Custom claims (Modify)

The single "Reserved names" line is replaced:

> **Reserved claim names.** `RESERVED_CLAIMS` is the closed set of 24 registered and
> de-facto protocol claim names — the RFC 7519 §4.1 names `iss`, `sub`, `aud`, `exp`, `nbf`,
> `iat`, `jti`; the OpenID Connect, RFC 9068, and RFC 7800 names `acr`, `amr`, `at_hash`,
> `auth_time`, `azp`, `c_hash`, `cnf`, `nonce`, `sid`, `typ`, `client_id`; and the de-facto
> authorization names `scope`, `scp`, `roles`, `groups`, `entitlements`, `permissions`. It
> is a closed set rather than a maintained denylist: a name is reserved because a conformant
> verifier or a relying party reads it as protocol-defined, and the set changes only when
> the registry it mirrors does.
>
> The set is enforced at three points, not one:
>
> 1. **Write** — `admin_set_claims` and `admin_merge_claims` reject a payload containing a
>    reserved key with `InvalidRequest`, naming the offending key. A reserved name is never
>    persisted, so it cannot be re-exported later.
> 2. **Config templates** — a `token.custom_claims` entry keyed by a reserved name is
>    rejected by `AppConfig::validate` at startup rather than silently dropped at token build.
> 3. **Token build** — `resolve_custom_claims` still drops reserved names from both sources,
>    and `resolve_field` refuses a `{{ user.claims.<reserved> }}` reference, so a claim
>    persisted before this rule existed cannot reach a signed token through either route.

### `.specs/service/specs/03-service-flows.md` → Admin operations (Modify)

The preamble and three rows change; the remaining rows keep their current behaviour text:

> All under `/internal/*`. Every method takes the request's `OperatorPrincipal`
> ([01-domain-model.md](01-domain-model.md)) and every audit event it emits carries it, so
> each admin mutation records who performed it as well as what changed. User-sync
> notifications are **best-effort** — failures are logged via `tracing` and never fail the
> admin call.
>
> | Method | Behaviour |
> |---|---|
> | `admin_set_claims` | reject any reserved claim name (`InvalidRequest`), then replace the whole claims map |
> | `admin_merge_claims` | reject any reserved claim name (`InvalidRequest`), then merge new keys over existing (new wins) |
> | `admin_list_users` | `list_users(cursor, limit)` with `limit` clamped to `MAX_ADMIN_PAGE_SIZE` → `UserPage` |

### `.specs/service/specs/07-telemetry-and-audit.md` → Audit (Add)

> The admin plane contributes two security events. Every rejected `/internal/*`
> authentication produces one `OperatorAuthenticationFailed` — rendered as an `Unauthorized`
> `AuditEvent` at `warning` — carrying the peer address, the route, and a `reason`
> (`missing_credential`, `invalid_credential`, `not_configured`) but never the presented
> credential, alongside a `tracing::warn!` in the request span. When the `OperatorAuth`
> rate-limit budget is exhausted, one `ThrottleExceeded` records the lockout. Both travel the
> mandatory channel, so no `emit_threshold` can filter them, and a failed write follows
> `audit.durability` — failing the request under `enforce` — rather than being silently
> dropped.
>
> Every successful admin mutation carries `operator` ([01-domain-model.md](01-domain-model.md)).
> Under the `shared_secret` mechanism that field is present and explicitly unattributed, so a
> reviewer can measure migration progress from the event stream rather than from
> configuration.

### `.specs/service/specs/00-overview.md` → System shape (Modify)

> ```
> operator ──principal──► /internal/* on the admin listener (user CRUD, claims, stats) ◄── admin-ui
> ```

### `.specs/service/specs/00-overview.md` → Non-goals (Modify)

The rate-limiting bullet, as the sibling audit spec leaves it (a globally coordinated rate
limit), is extended with the admin plane's variant of the same per-process caveat:

> - A globally coordinated rate limit. The service bounds attempts per process; a shared
>   budget across instances needs the `RateLimiter` port backed by a shared store, or an
>   edge gateway. The admin plane's failed-authentication throttle shares that shape: its
>   `OperatorAuth` budget is per process, bounds credential guessing against `/internal/*`,
>   and does nothing to successful requests.

### `.specs/service/specs/00-overview.md` → Scope summary (Modify)

> | Internal admin API (users, claims, stats) | Yes | named operator principal; shared secret accepted during migration |
> | Service roles (`all`/`exchange`/`admin`) | Yes | conditional adapter wiring and one listener per plane; default `exchange` |

### `.specs/admin-ui/specs/00-overview.md` → Pages (Modify)

The `(app)/users` row's loader reference becomes `listUsers(cursor, limit)`.

### `.specs/admin-ui/specs/00-overview.md` → Internal API client (Modify)

> ## Internal API client (`src/lib/api.ts`)
>
> A client generated at build time from `schemas/internal-api.schema.json`, the service's
> published internal-API contract. The generator emits the path encoding and the wire-format
> enum values, so a route parameter cannot alter which endpoint a credentialed request
> addresses and a status value cannot disagree with the service's spelling — both properties
> hold by construction rather than by every call site remembering. Base `INTERNAL_API_URL`
> (default `http://localhost:8081`, the service's admin listener). `src/lib/types.ts` is
> generated from the same schema.
>
> The client authenticates with the operator credential the deployment configured:
> `INTERNAL_API_TOKEN` (an operator token) or a client certificate, falling back to
> `INTERNAL_API_SECRET` where the deployment has not yet migrated. Whichever it holds stays
> server-side; the browser never sees it. `listUsers` pages with `cursor`/`limit` and follows
> `next_cursor` until it is null.

### `.specs/admin-ui/specs/00-overview.md` → Environment (Modify)

Only the credential entry changes; every other variable — including the
`OIDC_EXCHANGE_ISSUER` and `ADMIN_UI_AUDIENCE` the session-JWT sibling adds — stays as that
spec leaves the section. `INTERNAL_API_SECRET` becomes:

> one of `INTERNAL_API_TOKEN` / `INTERNAL_API_CLIENT_CERT` + `INTERNAL_API_CLIENT_KEY` /
> `INTERNAL_API_SECRET` (in that order of preference)

### `.specs/admin-ui/specs/00-overview.md` → Decisions (Modify)

> - *Secret stays server-side.* **The operator credential lives only in the SvelteKit server;
>   the browser holds only the session cookie.** The browser never sees the internal-API
>   credential.
> - *Generated client, not a hand-written one.* **`src/lib/api.ts` and `src/lib/types.ts` are
>   generated from the service's internal-API schema.** Path encoding and enum spelling become
>   properties of the generator rather than obligations on eight call sites and three badge
>   sites; a service-side rename breaks the build instead of silently breaking an operator
>   control.

---

## Type changes

```json
{
  "$comment": "Fragment for 2026-08-05-harden_admin_plane. Folds into .specs/service/specs/canonical-types.schema.json on merge. AuditEvent carries one added property, `operator`; the base shown is today's canonical shape — 2026-08-05-audit_and_throttle_authentication_failures adds ip_address_source to the same object first, and the merge keeps whatever shape the file then carries. AuditEventType is not restated here — that sibling owns its `throttle_exceeded` addition, and this change constructs the already-declared `unauthorized`.",
  "$defs": {
    "OperatorAuthMechanism": {
      "type": "string",
      "enum": ["mtls", "operator_token", "shared_secret"],
      "description": "How an /internal/* request was authenticated."
    },
    "OperatorPrincipal": {
      "type": "object",
      "required": ["id", "mechanism"],
      "properties": {
        "id": {
          "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString",
          "description": "Certificate subject, operator-token `sub`, or the reserved literal `unattributed` when mechanism is shared_secret."
        },
        "mechanism": { "$ref": "#/$defs/OperatorAuthMechanism" }
      }
    },
    "UserPage": {
      "type": "object",
      "required": ["users", "next_cursor"],
      "properties": {
        "users": { "type": "array", "items": { "$ref": "#/$defs/User" } },
        "next_cursor": {
          "type": ["string", "null"],
          "description": "Opaque, adapter-issued. Null means the listing is exhausted. A page may be shorter than the requested limit while this is non-null."
        }
      }
    },
    "AuditEvent": {
      "type": "object",
      "required": ["id", "timestamp", "severity", "event_type", "detail", "outcome"],
      "properties": {
        "id": { "$ref": "../../canonical-types.schema.json#/$defs/Ulid" },
        "timestamp": { "$ref": "../../canonical-types.schema.json#/$defs/Timestamp" },
        "severity": { "$ref": "#/$defs/AuditSeverity" },
        "event_type": { "$ref": "#/$defs/AuditEventType" },
        "actor": {
          "type": ["string", "null"],
          "description": "User id if known — the subject of the action."
        },
        "operator": {
          "oneOf": [{ "$ref": "#/$defs/OperatorPrincipal" }, { "type": "null" }],
          "description": "Added. The principal that performed the action; present on /internal/* operations, null on the exchange plane."
        },
        "provider": { "type": ["string", "null"] },
        "ip_address": { "type": ["string", "null"] },
        "user_agent": { "type": ["string", "null"] },
        "detail": { "type": "object", "additionalProperties": true },
        "outcome": { "$ref": "#/$defs/AuditOutcome" }
      }
    }
  }
}
```

`UserStatus` is unchanged — the console adopts the schema's existing `active`/`suspended`/
`deleted` values rather than the schema adopting the console's. That is the point of the
mismatch fix: one canonical representation, and it is the service's.

---

## Implementation notes

Ordered so each step is independently shippable and revertible, and so the shared secret
keeps working until the last one. Step 1 depends on nothing here; step 2 depends on the
`RateLimiter` port and mandatory audit channel from
[`2026-08-05-audit_and_throttle_authentication_failures.md`](2026-08-05-audit_and_throttle_authentication_failures.md).

```
 1. Console, tactical (Option 1's non-session half; ships with the prerequisite change spec).
 2. Throttle + audit on internal_auth_layer.
 3. Default role → "exchange".
 4. Second listener.
 5. OperatorPrincipal, accepted alongside the shared secret.
 6. Generated console client.
 7. Closed reserved-claim set.
 8. Bounded admin reads.
 9. Shared-secret deprecation warning (one release).
10. Shared-secret removal.
```

1. **Console, tactical.** Wrap every `params.id` in `encodeURIComponent` inside
   `apps/admin-ui/src/lib/api.ts` (`getUser`, `updateUser`, `deleteUser`, `getUserClaims`,
   `setClaims`, `mergeClaims`, `clearClaims`) rather than at the five call sites in
   `routes/(app)/users/[id]/+page.server.ts` — encoding in the client is one place, not five,
   and it is what step 6 makes structural. Change `types.ts:9` to
   `"active" | "suspended" | "deleted"` and fix all four status sites:
   `(app)/users/[id]/+page.svelte:13-19` and `:62-65`, `(app)/users/+page.svelte:30-36`,
   `(app)/+page.svelte:56-62`. Verify with correction #41's exact payloads —
   `x%2f..%2fstats` and `%2e%2e%2fstats` reach different endpoints and each must now reach
   only `/internal/users/<literal>`.
2. **Throttle + audit.** In `crates/server/src/middleware/internal_auth.rs`, keep
   `constant_time_eq` (`:55-63`) untouched. Add `RateLimitKey::OperatorAuth(IpAddr)` to the
   sibling spec's `RateLimiter` port, consume one unit on each failure, and short-circuit on
   `Deny` before any credential is evaluated. Add the `OperatorAuthenticationFailed`
   `SecurityEvent` variant and emit it on every failure arm — the `AuditEventType::Unauthorized`
   it renders to is the variant at `crates/core/src/domain/audit.rs:52` that nothing currently
   constructs. The peer address comes from axum's `ConnectInfo` as a `ClientAddr::Peer`, never
   `X-Forwarded-For`; the admin listener is behind no untrusted proxy. In the same step,
   extend `AppConfig::validate` (`crates/core/src/config.rs:74-90`) to require at least 32
   bytes for `internal_api.shared_secret` — today it requires only non-empty — with a
   release note for deployments holding a shorter secret, and a test asserting that a
   31-byte secret fails startup while a 32-byte one boots.
3. **Default role.** `crates/core/src/config.rs:158`. Release note required: an existing
   deployment relying on the implicit `all` must now set `role` explicitly.
4. **Second listener.** Split `build_router` (`crates/server/src/bootstrap.rs:326-363`) into
   `build_public_router` / `build_admin_router` sharing the middleware stack and `AppState`;
   `routes::internal_routes` stops being merged into the public router at `:339-341`.
   `crates/server/src/main.rs` (router build at `:30`, bind-and-serve at `:52-70`) binds one
   socket per router and joins them under the existing shutdown signal. `crates/server/src/lambda.rs` and `crates/ffi` take the
   single-plane rule and the `role = "all"` startup warning. Add `host`/`port` to
   `InternalApiConfig` (`crates/core/src/config.rs:388-392`) and the collision check to
   `AppConfig::validate`.
5. **Operator principal.** `OperatorPrincipal` in `crates/core/src/domain/`; the
   `OperatorAuthenticator` trait and its three implementations in
   `crates/server/src/middleware/`. Thread the principal from the request extension through
   `crates/core/src/service/user_admin.rs` into `emit_audit`. `role = "admin"` builds a noop
   key manager today (`bootstrap.rs:254`); `operator_token` needs a real one, so
   `AppConfig::validate` must catch that combination at startup.
6. **Generated client.** Publish `schemas/internal-api.schema.json` next to the existing
   `schemas/datamodel.schema.json`; generate `apps/admin-ui/src/lib/api.ts` and `types.ts`
   from it in the build; delete the hand-written `api()`. The step-1 `encodeURIComponent`
   calls stay until this lands.
7. **Reserved claims.** Widen `RESERVED_CLAIMS` (`crates/core/src/service/claims.rs:8`) to the
   closed set and enforce it at `admin_set_claims` / `admin_merge_claims`
   (`crates/core/src/service/user_admin.rs`), in `AppConfig::validate` for
   `token.custom_claims`, and in `resolve_field`'s `"claims"` arm (`claims.rs:116-119`).
   Enumerate all 24 names in the test, including `sid`.

   **Merge order matters here.** [2026-08-05-validate_revoke_token_claims.md](2026-08-05-validate_revoke_token_claims.md)
   merges before this spec and adds `sid` (with `nbf`) to `RESERVED_CLAIMS`, because a
   per-user claim named `sid` collides with the flattened `AccessTokenClaims.sid` field and
   would let an admin-set claim forge the session binding that `/revoke` resolves. This
   closed set therefore **includes** `sid`; replacing the line with a set that omits it would
   silently re-open that collision. If this spec ever merges first, that sibling's `sid` and
   `nbf` additions must be folded in rather than overwritten.
8. **Bounded reads.** Change the port signature (`crates/core/src/ports/repository.rs`) and
   all three adapters: `dynamo/mod.rs:593-633` becomes one bounded `Scan` per page;
   `postgres/mod.rs:474-486` and `sqlite/mod.rs:469-481` move from `LIMIT/OFFSET` to a
   `(created_at, id)` keyset. Add the `STATS#USERS` counter update to the create/delete
   `TransactWriteItems` paths, promote the status-changing `update_user` write — today a
   plain versioned `PutItem` except on the transition into `Deleted` — into a transaction
   that carries it, and cache `count_active_sessions`
   (`dynamo/mod.rs:553-…`, `:686-…`). Update `crates/server/src/routes/internal.rs:47-61` and
   `crates/test-utils`. Measure consumed read capacity, not wall time, in the test.
9. **Deprecation warning**, then **10. removal** — a release apart.

---

## The transition cost

Operator identity replaces one environment variable with a credential-distribution
mechanism — certificate issuance and rotation, or an operator-token flow the service does not
have today. That is the largest adoption barrier in this change, and it is why the shared
secret keeps working through a long transition rather than a short one.

Concretely: steps 1–4 and 7–8 require no credential *mechanism* change at all, so most of
the security gain is available to a deployment that never migrates — with one narrow
exception: step 2's length floor forces a deployment whose shared secret is shorter than
32 bytes to regenerate the value (not the mechanism), which is a one-line change and a
secret that was too weak to keep. Step 5 adds the new
mechanisms alongside `shared_secret`, with both accepted and every audit event recording which
one authenticated the request. Step 9 warns at startup when `shared_secret` is the only enabled
mechanism, and runs for at least one release. Step 10 removes it.

The gain from steps 5–10 arrives only when a deployment actually moves, and the event stream —
the ratio of attributed to `unattributed` operators — is what tells a maintainer whether it
has. That number, not the presence of the feature, is the acceptance criterion for step 10. A
migration described honestly and completed slowly is worth more than one described briskly and
abandoned.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**` to
   the merge date.
2. Fold the `Type changes` `$defs` into
   `.specs/service/specs/canonical-types.schema.json` — `OperatorPrincipal`,
   `OperatorAuthMechanism` and `UserPage` are new; `AuditEvent` gains one property and
   otherwise keeps whatever shape the file carries.
3. Remove the resolved Open question from `.specs/service/specs/08-persistence.md`.
4. Confirm both siblings have merged first:
   [`2026-08-05-verify_admin_ui_session_jwt.md`](2026-08-05-verify_admin_ui_session_jwt.md)
   owns the admin-ui page's Authentication-model section, and
   [`2026-08-05-audit_and_throttle_authentication_failures.md`](2026-08-05-audit_and_throttle_authentication_failures.md)
   owns `SecurityEvent`, `ClientAddr`, `RateLimiter` and the `AuditEventType` additions this
   spec's blocks extend. Applying this spec's `SecurityEvent` and `RateLimitKey` blocks before
   those sections exist has no target.
5. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, move it to
   `.specs/changes/merged/`.
6. Update `.specs/README.md`'s Change specs table.

---

## Assumptions and open questions

### Assumptions

- The admin listener is reached over an operator network or a TLS-terminating proxy the
  deployment controls. The `mtls` mechanism reads its subject from that proxy's header, which
  is trustworthy exactly to the degree the listener is not publicly routable — which is why
  the listener defaults to `127.0.0.1`.
- The in-process `RateLimiter` adapter is per process. A fleet behind one address multiplies
  the guessing budget by the instance count; the audit record is what makes the attempt visible
  regardless. A shared-store limiter would close the gap and is that port's concern, not this
  spec's.
- `.specs/service/specs/03-service-flows.md` has absorbed the Admin-operations delta from
  [`merged/2026-07-01-enforce_user_lifecycle_transitions.md`](merged/2026-07-01-enforce_user_lifecycle_transitions.md)
  — `NotFound` on unknown ids, transition validation, revoke-on-status-change, and the
  admin-audit paragraph. This spec's Admin-operations block changes only the preamble and the
  three rows it names and is applied on top of that table as it stands at merge time.

### Decisions

- *Named principal, not a better secret.* **`internal_auth_layer` authenticates an
  `OperatorPrincipal`; the shared secret becomes one mechanism among three and identifies
  nobody.** A rotated or lengthened secret still cannot answer "who granted this claim", and
  that question is the one an admin audit trail exists to answer. Everything downstream —
  attribution, per-operator throttling, and later per-operator scopes — needs a name to attach
  to.
- *A minimum secret length all the same.* **`internal_api.shared_secret` must be at least
  32 bytes whenever the mechanism is enabled; `AppConfig::validate` today requires only
  non-empty.** "Not a better secret" is about what a secret cannot do, not a licence for a
  weak one: while `shared_secret` remains accepted — which the transition keeps long — it is
  the plane's entire authentication, and the throttle bounds only the guess *rate* where
  entropy bounds the guess *space*. Validation measures length because entropy cannot be
  measured at load; 32 bytes of generated randomness (`openssl rand -base64 32`, which the
  docs should recommend beside the key) puts online guessing out of reach without imposing
  a format.
- *Unattributed is a value, not an absence.* **The `shared_secret` mechanism yields
  `id = "unattributed"` rather than a null operator.** An event that says "authenticated, by a
  mechanism that identifies nobody" is a different and more useful record than an event with a
  missing field, and it makes migration progress measurable from the event stream.
- *Reuse the limiter and the event channel, do not build a second.* **Admin-plane throttling
  goes through the `RateLimiter` port with its own `OperatorAuth` key, and admin-plane failures
  go through the mandatory `SecurityEvent` channel with one new variant.** A bespoke counter and
  a bespoke log line for one route would be quicker to write and would then have to be
  discovered separately by anyone auditing what this service records. One channel, one limiter,
  two budgets.
- *Two listeners, not two processes.* **`role = "all"` binds `server.port` and
  `internal_api.port` separately rather than splitting the binary.** It delivers the property
  AP3 asks for — network policy can act on the admin plane independently — at the cost of one
  extra bind, and it keeps single-node deployments viable.
- *Default role becomes `exchange`.* **A stock process serves only the public plane.** The
  hazard C6 identifies is a single configuration step, and the fix is to make that step
  explicit; the cost is a release note for deployments relying on the implicit `all`.
- *Closed set, not a namespace.* **Reserved claims are the 24 registered and de-facto
  protocol names, refused at the write path.** Namespacing — the proposal's Option 2 preference — is stronger,
  because its correctness does not depend on a set staying complete, but it changes the wire
  format of every issued token and therefore every relying party. The closed set closes the
  same escalation with no relying-party migration; namespacing stays open below.
- *Enforce at write, not only at read.* **A reserved name is rejected by the internal API
  rather than accepted and filtered at token build.** Filtering at read alone leaves the name
  persisted on the user record and re-exportable through a `{{ user.claims.KEY }}` template —
  a second route to the same signed token.
- *Cursor pagination replaces `offset`/`limit` outright.* **`GET /internal/users` takes
  `cursor` and returns `UserPage`; `offset` is removed rather than deprecated.** The response
  shape changes in the same release, the only in-tree consumer is the console (regenerated in
  step 6), and an `offset` that DynamoDB cannot honour without a full scan is the defect, not
  a compatibility surface worth preserving.
- *Counters for users, a cache for sessions.* **User counts come from a transactional counter
  item; the active-session count stays a scan, cached per `stats_cache_ttl`.** User writes all
  pass through the adapter and can carry a counter update atomically; DynamoDB's TTL reaper
  deletes sessions without passing through the adapter, so a session counter would drift
  downward with nothing to correct it.
- *Encoding in the client, then generated.* **Step 1 puts `encodeURIComponent` inside
  `api.ts`, not at the call sites; step 6 deletes both.** Encoding at eight call sites is the
  same shape of control that produced the finding — a step every caller must remember.
- *The console's credential is replaced, not removed.* **The console holds a long-lived
  operator credential.** Exchanging the operator's verified session for a short-lived scoped
  grant (the proposal's Option 3) is the better end state and needs the principal from step 5
  to exist first; it is a separate change spec.

### Open questions

- *mTLS or operator tokens?* Both satisfy the named-principal requirement. Tokens reuse the
  key manager and JWKS this service already has but need an issuance flow that does not exist;
  mTLS is stronger and harder to operate. Which becomes the recommended default depends on how
  operators deploy, and the answer should precede step 5's documentation.
- *Namespaced claims?* If the maintainers prefer the proposal's namespace over the closed set,
  the write-path enforcement in step 7 is where it lands, with a dual-emission window so
  relying parties can migrate. This changes the token wire format and needs a maintainer
  decision, not an engineering one.
- *Already-persisted reserved claims.* A user record written before step 7 may hold a reserved
  name. Token build refuses to emit it, so the escalation is closed, but the record still
  carries it — open whether a maintenance command reports or strips them.
- *Names beyond the probe set.* The set mirrors the scan finding's 23-name enumeration plus
  `sid`, which the revoke sibling reserves because this service reads it as protocol-defined.
  IANA registers claims the probe did not exercise — `token_use` among them — and any of them
  can only be reserved by changing the closed set. Whether to track the registry beyond these
  24 is open; namespacing (above) would make the question moot.
- *`created_at` ordering on DynamoDB.* Cursor paging is stable and complete but returns scan
  order, not `created_at DESC`. A GSI keyed for profile listing would restore the ordering at
  the cost of a table-schema change and a migration; open whether the admin list needs it.
- *Does `role = "all"` survive?* Two listeners make it safe, and it stays convenient for
  single-node deployments. Removing it would make plane separation unconditional. Keeping it
  is the proposal's lean and this spec's, but the argument the other way is real.
- *Is the console deployed anywhere?* If it is not — and the fact that every Edit User
  submission currently fails 422 suggests it has never run against a live service — deleting
  it is a legitimate alternative to hardening it, and that decision should precede step 6.
