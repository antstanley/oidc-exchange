# Change: One owned outbound HTTP boundary

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** crates/adapters, crates/providers (service)

Make every byte this service sends to, and accepts from, a remote host pass through a
transport that owns the integrity properties rather than re-deciding them per call site.
Inbound: introduce a `ProviderTransport` that is the only way the workspace issues an
outbound provider request, and a `VerificationKeySet` that is the only way a JWK becomes a
verification key — replacing two independent `find_jwk` copies that select on `kid` alone,
bounding the JWKS and discovery success bodies, constraining discovery-supplied endpoints to
a per-provider origin set pinned at config load, and taking the JWKS cache's write guard off
the network path. Outbound: bind each user-sync webhook delivery to a single delivery
occasion by signing a timestamp and a delivery id alongside the body, and oblige receivers to
check both.

---

## Motivation

`crates/adapters` performs outbound HTTP on two boundaries — B2, the provider leg, and B9,
the egress leg to the operator's webhook receiver — and on both, the properties that make the
traffic trustworthy are implemented at the call site rather than owned by a type. The result
is exactly what per-call-site security produces. `find_jwk` exists twice, in
`crates/adapters/src/oidc/mod.rs:52` and `crates/providers/src/apple.rs:67`, and neither copy
consults `use`, `key_ops`, or `kty`; the two have already drifted, one growing a nine-arm
`alg` match with an inference fallback and the other a two-arm match with none, which is why
the threat model's contradiction **C12** — do the two validators accept the same tokens? — is
recorded as unresolved. `crates/adapters/src/shared/` holds three outbound fetchers; one
bounds its response status, none bounds its size, and the one that reads the discovery
document chooses the host for the other two. `JwksCache::get_keys` obtains single-flight as a
side effect of a data lock and therefore holds that lock across the fetch, while `refresh()`
sitting eleven lines below it gets the ordering right. And the webhook signs *what* it is
saying without signing *which delivery* it is, so a captured `(body, signature)` pair stays
valid forever.

These belong in one change because they are one design defect with several symptoms: the
outbound boundary is conceptually real and structurally unowned. Fixing the eight sites
individually leaves the copies, and the ninth site is already being written — the evidence
for that is the scan's own root cause for `g2-jwk-selection-apple`, which names duplication
rather than omission. Putting inbound provider responses and outbound webhook delivery under
one spec is the point rather than a packaging convenience: they are the same class of
control at the same architectural layer, and separating them is what let the webhook keep an
integrity property (the redirect ban) that the provider fetchers have, while missing one
(replay binding) that neither has.

