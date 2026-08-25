# Change: Eliminate secret leakage in logs, spans, and error responses

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** crates/adapters, crates/providers, crates/server (service)

Make it a compile error, rather than a review miss, for credential-bearing material to reach a
log line, a tracing span field, or a client-facing error body. Introduce a `Secret<T>` newtype in
`crates/core` that implements neither `Debug` nor `Display`, wrap the enumerated set of
credential-derived values in it, replace the three sites that interpolate an upstream response
body into an error detail with a bounded, redacting shared helper, split the domain error's
internal diagnostic from its client-facing description so `/token` stops acting as a validation
oracle, and bound the client-chosen `X-Request-Id` in length and charset.

---

## Motivation

The repository already treats formatting as a disclosure channel: six types hand-implement
`Debug` purely so that a secret renders as `"<redacted>"` — `WebhookConfig`
(`crates/core/src/config.rs:351`), `InternalApiConfig` (`:394`), `OidcProviderConfig`
(`crates/core/src/domain/provider.rs:22`), `TokenResponse` (`crates/core/src/domain/token.rs:19`),
`ProviderTokens` (`:56`), and `AppleProvider` (`crates/providers/src/apple.rs:39`). Six correct
applications of a discipline is exactly the state in which the seventh is missed. `Session`
(`crates/core/src/domain/session.rs:4`) carries `refresh_token_hash` — the session lookup key —
and plainly derives `Debug`, so `#[instrument]`'s default argument capture publishes it: the
Valkey adapter records the whole struct at `crates/adapters/src/valkey/mod.rs:52`, and the LMDB
adapter names the digest as an explicit span field at `crates/adapters/src/lmdb/mod.rs:55` and
lets it auto-record at `:102` and `:140`. The three adapters that get it right rely on an
unenforced name match between the declared `fields(token_hash)` entry and the parameter name; a
rename defeats them silently. And `OidcProvider` (`crates/adapters/src/oidc/mod.rs:15`) holds a
configured `client_secret` with no `Debug` impl at all, derived or manual — protected only by
that absence until someone derives one.

The same conflation appears at the provider boundary and at the HTTP boundary. Three sites treat
an upstream non-2xx response body as trusted diagnostic text and move it into
`Error::ProviderError.detail`, which `crates/server/src/error.rs:74` writes to the error log in
full: the shared token-endpoint helper's `_ => raw_body` arm
(`crates/adapters/src/shared/token_endpoint.rs:53`), the OIDC revocation path
(`crates/adapters/src/oidc/mod.rs:251-258`), and the Apple revocation path
(`crates/providers/src/apple.rs:346-353`) — the last two with no classification arm at all, on
requests that carry the token being revoked and, for Apple, a freshly signed ES256 client
assertion. And `map_domain_error_inner` (`crates/server/src/error.rs:80-124`) clones the internal
`reason` or `detail` of every 4xx variant except `UserSuspended` straight into
`error_description`, so an unauthenticated caller
learns which validation step failed and gets its own unverified `kid` echoed back
(`crates/adapters/src/oidc/mod.rs:154`, `crates/providers/src/apple.rs:255`) — the one function
that genericises the `server_error` class and guards it with an `assert_ne!` does not apply the
same rule one arm over. Each of these is a place where someone had to remember something. The
structural answer is a type that cannot be formatted, so the compiler remembers instead.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md) | `Session.refresh_token_hash` becomes `Secret<String>`; `Session` gains a hand-written `Debug` |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | `SessionRepository` token-hash parameters become `&Secret<String>`; `Shared OIDC utilities` gains the bounded, redacting `upstream` helper |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | Middleware stack item 1: `X-Request-Id` bound and charset; Error mapping: `client_description` split, every class logged |
| [`.specs/service/specs/05-provider-system.md`](../service/specs/05-provider-system.md) | Both `revoke_token` paths and `exchange_code` route upstream bodies through the shared helper |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | Decisions: the per-type redacting `Debug` decision is superseded by `Secret<T>`; `[user_sync]` and `[internal_api]` wording |
| [`.specs/service/specs/07-telemetry-and-audit.md`](../service/specs/07-telemetry-and-audit.md) | New `Telemetry hygiene` section stating what may not enter a span, log, or error body |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) | Session-only stores: LMDB and Valkey span-field redaction |

