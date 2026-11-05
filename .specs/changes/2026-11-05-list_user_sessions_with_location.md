# Change: List a user's sessions and record caller-supplied session location

**Status:** Accepted · **Date:** 2026-11-05 · **Owner:** Ant Stanley · **Target:** service (crates/core, crates/adapters, crates/server, crates/ffi, bindings), schemas

Add `GET /internal/users/{id}/sessions`, which lists a user's live sessions (one per refresh-token
family, newest first, capped at 50) with an opaque stable id and an optional `location`. Add a
provider-neutral `SessionLocation { city, region, country }` to `Session`: callers of the exchange
flow supply it, rotation carries it forward, and every session adapter persists it. The HTTP server
is one such caller; it fills the value from operator-named request headers under `[session.location]`
(off by default, trust-gated), and may also take the client address from a named header, recorded as
`Reported` (trusted-proxy peer) or `Asserted` (escape hatch) provenance. No cloud-provider name appears
in code. Resolves GitHub issue #53. Merge order relative to
[`2026-08-25-close_r2_audit_code_divergences.md`](2026-08-25-close_r2_audit_code_divergences.md) is
settled in the Merge plan.

---

## Motivation

A consuming application wants to show a signed-in user where each session came from. Today the only
way to enumerate sessions is to read the repository's tables directly, which couples the consumer to
an adapter's private key layout and forces it to hold read access to the auth store. Sessions record
`ip_address` and `user_agent` but no location. A proxy in front of the service often already knows the
viewer's city, region, and country; the service only has to record what a trusted caller reports.

The design keeps provider knowledge out of the service. The core takes a plain optional value from its
caller; header names are configuration, not code. Header-derived values are only as trustworthy as the
path the request took, and the service cannot verify that its origin is reachable solely through the
proxy. The header mapping is therefore opt-in and gated by the existing trusted-proxy machinery, with
one explicit, documented escape hatch for platforms (Lambda behind an API gateway) whose peer address is
not a stable, listable CIDR.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md) | Add `SessionLocation` and `SessionSummary`; `Session` gains `location`; `ExchangeRequest` gains `location`; `ClientAddr` gains `Reported`; Session lifecycle notes location survives rotation |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | `SessionRepository` gains `list_live_sessions`; conformance suite gains listing and location cases |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | Exchange stores location; refresh copies it; new `admin_list_user_sessions` flow |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | New internal route; audit-context middleware step reads the location and address headers; the per-IP throttle paragraph admits `Reported` as a key |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | New `[session.location]` section and validation rules; Committed default block and Defaults summary rows |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) | `location` column/attribute per adapter and idempotent migration |
| [`.specs/service/specs/canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | `SessionLocation`, `SessionSummary`; `Session.location`; `ExchangeRequest.location`; `ClientAddrSource` gains `reported` |
| `schemas/datamodel.schema.json` | Code-side artifact: `sessions` gains nullable `location`; `ip_address_source` enum gains `reported` |
| `config/default.toml` | Commented `[session.location]` block, all header names empty |

---

## Proposed changes

### `01-domain-model.md` → Entities → Session (Modify)

> `Session` gains `location: Option<SessionLocation>` (`#[serde(default)]`; `None` for rows written
> before this field existed and whenever the caller supplied none). `location` is set at exchange from
> `ExchangeRequest.location` and copied unchanged into every replacement generation at rotation, exactly
> as `device_id`, `user_agent`, and `ip_address` are.

### `01-domain-model.md` → Entities → SessionLocation and SessionSummary (Add)

> ```rust
> struct SessionLocation {
>     city: Option<String>,
>     region: Option<String>,
>     country: Option<String>,
> }
>
> struct SessionSummary {
>     id: String,                         // "ses_…", opaque and stable per family
>     provider: String,
>     user_agent: Option<String>,
>     ip_address: Option<String>,
>     location: Option<SessionLocation>,
>     created_at: DateTime<Utc>,
>     expires_at: DateTime<Utc>,
> }
> ```
>
> `SessionLocation` is plain data with no knowledge of headers or providers. Its only constructor,
> `SessionLocation::new(city, region, country) -> Option<Self>`, strips control characters, trims
> whitespace, truncates each part to 80 characters, drops parts that are empty afterwards, and returns
> `None` when no part remains. Core records the value as given and makes no trust decision: the caller
> must pass only data it has established.
>
> `SessionSummary` is a read model derived from the live `Session` of one family. It has no refresh-token
> hash, `device_id`, or `family_id`. Its `id` is `"ses_"` followed by the unpadded base64url encoding of
> the first 16 bytes of `SHA-256("oidc-exchange/session-id/v1:" ‖ family_id)`. The id is a pure function
> of the family, so it is stable across reads and rotations; it is one-way and shares no input with the
> refresh token or its hash.