This change adopts Option 2 of
[`hardening/proposals/provider-response-boundary.md`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/hardening/proposals/provider-response-boundary.md).
It deliberately stops short of Option 3 — collapsing `AppleProvider` into one profile-driven
validator — because C12 must be answered with evidence first, and the cross-provider corpus
this change introduces is what answers it.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | `Shared OIDC utilities` gains `ProviderTransport`, `VerificationKeySet`, the bounded success paths and the cache's lock discipline; `Webhook adapter contract` gains the signed input, the headers and the receiver obligations; `Adapter inventory` and `Decisions` follow |
| [`.specs/service/specs/05-provider-system.md`](../service/specs/05-provider-system.md) | `OidcProvider behaviour`, `Tiers` (both), `Assumptions` and `Decisions`: one purpose-filtering selector, algorithm as data, the pinned endpoint-origin set |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | `[providers.<name>]` gains `endpoint_origins`; `Decisions` records why |
| [`.specs/development-guidelines.md`](../development-guidelines.md) | `Rust conventions → Formatting and linting` gains the committed `clippy.toml`; the open question about a committed ruleset is answered in part |
| [`canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | `OidcProviderConfig` gains `endpoint_origins`; new `WebhookDelivery` `$def` |

No new canonical page. Three sibling change specs own adjacent surface and are referenced
rather than restated:

- [`2026-08-05-fail_closed_across_config_and_adapters.md`](2026-08-05-fail_closed_across_config_and_adapters.md)
  owns the discovery **HTTP status** check and the `HttpsUrl` **scheme** constraint on
  configured and discovered endpoints. `ProviderTransport` consumes `HttpsUrl`; this change
  adds the **origin** half that the scheme check leaves open, and does not restate either.
- [`2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md`](2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md)
  owns `http::read_bounded`, `MAX_UPSTREAM_BODY_BYTES`, and `upstream::error_detail`, and
  routes the three **error-body** sites through them. This change routes the two remaining
  **success-body** sites through the same ceiling and does not redefine the helper.
- [`2026-08-05-bind_id_token_grant_replay_protection.md`](2026-08-05-bind_id_token_grant_replay_protection.md)
  moves `nonce`/`azp`/`at_hash`/one-time-use above the provider boundary and states that "the
  binding does not move" whichever way the validators consolidate. This change is that
  consolidation. Its `IdentityClaims.signing_alg` is the algorithm-as-data a
  `VerificationKeySet` carries, surfaced early; once this change lands, both validators read
  it from the key set rather than from their own `alg` match.

---

## Proposed changes

### `.specs/service/specs/02-ports-and-adapters.md` → Shared OIDC utilities (Modify)

Replace the section body:

> Reused by the OIDC and Apple providers. Every outbound request to a provider endpoint is
> issued by `transport::ProviderTransport` and by nothing else — no adapter calls `reqwest`
> against a provider directly. The transport takes an `HttpsUrl`, issues the request through
> the single shared process-wide `reqwest::Client` (5s connect timeout, 10s total request
> timeout, redirects disabled), reads the response status **before** any body is read, and
> reads the body through a ceiling that fails at the limit rather than after it. It returns
> an `UpstreamBody`, which exposes `parsed::<T>()` for a known success shape and hands a
> non-success response to `upstream::error_detail`. A body that exceeds the ceiling is a
> distinct `ProviderError` naming the limit and the endpoint, so it is alertable as a
> provider fault rather than indistinguishable from a parse failure.
>
> - `transport::ProviderTransport` — `get_json::<T>(url)` and `post_form::<T>(url, params)`.
>   The four provider fetch shapes — discovery, JWKS, token endpoint, revocation — are its
>   only callers, across five call sites (`discovery::discover`, `JwksCache::fetch_keys`,
>   `token_endpoint::exchange_code`, and each provider's `revoke_token`).
> - `keys::VerificationKeySet` — the only way a JWK becomes something a signature is verified
>   with. Built from a fetched JWKS and an admitted-algorithm set; held behind an `Arc` and
>   handed out by cheap clone, never deep-cloned per request. Its constructor is where key
>   eligibility lives: an entry is dropped when `use` is present and is not `"sig"`, when
>   `key_ops` is present and does not contain `"verify"`, when its declared `alg` is outside
>   the caller's admitted set, or when its `alg` is inconsistent with its `kty`/`crv`. Lookup
>   by `kid` returns a `VerificationKey` carrying its algorithm as data, so no caller
>   re-derives one. When several entries share a `kid`, the eligible entry is returned
>   regardless of array order.
> - `jwks::JwksCache` — fetches and caches a remote JWKS as a `VerificationKeySet` behind a
>   TTL (default 1h); `with_ttl` overrides. A non-2xx JWKS response is a `ProviderError` and
>   is never cached. No lock that protects the cached value is held across the fetch: refill
>   elects one fetcher through a single-flight permit, releases the cache guard, fetches, and
>   re-acquires the guard to store. Callers that arrive during an in-flight refill are served
>   the stale-but-parsable set if one exists and otherwise await the permit; a `kid` absent
>   from a stale set still falls through to the rate-limited forced refetch, so staleness
>   fails closed. `refresh()` records its rate-limit timestamp, releases that guard, and only
>   then fetches. When a token's `kid` is not in the cached set, the cache refetches once
>   (rate-limited by a 30s minimum refresh interval) before the provider rejects the token.
> - `discovery::discover(issuer)` — fetches and parses `.well-known/openid-configuration`
>   into `DiscoveryDocument { issuer, token_endpoint, jwks_uri, revocation_endpoint }`. The
>   body is read through the transport's ceiling, so an oversized document is rejected before
>   it is materialised. Each endpoint the document supplies must have an origin in the
>   provider's pinned endpoint-origin set; a document naming an origin outside it is rejected
>   with a `ProviderError` naming the endpoint, the origin, and the permitted set.
> - `token_endpoint::exchange_code(endpoint, client_id, client_secret, code, redirect_uri)` —
>   the standard form-encoded `grant_type=authorization_code` POST.
> - `http::read_bounded_bytes(response)` — the accumulator both body readers share: it reads
>   at most `MAX_UPSTREAM_BODY_BYTES` and fails at the ceiling rather than after it, so the
>   ceiling is one constant applied to success and failure bodies alike.

### `.specs/service/specs/02-ports-and-adapters.md` → Webhook adapter contract (Modify)

Replace the section body:

> `POST` `application/json`, body `{ "event": "user.created"|"user.updated"|"user.deleted",
> "timestamp": <RFC3339>, "data": <User> }`. Three headers authenticate and identify the
> delivery: `X-Webhook-Timestamp` carrying the RFC3339 instant the delivery was minted,
> `X-Webhook-Delivery-Id` carrying a ULID unique per delivery, and `X-Signature-256` carrying
> `sha256=` followed by the hex HMAC-SHA256, under the configured secret, of the canonical
> string `<timestamp> "." <delivery-id> "." <raw body>`. The separators make the input
> unambiguous, and the algorithm prefix makes a future algorithm change expressible. The
> signature and the delivery id are minted **once**, outside the retry loop, so every attempt
> in a retry burst carries the same id and the same signature. The in-body `timestamp`
> remains, so a receiver written against the previous contract keeps parsing.
>
> A conforming receiver **must** reject a delivery whose `X-Webhook-Timestamp` is outside a
> ±5-minute tolerance of its own clock; **must** deduplicate on `X-Webhook-Delivery-Id`,
> retaining seen ids for at least the tolerance window; and **must** treat a repeated delivery
> id as a retry of one delivery rather than as an anomaly. It should verify the signature
> before parsing the body, which makes the header timestamp an authenticated value. Any 2xx is
> success; 5xx or timeout retries up to `retries` with exponential backoff; 4xx is not
> retried. The client follows no redirects: re-signing semantics across hosts are undefined,
> and forwarding a signed body to an unconfigured host is a credential-adjacent leak.

### `.specs/service/specs/02-ports-and-adapters.md` → Adapter inventory (Modify)

The `UserSync | Webhook` row's Notes cell becomes:

> HMAC-SHA256 over timestamp + delivery id + body; one signature per delivery, reused across the retry burst

### `.specs/service/specs/02-ports-and-adapters.md` → Decisions (Add)

> - *One transport for provider traffic, one client for webhook delivery.* **Every provider
>   request goes through `ProviderTransport`; webhook delivery keeps its own `reqwest::Client`.**
>   The provider client's timeouts are compile-time constants because a provider is
>   infrastructure this service does not control; the webhook client's timeout is
>   operator-configured because the receiver is the operator's own. Two clients, two owners —
>   but the same rule about who may issue a request applies to both, and neither is called
>   from an adapter directly.
> - *Key eligibility is a constructor, not a call-site check.* **`VerificationKeySet` is the
>   only way a JWK becomes a verification key, and its constructor applies the RFC 7517
>   §4.2–4.3 purpose filter.** Two independent `find_jwk` implementations both selected on
>   `kid` alone and had already drifted on algorithm handling; fixing both copies leaves the
>   copies. Concentrating the filter makes it worth testing exhaustively, which is not true of
>   two copies — at the cost of it becoming a single trusted component, which the cross-provider
>   corpus exists to hold.
> - *A signed delivery is bound to one occasion.* **The webhook HMAC covers the timestamp and
>   the delivery id as well as the body.** Origin authenticity ("this came from the holder of
>   the secret") was already established; this adds "and this is the first time you have been
>   asked to act on it". The sender emits up to eleven byte-identical POSTs on retry, so
>   without a delivery id a receiver cannot distinguish a retry from an injected replay — the
>   one gap no receiver-side diligence closes.

### `.specs/service/specs/05-provider-system.md` → Tiers, Tier 1 (Modify)

The example config block and the paragraph after it become:

> ```toml
> [providers.google]
> adapter = "oidc"
> issuer = "https://accounts.google.com"
> client_id = "${GOOGLE_CLIENT_ID}"
> client_secret = "${GOOGLE_CLIENT_SECRET}"
> scopes = ["openid", "email", "profile"]
> endpoint_origins = ["https://oauth2.googleapis.com", "https://www.googleapis.com"]
> ```
>
> `from_config` discovers the `token_endpoint`, `jwks_uri`, and `revocation_endpoint` from the
> issuer's `.well-known/openid-configuration` when they are not given. Every endpoint a
> discovery document supplies must have an origin in the provider's **pinned endpoint-origin
> set**: the issuer's own origin, plus the origin of any endpoint the operator configured
> explicitly, plus every origin listed in `endpoint_origins`. The set is fixed at config load,
> so a discovery document may confirm which origins this service talks to but can never widen
> them — a compromised or hostile document cannot relocate the verification-key source or the
> destination the client secret is posted to. Cross-origin endpoints are ordinary, not
> exceptional: Google publishes its `token_endpoint` and `revocation_endpoint` on
> `oauth2.googleapis.com` and its `jwks_uri` on `www.googleapis.com`, none of which is the
> issuer's origin, which is why the set is declared rather than derived. Adding a Tier 1
> provider is a config block — no code.

### `.specs/service/specs/05-provider-system.md` → Tiers, Tier 2 (Modify)

Append to the Tier 2 paragraph:

> It reuses the shared `JwksCache` and the shared `VerificationKeySet` for the standard
> ID-token validation parts, constructing the key set with the admitted-algorithm set
> `{RS256, ES256}` — the two algorithms Apple's own validator has always accepted. Its
> optional `token_endpoint`, `jwks_uri`, and `revocation_endpoint` overrides are pinned the
> same way as a Tier 1 provider's: the defaults are all on `appleid.apple.com`, so an override
> onto another origin must be declared in `endpoint_origins`. The issuer stays pinned to the
> `https://appleid.apple.com` constant regardless.

### `.specs/service/specs/05-provider-system.md` → OidcProvider behaviour (Modify)

Replace the second and third bullets:

> - `validate_id_token` decodes the JWT header for its `kid`, obtains the provider's
>   `VerificationKeySet` through the cached `JwksCache`, and looks the `kid` up in it. The
>   resolved `VerificationKey` carries the algorithm it will verify with, so the validation is
>   configured from the **key set** rather than from a per-provider `alg` match and never from
>   the untrusted header. A `kid` that matches only an ineligible entry is a miss: it takes the
>   forced-refetch branch and then fails closed, which is the shape invariant I3 requires.
>   Validation requires the `exp`, `iss`, and `aud` claims to be **present**
>   (`set_required_spec_claims`) and to match the configured issuer and `client_id`; `nbf` is
>   validated when present. A token missing `iss` or `aud` — e.g. a provider access token
>   presented as an ID token — is rejected.
> - Eligibility and algorithm are decided together, in the key set's constructor. An entry
>   whose `use` is present and is not `"sig"`, or whose `key_ops` is present and omits
>   `"verify"`, is not a candidate. An entry declaring an `alg` outside the provider's
>   admitted set is dropped rather than falling through to inference, so a key published as
>   `alg: "RSA-OAEP"` is rejected instead of being resolved to `RS256` from its key type.
>   Inference applies only when `alg` is genuinely absent (Azure-AD-style JWKS omit it) and is
>   restricted to the key shapes it can decide: `kty: RSA` → RS256, `kty: EC` by `crv`
>   (P-256 → ES256, P-384 → ES384), `kty: OKP` with `crv: Ed25519` → EdDSA. Any other alg-less
>   key is rejected, so an OKP key on a curve that is not a signature curve has no arm to land
>   in. A resolved algorithm must agree with the entry's `kty`/`crv` before a decoding key is
>   built.

### `.specs/service/specs/05-provider-system.md` → Assumptions (Modify)

> - Provider issuers expose a standard `.well-known/openid-configuration`; where they do not,
>   the endpoint fields must be set explicitly in config.
> - A provider's endpoint origins are known when it is configured. Providers do publish
>   endpoints off the issuer's origin — Google is the shipped example — so the origins are
>   declared in config rather than inferred from the issuer, and a provider that relocates an
>   endpoint to a new origin needs a config change before discovery will accept it.
> - The JWKS cache TTL (default 1h) is short enough to pick up upstream key rotation without
>   manual intervention, and a key set served past its TTL while a refill is in flight is
>   stale rather than untrusted.

### `.specs/service/specs/05-provider-system.md` → Decisions (Modify)

Replace *Algorithm from the JWK* and add three:

> - *Algorithm from the JWK, carried as data.* **ID-token validation uses the algorithm the
>   resolved `VerificationKey` carries, decided in the key set's constructor, not the token
>   header and not a per-provider match at the call site.** Closes the `alg`-confusion class,
>   and removes the possibility of two providers deciding the same question differently.
> - *Key purpose is binding when declared, permissive when absent.* **`use` must be `"sig"`
>   and `key_ops` must contain `"verify"` when either member is present; a JWK carrying
>   neither is eligible.** RFC 7517 §4.2–4.3 make both optional and many identity providers
>   omit them, so rejecting alg-less, use-less keys would break working deployments; treating
>   a declared purpose as decoration is what let an encryption key verify an identity
>   assertion.
> - *One selector, per-provider admitted algorithms.* **Both providers use the same
>   `VerificationKeySet`; the set of algorithms each admits is a parameter.** The generic
>   adapter admits the nine JWS algorithms it always has and Apple admits `{RS256, ES256}`, so
>   consolidating the selector neither widens Apple nor narrows the generic path. Whether the
>   two validators are otherwise equivalent is threat-model contradiction C12, and it is
>   answered by the cross-provider corpus rather than assumed by the merge.
> - *Discovery may confirm origins, never widen them.* **A provider's endpoint-origin set is
>   fixed at config load; a discovery-supplied endpoint outside it is rejected.** The RFC 8414
>   issuer self-consistency check is a string comparison and constrains nothing about the
>   endpoints the document goes on to name — threat-model contradiction C4. Pinning the set in
>   config keeps the operator's declared intent authoritative over the provider's runtime
>   assertion.

### `.specs/service/specs/06-configuration.md` → `[providers.<name>]` (Modify)

> ### `[providers.<name>]`
> `adapter` (`oidc` | `apple`) plus adapter-specific fields captured via a flattened
> `extra: HashMap<String, toml::Value>`. `endpoint_origins` is an optional array of `scheme
> "://" host [":" port]` origins that a discovery document is permitted to name in addition to
> the issuer's own origin and the origins of any explicitly configured endpoint; each entry
> must parse as an `https` origin with no path, query, or fragment. It defaults to empty,
> which pins a provider to its issuer's origin. See
> [05-provider-system.md](05-provider-system.md).

### `.specs/service/specs/06-configuration.md` → Decisions (Add)

> - *Endpoint origins are declared, not derived.* **`endpoint_origins` lists the extra origins
>   a provider's discovery document may name.** Deriving the permitted set from the issuer
>   alone would reject Google, whose `token_endpoint`, `jwks_uri`, and `revocation_endpoint`
>   are on two origins that are neither the issuer nor each other; deriving it from the
>   discovery document is what the constraint exists to prevent. Declaring it makes the
>   trusted set reviewable in the same file that names the provider.

### `.specs/development-guidelines.md` → Rust conventions → Formatting and linting (Modify)

> - `cargo fmt --all` clean before pushing (`cargo fmt --check --all` in CI).
> - `cargo clippy --workspace -- -D warnings` clean — zero warnings.
> - A committed `clippy.toml` configures `await-holding-invalid-types` with
>   `tokio::sync::RwLockWriteGuard`, `tokio::sync::RwLockReadGuard`, and
>   `tokio::sync::MutexGuard`, so `clippy::await_holding_invalid_type` fires at the binding
>   site when an async-aware lock guard is alive across an `.await`. The better-known
>   `clippy::await_holding_lock` covers only `std::sync` and `parking_lot` guards and does not
>   catch tokio's, which is why the type list is configured deliberately. The stated rule
>   behind the lint: **no lock guard may be alive across an `.await` that performs I/O**, and
>   single-flight is expressed with its own primitive rather than obtained as a side effect of
>   a data lock.

### `.specs/development-guidelines.md` → Open questions (Modify)

> - A `clippy.toml` is committed, configuring `await-holding-invalid-types` only. Whether to
>   extend it toward a pedantic-adjacent ruleset is still open; the file existing removes the
>   obstacle but not the question.

---

## Type changes

```json
{
  "$comment": "Fragment for 2026-08-05-own_outbound_http_boundary. Folds into .specs/service/specs/canonical-types.schema.json on merge.",
  "$defs": {
    "OidcProviderConfig": {
      "type": "object",
      "required": ["provider_id", "issuer", "client_id", "scopes"],
      "properties": {
        "provider_id": { "type": "string" },
        "issuer": { "type": "string", "description": "Used for OIDC discovery. Its origin is always in the permitted endpoint-origin set." },
        "client_id": { "type": "string" },
        "client_secret": { "type": ["string", "null"] },
        "jwks_uri": { "type": ["string", "null"] },
        "token_endpoint": { "type": ["string", "null"] },
        "revocation_endpoint": { "type": ["string", "null"] },
        "endpoint_origins": {
          "type": "array",
          "default": [],
          "items": { "type": "string", "pattern": "^https://[^/?#]+$" },
          "description": "Extra origins a discovery document may name for token_endpoint, jwks_uri, or revocation_endpoint, beyond the issuer's own origin and those of explicitly configured endpoints. Scheme, host, optional port; no path, query, or fragment."
        },
        "scopes": { "type": "array", "items": { "type": "string" } },
        "additional_params": { "type": "object", "additionalProperties": { "type": "string" } }
      }
    },
    "WebhookDelivery": {
      "type": "object",
      "description": "One user-sync webhook delivery. The body is the signed payload; the headers carry the material bound into the signature.",
      "required": ["headers", "body"],
      "additionalProperties": false,
      "properties": {
        "headers": {
          "type": "object",
          "required": ["X-Webhook-Timestamp", "X-Webhook-Delivery-Id", "X-Signature-256"],
          "additionalProperties": true,
          "properties": {
            "X-Webhook-Timestamp": { "$ref": "../../canonical-types.schema.json#/$defs/Timestamp" },
            "X-Webhook-Delivery-Id": { "$ref": "../../canonical-types.schema.json#/$defs/Ulid" },
            "X-Signature-256": {
              "type": "string",
              "pattern": "^sha256=[0-9a-f]{64}$",
              "description": "Hex HMAC-SHA256 over `<X-Webhook-Timestamp> \".\" <X-Webhook-Delivery-Id> \".\" <raw body>` under user_sync.webhook.secret."
            }
          }
        },
        "body": {
          "type": "object",
          "required": ["event", "timestamp", "data"],
          "additionalProperties": false,
          "properties": {
            "event": { "type": "string", "enum": ["user.created", "user.updated", "user.deleted"] },
            "timestamp": { "$ref": "../../canonical-types.schema.json#/$defs/Timestamp" },
            "data": { "type": "object", "additionalProperties": true }
          }
        }
      }
    }
  }
}
```

`ProviderTransport`, `UpstreamBody`, `VerificationKeySet`, and `VerificationKey` are internal
adapter types with no wire or storage form, so they do not enter the canonical entity schema —
the same treatment
[`2026-08-05-fail_closed_across_config_and_adapters.md`](2026-08-05-fail_closed_across_config_and_adapters.md)
gives its config newtypes. Their shape is documented by the `Shared OIDC utilities` prose above.

---

## Implementation notes

Sequencing against the siblings: `HttpsUrl` from
[`fail_closed`](2026-08-05-fail_closed_across_config_and_adapters.md) and `read_bounded` /
`upstream::error_detail` from
[`eliminate_secret_leakage`](2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md) are
prerequisites for steps 3 and 4. Steps 1, 2, and 8 depend on nothing and should land first.

1. **`Arc` the cached key set.** `crates/adapters/src/shared/jwks.rs:28` — `CachedJwks.keys`
   becomes `Arc<serde_json::Value>` (and later `Arc<VerificationKeySet>`), and the three
   `cached.keys.clone()` sites at `:60`, `:71`, and `:109` become `Arc::clone`. This removes a
   measured 73–95 ms of CPU per `POST /token` against a 64 MiB key set and scales with any
   legitimately large one. Benchmark before and after; the target is sub-millisecond.
2. **Commit `clippy.toml`** at the workspace root with the three tokio guard types under
   `await-holding-invalid-types`. It fires immediately on `jwks.rs:66` (`get_keys`) and on
   `jwks.rs:138` (`refresh`'s `last_forced_refetch` guard, which spans `fetch_keys` at `:153`
   and is a second instance of the same pattern), so land it with or immediately before step 7.
3. **Build `ProviderTransport` as a pass-through** in a new
   `crates/adapters/src/shared/transport.rs`, exported from `shared/mod.rs`. No new checks
   yet. Migrate the five call sites one commit at a time: `discovery.rs:23`, `jwks.rs:170`,
   `token_endpoint.rs:28`, `oidc/mod.rs:241`, `providers/src/apple.rs:331`. The proposal
   counts four fetch *shapes*; there are five call sites because both providers implement
   `revoke_token` independently.
4. **Enable the checks, one commit each**, so any behaviour change is attributable: status
   before body (already present on the JWKS path; the discovery half is owned by the
   `fail_closed` sibling and must not be removed until the transport is the sole caller),
   then `HttpsUrl`, then the byte ceiling. Add `http::read_bounded_bytes` alongside the
   sibling's `read_bounded`, sharing `MAX_UPSTREAM_BODY_BYTES`, and repoint the two remaining
   unbounded success sinks — `jwks.rs:187` (`response.json()`) and `discovery.rs:31-33`
   (`response.json::<DiscoveryDocument>()`) — at it, deserialising from the bounded slice.
   `token_endpoint.rs:39` reads one `raw_body` used by both paths and is already bounded by
   the sibling; do not add a second read there.
5. **Answer C12 before consolidating.** Write the cross-provider corpus (below) against the
   two validators *as they are*, record which entries each accepts, and commit the result.
   The `VerificationKeySet` filter must be a deliberate superset of both behaviours, not an
   accidental intersection — this step is what makes that a decision rather than an accident.
6. **Build `VerificationKeySet`** in a new `crates/adapters/src/shared/keys.rs`, then delete
   both `find_jwk` copies (`oidc/mod.rs:52-67`, `apple.rs:67-78`) and both `alg` matches
   (`oidc/mod.rs:176-192`, `apple.rs:275-286`), folding `infer_alg_from_jwk`
   (`oidc/mod.rs:32-45`) into the constructor with the `(Some("OKP"), _)` wildcard narrowed to
   `(Some("OKP"), Some("Ed25519"))`. The `assert_eq!` on `kid` at `oidc/mod.rs:159-163` and
   `apple.rs:259-263` asserts the tautology it selected on; replace it with the constructor's
   typed rejection, which disposes of threat-model contradiction C11 rather than answering it.
7. **Redesign the single-flight as its own commit with its own review.** A
   `tokio::sync::Semaphore` permit elects one refetcher; `try_acquire` before `acquire` keeps
   the cold-cache case correct; non-elected callers with a stale set are served it. Apply the
   same ordering to `refresh()`: write the timestamp, drop the `last_forced_refetch` guard,
   then fetch — the "record the attempt before the network call" semantics at `jwks.rs:147-151`
   are preserved exactly, because the timestamp is already written before the guard is
   released. Getting this wrong trades a lock-hold problem for a thundering herd, which is
   why it is not folded into step 6. **Step 4 must land before this one.** Today's
   guard-across-fetch accidentally bounds concurrent allocation to one body per provider,
   because racing callers queue on the lock instead of each launching a fetch; removing the
   serialisation removes that bound, so the byte ceiling has to be in place first or the
   concurrency fix briefly widens the allocation finding it is unrelated to.
8. **Pin the endpoint origins.** Add `endpoint_origins` to `OidcProviderConfig`
   (`crates/core/src/domain/provider.rs`) and lift it in `provider_config_to_oidc`
   (`crates/server/src/bootstrap.rs:787-798`). Compute the permitted set in
   `OidcProvider::from_config` (`crates/adapters/src/oidc/mod.rs:74-111`) and in
   `AppleProvider::from_config` (`crates/providers/src/apple.rs:133-151`), and check each
   discovery-supplied endpoint against it in `discovery::discover`. **Ship in warning mode
   first**: log a structured warning naming the endpoint and its origin, for one release,
   before rejecting — a deployment relying on an undeclared cross-origin endpoint should learn
   about it from a log line rather than an outage. Then update every shipped Google stanza:
   `examples/container/config/production.toml:30`,
   `examples/linux-postgres/config/postgres-only.toml:26` and `postgres-valkey.toml:33`,
   `examples/aws-web/config/oidc-exchange.toml:40`, the config test fixture at
   `crates/core/src/config.rs:538`, and the provider blocks in `README.md`,
   `README.docker.md`, and `docs/`. `config/default.toml` ships no `[providers.*]` section, so
   it needs no change.
9. **Bind the webhook delivery.** In `crates/adapters/src/webhook/mod.rs`, change
   `compute_hmac_hex` (`:124-130`) to take `timestamp` and `delivery_id` and update the MAC
   over `timestamp`, `b"."`, `delivery_id`, `b"."`, `body`. In `send_webhook` (`:59-121`) mint
   `sent_at` and a `ulid::Ulid` delivery id alongside the signature at `:70` — outside the
   retry loop at `:73` — and add `X-Webhook-Timestamp`, `X-Webhook-Delivery-Id`, and the
   `sha256=`-prefixed `X-Signature-256` to the header set at `:82-83`. `ulid` is already a
   dependency of this crate; a ULID is lexicographically sortable, so a receiver gets ordering
   for free. Mirror the contract change into `docs/architecture/adapters.md:222`, which
   currently repeats the body-only rule, and add a worked receiver example there — the failure
   mode this fixes is a receiver author doing exactly what the document says.

**Tests.**

- *Key-selection corpus* (`crates/adapters/src/shared/keys.rs`, run against both providers):
  `use: "enc"`; `key_ops: ["encrypt","wrapKey"]`; `key_ops` omitting `"verify"`; `alg`
  inconsistent with `kty`; `alg: "RSA-OAEP"` on an RSA key; `alg: "RS256"` on a `use: "enc"`
  RSA key; `alg: "ES256"` on a `use: "enc"` P-256 key; absent `alg`; duplicate `kid` across an
  `enc` and a `sig` entry in both array orders; `oct` key; `alg: "none"`. Each asserted
  rejected or resolved identically on both provider paths. This test **is** the answer to C12.
  A `use: "sig"` entry must still verify — the non-regression case.
- *Byte ceiling*: a `wiremock` endpoint serving a body over the ceiling, with an honest
  `Content-Length` and again with `Transfer-Encoding: chunked`; assert the distinctive
  cap error and that the cache is left unpopulated, in the style of the existing
  `non_2xx_response_is_error_and_leaves_cache_unpopulated`.
- *Concurrency*: three or more callers racing an expired TTL against a delayed origin; assert
  exactly one outbound fetch (`wiremock` `.expect(1)`) and that no caller waits longer than one
  fetch. The single-fetch assertion is what stops a naive "just drop the guard" fix landing.
- *Origin pinning*: a discovery document naming a `jwks_uri` on an undeclared origin is
  rejected; the same origin listed in `endpoint_origins` is accepted; the Google shape —
  three endpoints across two non-issuer origins, all declared — passes.
- *Webhook*: recompute the MAC over `timestamp.delivery_id.body` and assert it matches the
  header while the body-only MAC does **not**, so a regression to the old scheme fails loudly;
  a mutated `X-Webhook-Timestamp` invalidates the signature; two deliveries carry different
  ids; a retry burst (extending the existing `test_retry_on_5xx`) carries one id and one
  signature across all attempts.

Evidence: sealed scan bundle `.security/oidc-exchange/53cbdec9_20260804T102454Z/`, findings
`g2-jwk-selection-oidc`, `g2-jwk-selection-apple`, `g2-jwks-cache-lock-across-await`,
`g2-jwks-response-size-unbounded`, `g2-provider-endpoint-scheme-oidc`,
`g2-provider-endpoint-scheme-apple`, `g1-webhook-delivery-replayable`; structural context
`hardening/proposals/provider-response-boundary.md` (Option 2, invariants PB1–PB6); threat
model `artifacts/01_context/threat_model.md` (boundaries B2, B3, B9; invariants I3, I18;
contradictions C4, C11, C12).

---

## Compatibility

**Webhook receivers break.** Both the signed input and the `X-Signature-256` value format
change, so every existing receiver rejects every delivery until it is updated. There is no
handshake through which a sender could negotiate, and the failure is quiet: a 4xx is not
retried and a sync failure is logged and swallowed at
`crates/core/src/service/exchange.rs:225-227`, so an operator who does not read release notes
loses user sync without an error surfacing anywhere else. Three things bound the cost.
`user_sync.enabled` defaults to `false`; no shipped example, template, or test configuration
turns it on; and the service is pre-1.0. The break is therefore taken deliberately rather
than softened with a version knob — see Decisions — and it must be called out in the release
notes as a required receiver change, with the worked receiver example shipped alongside.

**`crates/adapters` stays source-compatible for embedders.** `IdentityProvider`'s signature is
unchanged; what changes is what the two implementations call underneath. `find_jwk` and
`infer_alg_from_jwk` are private, so their removal is invisible outside the crate.
`JwksCache::get_keys`'s return type changes from `serde_json::Value` to a shared
`VerificationKeySet` — `JwksCache` is `pub`, so this is a breaking change for an embedder who
uses it directly, and it is the only one.

**Undeclared cross-origin endpoints stop working.** A deployment whose provider publishes
endpoints off the issuer's origin — the common case, not the exotic one — must add
`endpoint_origins` before enforcement lands. This is why step 8 ships in warning mode for one
release; the warning names the endpoint and the origin, so the value to add is in the log
line.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**` to
   the merge date.