No new canonical page. No change to
[`canonical-types.schema.json`](../service/specs/canonical-types.schema.json) — see `Type changes`.

Companion change spec on the same root cause, seen from the audit side:
[2026-08-05-audit_and_throttle_authentication_failures.md](2026-08-05-audit_and_throttle_authentication_failures.md).
That spec owns the audit failure `reason` — it must be a fixed classification string precisely
because `ProviderError`'s `Display` embeds the upstream response body verbatim. This spec owns
the logging, tracing, and error-response surface; neither restates the other's delta.

---

## Proposed changes

### `.specs/service/specs/01-domain-model.md` → Entities → Session (Modify)

> ```rust
> struct Session {
>     user_id: String,
>     refresh_token_hash: Secret<String>,   // SHA-256 hex; never the raw token
>     provider: String,
>     expires_at: DateTime<Utc>,
>     device_id: Option<String>,
>     user_agent: Option<String>,
>     ip_address: Option<String>,
>     created_at: DateTime<Utc>,
> }
> ```
>
> The raw refresh token exists only in memory during issuance and in the response to the client.
> Only the hash is stored, and it is stored as `Secret<String>` — a newtype that implements
> neither `Debug` nor `Display`, so no formatter, tracing macro, or `#[instrument]` argument
> capture can render it. `Session` therefore cannot derive `Debug`; it hand-implements one that
> prints `refresh_token_hash: "<redacted>"` and passes the remaining fields through. The
> serialized form is unchanged: `Secret<T>` is transparent to `serde`, so every store writes and
> reads the same 64-character hex string it did before. `device_id`, `user_agent`, and
> `ip_address` are still populated from the request context: the audit-context middleware
> captures them at the HTTP edge and the exchange flow threads them into the stored session.

### `.specs/service/specs/02-ports-and-adapters.md` → SessionRepository (Modify)

> ```rust
> async fn store_refresh_token(&self, session: &Session) -> Result<()>;
> async fn get_session_by_refresh_token(&self, token_hash: &Secret<String>) -> Result<Option<Session>>;
> async fn revoke_session(&self, token_hash: &Secret<String>) -> Result<()>;
> async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;
> async fn count_active_sessions(&self) -> Result<u64>;
> async fn cleanup_expired_sessions(&self) -> Result<u64>;  // returns rows deleted
> ```
>
> The token-hash parameters are `&Secret<String>` rather than `&str`, so an adapter that leaves
> them out of `skip(...)` fails to compile instead of publishing the session lookup key as a span
> field. An adapter reaches the raw digest through `expose()` at the point it builds a store key.

### `.specs/service/specs/02-ports-and-adapters.md` → Shared OIDC utilities (Modify)