### `01-domain-model.md` → Entities → Exchange request types (Modify)

> `ExchangeRequest` gains `location: Option<SessionLocation>` (alongside `client_addr`, `user_agent`,
> and `device_id`), supplied by the caller and stored on the resulting session. `RefreshRequest` does not: rotation inherits the live generation's location.

### `01-domain-model.md` → Entities → SecurityEvent and ClientAddr (Modify)

> `ClientAddr` gains `Reported(IpAddr)`: an address read from a configured header
> (`session.location.ip_header`) on a request whose observed peer is in `server.trusted_proxies`.
> `ClientAddrSource` serializes it as `reported`. Because the peer is trusted, `Reported` is eligible as
> a rate-limit key, like `Peer` and `Forwarded`. A header value accepted only through
> `trust_headers_from_any_peer` is recorded as `Asserted` instead (never a rate-limit key). In both
> cases `audit_address()` returns the address, so it is stored as the session's `ip_address`.
> The rate-limit sentence reads: `Peer(IpAddr)`, `Forwarded(IpAddr)` and `Reported(IpAddr)` may
> become rate-limit keys; `Asserted(value)` and `Unknown` may not. `ClientAddrSource` serializes as
> `peer`, `forwarded`, `reported`, `asserted`, or `unknown`. When both a `Reported` and a `Forwarded`
> address are available, `Reported` wins.

### `01-domain-model.md` → Lifecycles → Session (Modify)

> Add after the rotation paragraph: a rotated generation inherits the family's `location` unchanged, so
> the place a sign-in was created from is stable for the life of the family.

### `02-ports-and-adapters.md` → SessionRepository (Modify)

> ```rust
> async fn list_live_sessions(&self, user_id: &str, now: DateTime<Utc>, limit: usize) -> Result<Vec<Session>>;
> ```
>
> Returns the live generation of each of the user's families whose `expires_at` is after `now`, ordered
> by `created_at` descending, at most `limit` entries. Retired generations are never returned, so a
> rotated family appears once. The result is strongly consistent with the most recent write. Adapters do
> not mint ids; core maps results to `SessionSummary`.

### `02-ports-and-adapters.md` → Session-store conformance suite (Modify)

> The suite gains cases that every adapter (DynamoDB, PostgreSQL, SQLite, LMDB, Valkey) must pass: a
> rotated family lists once with its live generation; expired families are omitted; ordering is newest
> first and the limit is honoured; `location` round-trips and survives rotation; a row written without
> `location` lists with `None`; on LMDB and Valkey, an index member whose session is missing or
> expired is skipped.

### `03-service-flows.md` → Token exchange and Admin flows (Modify, Add)

> Exchange stores `request.location` on the generation-0 session. Refresh copies `live.location` into the
> replacement. `admin_list_user_sessions(user_id)` returns `None` when the user is absent or
> soft-deleted, and otherwise lists up to 50 live sessions as `SessionSummary` values.

### `04-http-api.md` → Routes → Internal (Add row)

> | GET | `/internal/users/{id}/sessions` | list the user's live sessions, newest first, at most 50; 404 for an unknown or soft-deleted user |
>
> Response `200`: `{ "sessions": [SessionSummary…] }` with `location` omitted when none was recorded.
> Auth and error behaviour are those of every other internal route.

### `04-http-api.md` → Middleware stack → per-IP throttle paragraph (Modify)

> Replace "only `Peer` and `Forwarded` values become rate-limit keys" with "only `Peer`, `Forwarded`
> and `Reported` values become rate-limit keys". "Asserted or unknown addresses are not throttled"
> is unchanged.

### `04-http-api.md` → Middleware stack → Audit context (Modify)

> When `session.location` names headers, the audit-context middleware reads them only if the request's
> peer is in `server.trusted_proxies`, or `session.location.trust_headers_from_any_peer = true`.
> Header values are percent-decoded (invalid UTF-8 replaced), then passed through
> `SessionLocation::new`. When `ip_header` is named and its value, after stripping a trailing `:port`
> (and brackets for IPv6), parses as an `IpAddr`, the resolved address is `Reported` when the peer is in
> `server.trusted_proxies` and `Asserted` when the header was accepted only through
> `trust_headers_from_any_peer`; otherwise the existing `Peer`/`Forwarded` resolution stands. Absent, empty, or unparseable headers yield no location
> and leave address resolution unchanged. With no header named (the default) the middleware does
> nothing new.

### `06-configuration.md` → Sections → `[session.location]` (Add)