2. Fold the `Type changes` `$defs` into
   [`.specs/service/specs/canonical-types.schema.json`](../service/specs/canonical-types.schema.json):
   replace `OidcProviderConfig` wholesale and add `WebhookDelivery`.
3. No new canonical page is created, so `.specs/README.md` needs no new spec row — but move
   this file from the pending change-spec table to the merged area, and confirm the pending
   table lists the sibling specs this one references (it currently omits
   `2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md` and
   `2026-08-05-harden_release_supply_chain.md`).
4. Confirm the three sibling change specs have merged first, or that their prerequisite pieces
   (`HttpsUrl`, `read_bounded`, `upstream::error_detail`) shipped; the `Shared OIDC utilities`
   block above assumes their prose is already in place and edits around it rather than over it.
5. Flip this file's `**Status:**` to `Merged`, add a `**Merged:** YYYY-MM-DD` field to its
   header, and move it to `.specs/changes/merged/`.
6. Update `.specs/README.md`: remove the file from the pending list, leave the merged area
   pointing at `changes/merged/`.

---

## Assumptions and open questions

### Assumptions

- `HttpsUrl`, `read_bounded`, `MAX_UPSTREAM_BODY_BYTES`, and `upstream::error_detail` land
  from the sibling specs and are not redefined here. If either sibling is rejected, this
  change grows a dependency it did not budget for.