> Reused by the OIDC and Apple providers. All outbound HTTP goes through a single shared
> `reqwest::Client` per process with a 5s connect timeout, a 10s total request timeout
> (compile-time constants, not configuration), and redirects disabled; a hung or slow provider
> fails the request rather than stalling `/token`. On the token-endpoint and revocation paths a
> non-2xx response is an error whose body reaches nothing but `upstream::error_detail`; success
> payloads are parsed only from 2xx bodies.
>
> - `jwks::JwksCache` — fetches and caches a remote JWKS behind a read/write lock with a TTL
>   (default 1h); `with_ttl` overrides. A non-2xx JWKS response is a `ProviderError` and is never
>   cached. When a token's `kid` is not in the cached set, the cache refetches once (rate-limited
>   by a 30s minimum refresh interval) before the provider rejects the token, so upstream key
>   rotation takes effect immediately instead of at TTL expiry.
> - `discovery::discover(issuer)` — fetches and parses `.well-known/openid-configuration` into
>   `DiscoveryDocument { issuer, token_endpoint, jwks_uri, revocation_endpoint }` and errors if
>   the document's `issuer` does not equal the configured issuer (RFC 8414 §3.3).
> - `token_endpoint::exchange_code(endpoint, client_id, client_secret, code, redirect_uri)` — the
>   standard form-encoded `grant_type=authorization_code` POST. A non-2xx response is turned into
>   a `ProviderError` detail by `upstream::error_detail`; a 2xx response without an `id_token` is
>   an error, not an empty string.
> - `http::read_bounded(response)` — reads a response body to at most `MAX_UPSTREAM_BODY_BYTES`
>   (64 KiB) and returns it as `Secret<String>`, so an upstream cannot choose how many bytes the
>   service retains and the body cannot be formatted by accident.
> - `upstream::error_detail(status, body)` — the only way to build a `ProviderError` detail from
>   upstream bytes. It prefers the structured RFC 6749 error object (`error`, optionally
>   `error_description`); failing that it returns the status, the body's byte length, and a
>   bounded excerpt (256 characters) produced by a redacting function that percent-decodes first
>   and then masks the values of `token`, `refresh_token`, `client_secret`, `code`, and any
>   bare compact JWS. It consumes the `Secret<String>` and returns a plain `String`, so it is the
>   single audited point at which upstream text becomes loggable.

### `.specs/service/specs/04-http-api.md` → Middleware stack, item 1 (Modify)

> 1. **Request ID** (`middleware/request_id.rs`) — reuse an inbound `X-Request-Id` only when it
>    is a plausible correlation identifier: non-empty, at most `MAX_REQUEST_ID_LEN` (128) bytes,
>    and drawn from `[A-Za-z0-9_-]`. Anything else — absent, empty, over-long, wrongly shaped, or
>    not visible ASCII — is discarded and a fresh UUIDv4 is generated instead; the request is
>    never failed over a malformed correlation header, and the rejected value is never logged.
>    Open a per-request `info_span` carrying `request_id` so all downstream logs — including the
>    `server_error` detail log — inherit it; echo in the response header.

### `.specs/service/specs/04-http-api.md` → Error mapping (Modify)

> `ApiError` wraps the domain `Error` (plus `UnsupportedGrantType`) and renders
> `{"error": <code>, "error_description": <detail>}` (RFC 6749 §5.2). The status and `error` code
> are as tabulated above; the `error_description` is **always** `Error::client_description()` — a
> stable `&'static str` per variant, drawn from a small fixed set, that never embeds caller input,
> library error text, provider key state, or cache internals. The internal `reason`/`detail` an
> adapter composed is never published. (`UnsupportedGrantType`, a route-level error with no
> domain counterpart, keeps its fixed static description — generic by construction, so the same
> rule holds.)
>
> Every mapped domain error — not only the `server_error` class — logs its full `Display` via
> `tracing::error!` (5xx) or `tracing::warn!` (4xx) inside the request span, so the log carries
> the request id and the operator loses no diagnostic power. A debug assertion checks that the
> rendered description equals `err.client_description()` for every arm, generalising the guard
> that previously protected only `server_error`.
>
> The consequence for a caller is that an unknown `kid`, a bad signature, an expired token, and a
> wrong audience are indistinguishable at `/token`: each is
> `400 {"error":"invalid_grant","error_description":"the provided grant could not be validated"}`.
> RFC 6749 §5.2 makes `error_description` optional and developer-facing, so genericising it
> breaks no conformance.

### `.specs/service/specs/05-provider-system.md` → OidcProvider behaviour (Modify)