> `city_header`, `region_header`, `country_header`, `ip_header` (all `""` by default; an empty name
> disables that field), and `trust_headers_from_any_peer` (`false`). Header names must be valid HTTP
> header names, case-insensitive, and distinct from one another. Setting `trust_headers_from_any_peer =
> true` with no header named is rejected at load. The key states a deployment fact the service cannot
> verify: the origin is unreachable except through the proxy that sets these headers.

### `06-configuration.md` → Committed default and Defaults summary (Modify)

> The committed default gains a `[session.location]` block with `city_header = ""`,
> `region_header = ""`, `country_header = ""`, `ip_header = ""`, and
> `trust_headers_from_any_peer = false`. The Defaults summary gains a row:
> `session.location.*_header` / `trust_headers_from_any_peer` | `""` / `false`.

### `08-persistence.md` → per adapter (Modify)

> `sessions` gains a nullable `location` column: JSON text on SQLite, `jsonb` on PostgreSQL, added by an
> idempotent `ALTER TABLE … ADD COLUMN`, as `family_id`, `generation`, and `rotated_at` were. DynamoDB
> session items gain an optional `location` map attribute; listing reads the user item's `families`
> roster with a strongly consistent read, then batch-gets the live generations, and never queries GSI1.
> LMDB and Valkey serialize `location` inside the stored session record (absent in older records). Valkey
> and LMDB list through the same per-user index `revoke_all_user_sessions` uses
> (`user_sessions`), skipping members whose session record is missing or expired, since those indexes
> can outlive the sessions they name. Existing
> rows keep `NULL`.

---

## Type changes

```json
{
  "$comment": "Fragment for 2026-11-05-list_user_sessions_with_location. Folds into .specs/service/specs/canonical-types.schema.json on merge.",
  "$defs": {
    "SessionLocation": {
      "type": "object",
      "additionalProperties": false,
      "minProperties": 1,
      "properties": {
        "city": { "type": "string", "minLength": 1, "maxLength": 80 },
        "region": { "type": "string", "minLength": 1, "maxLength": 80 },
        "country": { "type": "string", "minLength": 1, "maxLength": 80 }
      }
    },
    "SessionSummary": {
      "type": "object",
      "additionalProperties": false,
      "required": ["id", "provider", "created_at", "expires_at"],
      "properties": {
        "id": { "type": "string", "pattern": "^ses_[A-Za-z0-9_-]{22}$" },
        "provider": { "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString" },
        "user_agent": { "type": ["string", "null"] },
        "ip_address": { "type": ["string", "null"] },
        "location": { "$ref": "#/$defs/SessionLocation" },
        "created_at": { "$ref": "../../canonical-types.schema.json#/$defs/Timestamp" },
        "expires_at": { "$ref": "../../canonical-types.schema.json#/$defs/Timestamp" }
      }
    },
    "ExchangeRequest": {
      "$comment": "Modified: adds one optional property to the canonical ExchangeRequest (additionalProperties false).",
      "properties": {
        "location": { "oneOf": [{ "$ref": "#/$defs/SessionLocation" }, { "type": "null" }] }
      }
    },
    "ClientAddrSource": {
      "$comment": "Modified: enum gains 'reported' (peer, forwarded, reported, asserted, unknown). Description gains: reported is read from session.location.ip_header behind server.trusted_proxies and is rate-limit eligible.",
      "enum": ["peer", "forwarded", "reported", "asserted", "unknown"]
    },
    "Session": {
      "$comment": "Modified: adds one optional property.",
      "properties": {
        "location": { "oneOf": [{ "$ref": "#/$defs/SessionLocation" }, { "type": "null" }] }
      }
    }
  }
}
```

---

## Implementation notes

1. `crates/core/src/domain/session.rs`: add `SessionLocation` (sanitising constructor) and
   `Session.location`; add `SessionSummary` and the id derivation next to `new_family_id`.
2. `crates/core/src/ports/repository.rs`: add `list_live_sessions`; implement it in the mock store in
   `crates/test-utils`; extend `crates/test-utils/src/session_contract.rs` with the cases above and run it
   from every adapter's tests.
3. Adapters: SQLite (`crates/adapters/src/sqlite/mod.rs`, migration beside `SESSION_FAMILY_COLUMNS`
   near line 168), PostgreSQL (`MIGRATIONS` in `crates/adapters/src/postgres/mod.rs`), DynamoDB
   (`schema.rs` session item encode/decode near lines 147-250; roster read as in `revoke_family`),
   LMDB, Valkey.