- The webhook receiver is the operator's own system, so a coordinated contract change is
  possible in a way it would not be for a public API.
- 64 KiB — the sibling's `MAX_UPSTREAM_BODY_BYTES` — is generous for a JWKS and a discovery
  document. Real key sets are a few kilobytes; the largest realistic multi-key set is an order
  of magnitude under the ceiling.
- `ulid` is already a workspace dependency of `crates/adapters`, so the delivery id costs no
  new dependency.
- Google's discovery document places `token_endpoint` and `revocation_endpoint` on
  `oauth2.googleapis.com` and `jwks_uri` on `www.googleapis.com`. These values are read from
  the live document and are the basis for the shipped `endpoint_origins`; they are provider
  facts and can change without notice, which is the reason they live in config.

### Decisions

- *Option 2, not Option 1 or 3.* **Own the controls in shared types; do not merge the two
  validators.** Option 1 fixes the same eight sites but leaves the copies that produced them,
  and no amount of careful local patching answers C12 — only shared ownership does. Option 3
  is the design this argues toward, but collapsing two validators is safe only once their
  differences are enumerated, and the corpus in step 5 is what enumerates them. Running both
  providers through the shared boundary for a release is what makes Option 3 a mechanical
  follow-on rather than a leap.
- *Inbound and outbound in one change.* **Provider responses and webhook delivery are specified
  together.** Both are `crates/adapters` HTTP boundaries whose integrity properties were
  implemented per call site, which is how the duplication and the omissions arose. Splitting
  them would repeat the mistake at the level of the spec: the webhook already has a control
  the provider fetchers lack (`Policy::none()`, with a regression test) and lacks one neither
  has, and only looking at them together makes that visible.