> - `exchange_code` delegates to `shared::token_endpoint::exchange_code` (form-encoded
>   `authorization_code` POST with client credentials). A non-2xx upstream response yields a
>   detail built by `shared::upstream::error_detail`, never the raw body.
> - `revoke_token` POSTs to the discovered revocation endpoint with the client id. A non-2xx
>   response is read with `shared::http::read_bounded` and rendered through
>   `shared::upstream::error_detail`, so an intermediary that echoes the submitted form cannot put
>   the token being revoked into the error log.

### `.specs/service/specs/05-provider-system.md` → Tiers, Tier 2 (Modify)

> Apple is mostly OIDC but requires a freshly signed **ES256 client secret JWT** for each token
> endpoint call (`ClientSecretClaims { iss: team_id, sub: client_id, aud, iat, exp }`, ~5-minute
> lifetime, signed with the `.p8` key). `generate_client_secret` returns that assertion as
> `Secret<String>`, so it can be posted but not formatted. `revoke_token` sends the assertion
> alongside the token being revoked and renders any non-2xx response through
> `shared::upstream::error_detail`. It reuses the shared `JwksCache` for the standard ID-token
> validation parts.

### `.specs/service/specs/06-configuration.md` → Decisions (Modify)

Replace the per-type redaction decision:

> - *Secrets are unprintable by type.* **Credential-bearing config values are `Secret<T>`, a
>   newtype implementing neither `Debug` nor `Display`.** `WebhookConfig.secret`,
>   `InternalApiConfig.shared_secret`, and `OidcProviderConfig.client_secret` previously relied on
>   hand-written `Debug` impls rendering `"<redacted>"`; the newtype makes a leak a compile error
>   rather than a per-type discipline — the enclosing structs' `Debug` impls still elide the
>   secret field, but forgetting the elision now fails to compile instead of leaking.

### `.specs/service/specs/06-configuration.md` → Sections → `[user_sync]` and `[internal_api]` (Modify)

> ### `[user_sync]`
> `enabled` (bool), `adapter` (`webhook`), `[user_sync.webhook] { url, secret, timeout?,
> retries? }`. The `secret` is a `Secret<String>` and cannot be formatted.
>
> ### `[internal_api]`
> `enabled` (false — internal routes are not mounted unless true, regardless of `server.role`;
> a `role = "admin"` instance with the flag off serves only `/health`), `auth_method`
> (`shared_secret`), `shared_secret` (a `Secret<String>`; it cannot be formatted, must be
> non-empty when the internal API is served, and internal auth compares it in constant time via
> `subtle`).

### `.specs/service/specs/07-telemetry-and-audit.md` → Telemetry hygiene (Add)

Add after the `Telemetry (telemetry::init_telemetry)` section:

> ## Telemetry hygiene
>
> Two rules bound what the observability plane may carry, and both are enforced by types rather
> than by convention.
>
> **Credential-derived values cannot be formatted.** `Secret<T>` (`crates/core/src/secret.rs`)
> implements no `Debug`, no `Display`, and no `ToString`. `tracing` records a span field through
> one of those traits, so a `Secret<T>` reaching `tracing::info!(?x)`, `%x`, `format!`, or
> `#[instrument]`'s default argument capture is a compile error. The values it wraps are the
> session refresh-token hash, the raw refresh token at issuance, the three configured secrets
> (`user_sync.webhook.secret`, `internal_api.shared_secret`, `providers.<name>.client_secret`),
> Apple's generated client assertion, and every upstream response body read at a provider
> boundary. `expose()` unwraps deliberately and is legible in review; the type prevents accident,
> not intent.
>
> **A client-facing description is a different value from an internal diagnostic.**
> `Error::client_description()` is the only string that crosses the public HTTP boundary; the full
> `Display` is logged under the request span. See
> [04-http-api.md](04-http-api.md) → Error mapping.
>
> Adapter instrumentation states its argument capture explicitly: every `#[instrument]` on a
> session-repository method names each argument in `skip(...)`, and re-projects only
> non-sensitive values into `fields(...)`. Declaring a bare field name (`fields(token_hash)`) keeps the log schema stable —
> the name appears, the value never does — but is treated as a schema aid, not as the control;
> the control is the type.