4. `crates/core/src/service/exchange.rs` (store `location`) and `refresh.rs:284-286` (copy it forward).
5. `crates/core/src/service/user_admin.rs`: `admin_list_user_sessions`.
6. `crates/core/src/config.rs`: `SessionLocationConfig` and load-time validation; commented block in
   `config/default.toml`.
7. `crates/server/src/middleware/audit_context.rs`: header read, decode, `Reported` provenance; thread
   `location` through `AuditContext` into `routes/token.rs:245-276`. Add the route to
   `routes/internal.rs:33`.
8. FFI and bindings: the exchange request is built by the shared normaliser in
   `crates/server/src/bootstrap.rs` and `middleware/audit_context.rs` (neither `crates/ffi` nor
   `bindings` handles `device_id` today); accept an optional `location` object there so non-HTTP
   embedders can supply it, and extend the differential corpus.
   Also update `schemas/datamodel.schema.json` (`sessions.location`, `ip_address_source` enum),
   `crates/server/src/middleware/public_throttle.rs`/`throttle.rs` key selection to admit `Reported`,
   and `config/default.toml`.
9. Tests: `crates/server/tests/internal.rs` for auth parity, 404, response shape, and a check that no
   JSON value in a listing equals any stored token or hash; middleware tests for percent-encoding,
   control characters, over-length parts, untrusted-peer rejection, the escape hatch, and port/bracket
   stripping on `ip_header`. Add a contract fixture under `crates/test-utils` for the response.
10. Order: core and port, mock and contract suite, SQL adapters, DynamoDB, LMDB and Valkey, service flow,
    config, middleware and route, FFI.

---

## Merge plan

0. Ordering: this change and `2026-08-25-close_r2_audit_code_divergences.md` both rewrite the
   `ExchangeRequest` sketch and schema entry, the `ClientAddr` prose, and the `04` throttle paragraph.
   Merge the R2 change first. Before merging this one, re-resolve its `ExchangeRequest`, `ClientAddr`,
   and throttle Modify blocks against the merged text (the R2 change replaces `ip_address` with
   `client_addr` and adds `ClientAddr`/`provider_access_token` to the schema).
1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**`.
2. Fold the `Type changes` `$defs` into `canonical-types.schema.json`; update
   `schemas/datamodel.schema.json` in the code change.
3. Flip `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, move this file to `.specs/changes/merged/`.
4. Update `.specs/README.md`: change the row's link and status.

---

## Assumptions and open questions

### Assumptions

- A deployment that enables header mapping controls the proxy that sets those headers, and strips or
  overwrites any client-supplied copies.
- An access token's `sid` claim already exposes `family_id` to its holder, so a one-way function of
  `family_id` discloses nothing the holder does not have.

### Decisions

- *Location is caller-supplied plain data.* **`SessionLocation` and `ExchangeRequest.location`, not a
  proxy-specific reader in core.** Keeps provider names out of the service and lets FFI and library
  embedders supply the value.
- *Sanitising lives in the constructor.* **`SessionLocation::new` bounds and cleans every value.** All
  callers get identical hygiene and core never stores unbounded caller text; percent-decoding is a
  transport concern and stays in the server.
- *Header mapping is opt-in and trust-gated.* **Empty by default; honoured only from `trusted_proxies`
  peers unless `trust_headers_from_any_peer`.** Reuses the existing trust model; the escape hatch is
  explicit rather than implied.
- *Reported addresses are rate-limit keys only from trusted peers.* **`Reported` (trusted-CIDR peer) is
  throttle-eligible; a header accepted via `trust_headers_from_any_peer` is recorded as `Asserted` and
  is not.** A trusted CIDR is a verifiable fact; the escape hatch is an unverifiable operator
  assertion, so a spoofable value must not choose a throttle bucket.
- *Single-session revocation is a separate change.* **`DELETE /internal/users/{id}/sessions/{sid}` is not
  part of this change.** It reuses the opaque `id` (resolved by recomputing ids over the user's
  families) and needs its own audit and flow design.
- *Session id is an unkeyed domain-separated hash of the family id.* **`ses_` + 128 bits of SHA-256.**
  Stable and opaque with no key to configure, rotate, or lose, which matters because `shared_secret` is
  only one of several internal auth methods. It is independent of the refresh token and its hash.
- *Listing reads live rows only.* **One entry per family because retired generations are separate
  records.** No dedupe step is needed.
- *DynamoDB lists through the roster, not GSI1.* **Strongly consistent read of the `families` map.**
  Matches `revoke_family`'s reasoning about eventually consistent indexes.
- *All five adapters ship together.* **DynamoDB, PostgreSQL, SQLite, LMDB, Valkey.** One conformance
  suite keeps behaviour identical.

### Open questions

(None at this stage.)