- *Origin set declared in config, not derived from the issuer.* **A same-origin rule was
  evaluated and rejected.** It would reject Google — whose three endpoints sit on two origins,
  neither of them the issuer's — which is the repository's own flagship example. A registrable-
  domain rule fails the same test, since `google.com` and `googleapis.com` differ. Declaring
  the set keeps the security property that matters (a discovery document cannot introduce an
  origin at runtime) without a rule that is wrong about the ordinary case.
- *Per-provider admitted algorithms, not one union.* **The shared key set takes the admitted
  algorithm set as a parameter.** A single union would silently widen Apple from two
  algorithms to nine, which is adopting the weaker behaviour for both providers — the exact
  outcome the C12 sequencing exists to avoid.
- *Inference retained, narrowed.* **`infer_alg_from_jwk` survives, but only for a genuinely
  absent `alg`, and its OKP arm requires `crv: Ed25519`.** Removing it entirely would make the
  purpose invariant cleaner and would reject JWKS entries from providers that omit `alg`,
  which RFC 7517 permits and Azure AD does. The defect was never inference itself; it was that
  an *unrecognised* `alg` and an *absent* one took the same path. Splitting those two cases
  removes the reproduced vector at no compatibility cost. The OKP narrowing closes a latent
  edge that is currently held shut only by `jsonwebtoken`'s `EllipticCurve` variant list.