### `.specs/service/specs/08-persistence.md` → Session-only stores (Modify)

> - **LMDB (`adapters/lmdb`)** — embedded `heed` store with two named databases, `sessions`
>   (hash → session) and `user_sessions` (user → set of hashes for revoke-all). Constructed with a
>   path and a max map size in MB.
> - **Valkey/Redis (`adapters/valkey`)** — `fred` client; keys `{prefix}session:{hash}`, a
>   `{prefix}user_sessions:{user_id}` set, and a `{prefix}active_sessions` counter. A session write
>   applies the hash, its TTL, the user-set membership, an `INCR` of the counter, and a bump of the
>   user set's own TTL to the greatest member expiry — atomically (single pipeline). The set-TTL
>   bump uses `EXPIRE … GT` (only-extend), so a concurrent shorter-lived write can never shorten
>   the set's life, and idle users' index sets expire on their own. A session whose `expires_at` is
>   not in the future is rejected, so no TTL-less key is ever created. `count_active_sessions`
>   reads the counter, which is maintained by `INCR` on store and `DECR` on explicit revoke;
>   natural TTL expiry cannot decrement it, so it drifts upward between cleanups.
>   `cleanup_expired_sessions` prunes `user_sessions` set members whose session key no longer
>   exists, reconciles the counter by recomputing it from a SCAN of live `{prefix}session:*` keys,
>   and returns the number of members pruned; session bodies themselves need no sweep.
>
> Both implement `SessionRepository` only and are selected via `[session_repository]`.
>
> Every session adapter instruments its three session methods identically:
> `#[instrument(skip(self, session), fields(user_id = %session.user_id))]` on the write path and
> `#[instrument(skip(self, token_hash), fields(token_hash))]` on the lookup and revoke paths. The
> token hash and the session's client provenance (`ip_address`, `user_agent`, `device_id`) never
> become span field values on any backend.

---

## Type changes

No canonical-schema change. `Secret<T>` is transparent to `serde` — it serializes and
deserializes as the wrapped `T` — so `Session.refresh_token_hash` keeps its 64-character
lowercase-hex string form on every store, and its plain string shape (`NonEmptyString` in
[`canonical-types.schema.json`](../service/specs/canonical-types.schema.json), `string` in
`schemas/datamodel.schema.json`) is untouched. No stored record, wire body, or migration
changes.

The new type, for reference:

```rust
/// A value that must never reach a log line, a span field, or an error string.
///
/// Implements neither `Debug` nor `Display`, so `tracing`'s value capture — including
/// `#[instrument]`'s default argument recording — cannot render it. `serde` support is
/// transparent, because persistence and the log stream are different trust domains.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(value: T) -> Self { Self(value) }
    pub fn expose(&self) -> &T { &self.0 }
    pub fn into_inner(self) -> T { self.0 }
}
```

`PartialEq` is implemented only for `Secret<String>`, in constant time via `subtle`, so a
comparison cannot become a timing oracle.

### The values it wraps

| Value | Where it lives today | Becomes | serde needed |
|---|---|---|---|
| `Session.refresh_token_hash` | `crates/core/src/domain/session.rs:8` (`String`) | `Secret<String>` | yes — every store persists it |
| `SessionRepository` token-hash parameters | `crates/core/src/ports/repository.rs:26,27` (`&str`) | `&Secret<String>` | no |
| the refresh token minted at issuance | `crates/core/src/service/exchange.rs:293` (`String`) | `Secret<String>` | yes — one field of `TokenResponse` |
| `WebhookConfig.secret` | `crates/core/src/config.rs:325` (`String`) | `Secret<String>` | deserialize only |
| `InternalApiConfig.shared_secret` | `crates/core/src/config.rs:391` (`Option<String>`) | `Option<Secret<String>>` | deserialize only |
| `OidcProviderConfig.client_secret` | `crates/core/src/domain/provider.rs:11` (`Option<String>`) | `Option<Secret<String>>` | deserialize only |
| `OidcProvider.client_secret` | `crates/adapters/src/oidc/mod.rs:18` (`Option<String>`) | `Option<Secret<String>>` | no |
| Apple's generated client assertion | `crates/providers/src/apple.rs:187` (returns `Result<String>`) | `Result<Secret<String>>` | no |
| an upstream response body at a provider boundary | `raw_body` / `body` locals at `token_endpoint.rs:39`, `oidc/mod.rs:253`, `apple.rs:348` | `Secret<String>` from `shared::http::read_bounded` | no |

Wrapping `Session.refresh_token_hash` removes `Session`'s `#[derive(Debug)]`; it gains a
hand-written `Debug` eliding that field. `TokenResponse` and `ProviderTokens` keep their existing
hand-written `Debug` impls until their remaining `String` fields are wrapped — out of scope here.

---

## Implementation notes

Order matters: steps 1 and 2 are independent of the type and should land first.

1. **Ship the two span redactions immediately**, without waiting for `Secret<T>`. At
   `crates/adapters/src/lmdb/mod.rs:55` drop `token_hash` from the `fields(...)` list, keeping
   `user_id`; at `:102` and `:140` use `#[instrument(skip(self, token_hash), fields(token_hash))]`.
   At `crates/adapters/src/valkey/mod.rs:52` use
   `#[instrument(skip(self, session), fields(user_id = %session.user_id))]`; at `:141` and `:211`
   use `#[instrument(skip(self, token_hash), fields(token_hash))]`. Naming the argument in
   `skip(...)` as well as `fields(...)` is deliberate: the sibling adapters' redaction
   (`dynamo/mod.rs:653,671`, `postgres/mod.rs:522,536`, `sqlite/mod.rs:516,532`) relies on a name
   collision that a rename defeats silently.
2. **Bound the request id.** In `crates/server/src/middleware/request_id.rs`, add
   `MAX_REQUEST_ID_LEN: usize = 128` and an `is_acceptable_request_id(&str) -> bool` predicate
   (non-empty, within the bound, `[A-Za-z0-9_-]` only), and replace the `.filter(|s| !s.is_empty())`
   at `:31` with it. The predicate subsumes the emptiness filter, so the `assert!` at `:38-41` and
   its regression test keep holding. Rejection stays silent — logging the rejected value would
   reintroduce the unbounded field the bound exists to remove.
3. **Add the bounded, redacting upstream helper.** New `crates/adapters/src/shared/upstream.rs`
   with `MAX_UPSTREAM_EXCERPT: usize = 256` and `error_detail(status, Secret<String>) -> String`;
   add `read_bounded` with `MAX_UPSTREAM_BODY_BYTES: usize = 65_536` to
   `crates/adapters/src/shared/http.rs` (accumulate `response.bytes_stream()` to the ceiling rather
   than an unbounded `text()`). The redactor percent-decodes before masking — an echoed form
   returns `token=1%2F%2F…`, so a rule that matches the literal value passes while the leak
   remains. Export both from `crates/adapters/src/shared/mod.rs`.
4. **Route the three sites through it.** `crates/adapters/src/shared/token_endpoint.rs:44-58`
   (replace the `_ => raw_body` arm), `crates/adapters/src/oidc/mod.rs:251-258`, and
   `crates/providers/src/apple.rs:346-353` (both of which have no classification arm at all today).
   Keep the existing `exchange_code_surfaces_oauth_error_on_non_2xx` test passing — conformant
   RFC 6749 error objects must still surface `error` and `error_description`.