- *A hard webhook break, no `signature_version` knob.* **Receivers update; the sender does not
  offer the old scheme.** A version knob is a supported way to stay replayable, and every
  exception is a place the constraint does not apply. The feature is off by default, no shipped
  configuration enables it, and the service is pre-1.0 — the cheapest moment this break will
  ever cost.
- *Single-flight as its own commit.* **The `JwksCache` concurrency redesign is reviewed
  separately from the transport refactor.** It is the one piece here where a hasty change makes
  availability worse rather than better, and folding it into a larger change is how it gets
  less attention than it deserves.
- *`clippy.toml` for one lint, not a ruleset.* **The committed file configures
  `await-holding-invalid-types` and nothing else.** It is the durable control for the defect
  class this change fixes; a broader pedantic ruleset is a separate decision that should not
  ride along on a security change.

### Open questions

- Should the JWKS ceiling be separately configurable from the shared 64 KiB upstream ceiling?
  A provider that grows its key set past it breaks logins until an operator raises it, which
  is a denial-of-service shape in the other direction. Alerting on the distinctive cap error
  is specified above; whether that is sufficient without a knob needs a maintainer's view.
- Should a non-elected caller be served a stale key set at all, or should it await the permit?
  Serving stale is specified here because an expired entry is stale rather than untrusted and
  a rotated-away `kid` still fails closed through the forced-refetch path. A reviewer who
  prefers never to serve past the TTL can drop that branch; callers still wait one round trip
  rather than holding the cache hostage.
- Should `crates/adapters` remain a supported embedding surface? `revoke_token` has no in-repo
  caller and exists for embedders. If that audience is not real, the `JwksCache::get_keys`
  signature change costs nothing and Option 3's migration cost largely disappears too.
- Is a one-release `signature_version = "v1"` escape hatch on the webhook wanted after all?
  The decision above says no; an operator with a receiver they cannot redeploy on the same
  cadence may reasonably overrule it.
- The `X-Signature-256` value carries an algorithm prefix but no key id, so a receiver cannot
  tell which secret signed a delivery during a rotation and must try both. Whether to add a
  `k1=`-style key tag now or when secret rotation is specified is open.