5. **Split the error type.** Add `Error::client_description(&self) -> &'static str` in
   `crates/core/src/error.rs`. In `crates/server/src/error.rs`, have `map_domain_error_inner`
   return `err.client_description().to_string()` for every arm — including the 4xx arms at
   `:82-96`, `:97-100` (`UnknownProvider`), `:120-124` (`NotFound`), and the `Conflict` arm — and
   lift the log and the guard at `:56-75` out of the `if error_code == "server_error"` block so
   they cover every class, logging 5xx at `error` and 4xx at `warn` — the `assert_ne!` against
   the full `Display` generalising to the debug assertion that each arm's rendered description
   equals `err.client_description()`. Two existing tests
   codify the current leak and must be updated:
   `invalid_grant_emits_no_server_error_detail_log` asserts
   `description == "code already used"` (`crates/server/src/error.rs:334`), and the integration
   test at `crates/server/tests/routes.rs:148` asserts the body's `error_description`
   `contains("code")`.
6. **Introduce `Secret<T>`** in `crates/core/src/secret.rs`, re-exported from `crates/core/src/lib.rs`.
   Land the wraps one value at a time in the order of the `Type changes` table; the compiler
   enumerates the call sites for each. `Session`'s `#[derive(Debug)]` at
   `crates/core/src/domain/session.rs:4` is the first thing to break — that break is the point.
7. **Tests.** A `trybuild` compile-fail suite asserting that `tracing::info!(?secret)` and
   `format!("{secret}")` do not compile is the proof that the control is structural. Add a leak
   corpus that drives a store, a refresh, a revoke, and an upstream error through every
   `SessionRepository` implementation under a capturing subscriber with `FmtSpan::CLOSE` enabled —
   the stock subscriber's `FmtSpan::NONE` would otherwise let the assertion pass vacuously — and
   asserts the captured output contains no sentinel digest, token, or secret, matching after
   percent-decoding. Add request-id tests for a 64 KiB value, a wrongly-shaped-but-legal ASCII
   value, exactly `MAX_REQUEST_ID_LEN` bytes and one byte more, and keep
   `preserves_existing_request_id` passing so a fix cannot silently disable reuse. Add error-body
   tests asserting an unknown `kid` is not echoed and that signature, `exp`, and `aud` failures are
   indistinguishable to the caller.

Evidence: sealed scan bundle `.security/oidc-exchange/53cbdec9_20260804T102454Z/`, findings
`g1-upstream-token-body-written-to-logs`, `g1-upstream-body-logged-oidc-revoke`,
`g1-upstream-body-logged-apple-revoke`, `g1-token-error-response-oracle`,
`g3-lmdb-token-hash-span-exposure`, `g3-valkey-session-span-exposure`,
`g1-request-id-unbounded-and-client-chosen`; structural context
`hardening/proposals/observability-contract.md` (Option 2); threat model
`artifacts/01_context/threat_model.md` (invariants I10, I15, I16).

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**`.
2. Add the new `Telemetry hygiene` section to 07-telemetry-and-audit.md.
3. Replace the superseded redacting-`Debug` decision in 06-configuration.md rather than appending
   beside it.
4. No schema change to fold in.
5. Flip this file's `**Status:**` to `Merged`, stamp `**Merged:** YYYY-MM-DD`, and move it to
   `.specs/changes/merged/`.
6. Update `.specs/README.md`'s Change specs table.

---

## Assumptions and open questions

### Assumptions

- The shipped JSON subscriber (`crates/server/src/telemetry.rs:17-33`) escapes field values, so
  none of these leaks is also a log-injection primitive; a plain-text subscriber would change that
  conclusion, and nothing in the repository installs one.
- `ed25519_dalek::SigningKey`'s `Debug` deliberately omits the secret key
  (`finish_non_exhaustive`), so `LocalKeyManager`'s derived `Debug`
  (`crates/adapters/src/local_keys/mod.rs:9`) leaks nothing today. That is an upstream discipline
  this repository depends on but does not control.
- No consumer depends on `/token`'s current `error_description` text. The strings are internal
  diagnostics; RFC 6749 §5.2 marks the field optional and developer-facing.
- `serde` support on `Secret<T>` is not itself a disclosure channel: `tracing` captures values
  through `Debug`/`Display`, never through `Serialize`.

### Decisions

- *One generic type, not a family of newtypes.* **`Secret<T>` wraps every credential-derived
  value; there is no separate `TokenHash` or `SessionRef`.** The property being enforced —
  "cannot be formatted" — is identical for all of them, and distinct newtypes would multiply the
  migration without adding a property the surrounding function signatures do not already give.
- *`Secret<T>` keeps `serde`, drops `Debug` and `Display`.* **Persistence and the log stream are
  different trust domains.** Dropping `Serialize` would break every session store and the token
  response for no gain, since `tracing` cannot reach a value through `serde`.
- *Two redactions ship ahead of the type.* **The LMDB and Valkey `#[instrument]` fixes land
  immediately, before `Secret<T>` exists.** They are one-line-scale changes with no design
  dependency, and the type's migration is broad enough that waiting would leave a known leak open
  across several releases.
- *A rejected request id yields a fresh one, never a failed request.* **An over-long or wrongly
  shaped `X-Request-Id` is discarded and a UUIDv4 is generated.** The header is a correlation aid
  with no protocol semantics; rejecting the request would convert an observability nicety into an
  availability dependency on well-behaved clients, and 4xx-ing an otherwise valid `/token` call
  over a diagnostic header is a worse outcome than losing one trace hop.
- *Rejection is silent.* **The rejected id is not logged, not even truncated.** Logging it
  reintroduces the unbounded attacker-chosen log field the bound exists to remove.
- *Bodies are bounded at read, not at format.* **`read_bounded` caps the upstream body at 64 KiB
  before anything can hold it.** A redactor alone still lets a hostile upstream choose how many
  bytes the process buffers, and the cap also serves the bounded-body requirement the threat model
  states.
- *`error_detail` is the only constructor.* **A `ProviderError` detail built from upstream bytes
  can only be produced by `shared::upstream::error_detail`, which consumes a `Secret<String>`.**
  Fixing the three sites independently is what allowed the pattern to be copied twice already; a
  single audited constructor gives a fourth copy nowhere to come from.
- *Every class is logged, not only `server_error`.* **4xx errors log their full `Display` at
  `warn` under the request span.** Genericising the client body must not cost the operator the
  diagnostic; the request id already correlates the generic body with the full detail.
- *`fields(token_hash)` is schema, not control.* **The bare field name stays for log-schema
  stability, but the control is the type.** The name-collision behaviour it relies on is defeated
  by a parameter rename, which the scan reproduced by accident.

### Open questions

- Should the correlation key be server-authored unconditionally, with any inbound id recorded in a
  separately named `client_request_id` field? That closes the correlation-key-choice route
  completely rather than bounding it, but it changes the documented echo semantics — an inbound id
  would no longer come back on the response — and cross-fleet tracing depends on that echo. The
  bound in this spec is needed either way.
- `Session.device_id`, `user_agent`, and `ip_address` are client-asserted strings with no length
  bound, persisted on the session and recorded in audit events. Bounding them, and distinguishing
  observed from asserted provenance, is a `ClientAddr`-shaped change that needs
  `into_make_service_with_connect_info` at the serve call; it is not in this spec's scope.
- `TokenResponse.access_token` and `ProviderTokens`' three fields still rely on hand-written
  `Debug` impls (`TokenResponse.refresh_token` is already wrapped by this spec). Wrapping them
  in `Secret<String>` would retire the last of the hand-rolled redactions, but touches the FFI
  and binding surfaces; deferred.
