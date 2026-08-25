# Change: Bind the direct ID-token grant to a server-issued nonce and make it single-use

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** crates/adapters, crates/providers (service)

Give the direct `id_token` grant the replay protection it has never had: a `nonce` this
service mints, stores, and burns on use; an `azp` check; an `at_hash` check when an access
token accompanies the assertion; and a one-time-use marker keyed on the assertion's `jti`.
Enforce all four **once**, in `AppService::exchange`, rather than in each provider's
validator — and add a `[grants] id_token` switch, defaulting to `false`, so an operator who
only uses the authorization-code flow does not serve the grant at all.

---

## Motivation

Both ID-token validators check signature, issuer, audience, `exp` and `nbf`, and nothing
else. `crates/adapters/src/oidc/mod.rs:193-197` and `crates/providers/src/apple.rs:287-291`
are the same five lines twice; neither enforces `nonce`, `azp`, `at_hash`, or one-time use.
An OIDC ID token's only built-in replay defence is the `nonce` the relying party planted in
the authentication request and re-checks on the way back, and this relying party plants
none. So possession of a victim's ID token minted for the configured `client_id` is
sufficient to mint first-party access tokens carrying the victim's admin-assigned claims,
plus a 30-day refresh token — repeatedly, until the assertion's own `exp`. The
authorization-code branch of the same function is not exposed the same way: redeeming a
single-use code over an authenticated back channel is itself a freshness proof. The direct
branch drops that proof and puts nothing in its place. Scan evidence:
`.security/oidc-exchange/53cbdec9_20260804T102454Z/findings/g1-id-token-grant-replayable/`.

Two facts shape the design rather than just the fix. First, the grant cannot be turned off:
`crates/core/src/service/exchange.rs:74` selects the direct branch on the *presence* of the
`id_token` field, there is no configuration switch, and `crates/server/src/routes/well_known.rs:16`
advertises only `authorization_code` and `refresh_token` — so a live grant is absent from the
service's own metadata. Second, the identical omission occurred in two independent
implementations, which is the real finding. `hardening/proposals/provider-response-boundary.md`
records an unresolved question (C12) about whether the two validators even accept the same
tokens today, and both hardening proposals agree that the binding should land once. This
change therefore does not add checks to two validators. It adds them to the core exchange
flow, where a Tier 3 provider that shares no code with either validator inherits them too.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/00-overview.md`](../service/specs/00-overview.md) | Scope-summary row for the direct grant; rewrite the *Two grant inputs* Decision |
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md) | `IdentityClaims` gains `signing_alg`; new `SingleUseRecord` entity and query-pattern rows |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | `SessionRepository` gains the single-use methods; `IdentityProvider` gains `client_id()` |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | New assertion-binding step in the exchange flow; the `POST /nonce` flow; Decisions |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | `POST /nonce` route; `/token` direct-grant request shape; discovery `grant_types_supported` |
| [`.specs/service/specs/05-provider-system.md`](../service/specs/05-provider-system.md) | Both validators report `signing_alg`; binding is stated as a core concern, not a provider one |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | New `[grants]` section and defaults rows |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) | Single-use record storage per adapter; `SingleUseRecord` joins the logical schema |
| [`canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | `IdentityClaims.signing_alg`; new `SingleUseRecord` `$def` |

No new canonical page. Note the Target line names the two adapter crates because that is
where the defect lives; the enforcement deliberately lands in `crates/core` and the new
route in `crates/server`, and the adapters change only to report the algorithm they verified
with.

---

## Proposed changes

### `.specs/service/specs/06-configuration.md` → Sections (Add)

> ### `[grants]`
> Which grants `/token` serves and the parameters of the direct ID-token grant's replay
> protection. `id_token` (bool, default `false`) — whether the direct ID-token grant is
> served at all. `nonce_ttl` (humantime duration, default `"10m"`) — how long a nonce minted
> at `POST /nonce` remains claimable. `max_assertion_lifetime` (humantime duration, default
> `"1h"`) — the ceiling on an accepted provider ID token's remaining lifetime; an assertion
> with longer to live is refused. The authorization-code and refresh-token grants are always
> served and have no switch. Both durations are validated at startup by
> `AppConfig::validate`, so an unparseable value fails config load.

### `.specs/service/specs/06-configuration.md` → Defaults summary (Add)

> | `grants.id_token` | `false` |
> | `grants.nonce_ttl` / `max_assertion_lifetime` | `10m` / `1h` |

### `.specs/service/specs/04-http-api.md` → Routes, Public (Modify)

> | POST | `/nonce` | `nonce` | mint a single-use nonce for the direct ID-token grant; mounted only when `grants.id_token = true` |

> `POST /nonce` takes no body and returns `{"nonce": "<base64url>", "expires_in": <seconds>}`.
> The nonce is 32 random bytes, base64url-no-pad; only its SHA-256 hex digest is stored. The
> route is unauthenticated by necessity — the caller holds no credential yet — and is not
> mounted at all when the direct grant is disabled, so an operator who leaves the default in
> place gains no new public surface.

### `.specs/service/specs/04-http-api.md` → POST /token request (Modify)

> ```
> # code exchange:  grant_type=authorization_code & code=… & redirect_uri=… & provider=google
> # direct token:   grant_type=id_token & id_token=… & provider=google
> #                 [& provider_access_token=…]
> # refresh:        grant_type=refresh_token & refresh_token=…
> ```
>
> The direct grant requires the ID token to carry a `nonce` claim whose value came from this
> service's `POST /nonce`; the client passes that value into the provider's authentication
> request and does not resend it here — the service reads it from the verified assertion.
> `provider_access_token` is optional and carries the provider access token co-issued with
> the ID token, so the `at_hash` binding can be verified. When `grants.id_token = false` an
> `id_token` field is rejected with `unsupported_grant_type` whatever `grant_type` declares,
> so the switch cannot be evaded by the field-presence branch selection.

### `.specs/service/specs/04-http-api.md` → GET /.well-known/openid-configuration (Modify)

> `grant_types_supported` reports `authorization_code`, `refresh_token`, and — only when
> `grants.id_token = true` — `id_token`. The document describes the grants the process
> actually serves.

### `.specs/service/specs/03-service-flows.md` → Token exchange (Modify)

Insert a new step 3 between *Obtain verified claims* and *User lookup / registration policy*,
and renumber the rest:

> 3. **Bind the assertion** — every accepted ID token, on both grant paths, passes
>    `service::assertion::bind`, which runs in this order and rejects with `InvalidGrant` at
>    the first failure (each rejection audited as `ValidationFailed`/`Warning` with a
>    `detail.check` naming the failed control):
>    - **Lifetime ceiling** — `exp - now` must not exceed `grants.max_assertion_lifetime`, so
>      the single-use marker below always outlives the assertion it guards.
>    - **`azp`** — when `aud` is an array of more than one value, `azp` is required; whenever
>      `azp` is present it must equal `provider.client_id()`. A token minted for a sibling
>      client of the same provider is rejected.
>    - **`at_hash`** — when an access token accompanies the assertion (`provider_access_token`
>      on the direct grant, `ProviderTokens.access_token` on the code path) and the assertion
>      carries `at_hash`, the claim must equal the base64url of the left-most half of the
>      digest of the access token's ASCII octets (OIDC Core §3.1.3.6). The digest follows
>      `IdentityClaims.signing_alg`: SHA-256 for `*256`, SHA-384 for `*384`, SHA-512 for
>      `*512`. An `at_hash` on an `EdDSA`-signed assertion is unverifiable and is rejected.
>      An `at_hash` with no accompanying access token is not verifiable and is skipped.
>    - **Nonce (direct grant only)** — the `nonce` claim must be present, and
>      `take_single_use("nonce:<sha256hex>")` must report it present. That single atomic
>      operation is both the nonce check and the nonce's own one-time-use guarantee: an
>      absent, expired, or already-burned nonce is indistinguishable and all three reject.
>      The code-exchange path requires no nonce — redeeming a single-use code at the
>      provider supplies the binding.
>    - **Single use** — `put_single_use(assertion_key, exp)` must report the key newly
>      inserted; a key already present means the assertion has been spent and is rejected as
>      a replay. `assertion_key` is `assertion:<provider>:<sha256hex(jti)>` when the token
>      carries a `jti`, else `assertion:<provider>:d:<sha256hex(compact_jwt)>`; the `d:`
>      discriminator keeps a literal `jti` from colliding with a digest. The record's
>      `expires_at` is the assertion's own `exp`.

### `.specs/service/specs/03-service-flows.md` → Nonce issuance (Add, after Token exchange)

> ## Nonce issuance (`service/assertion.rs::mint_nonce`)
>
> `POST /nonce`, served only when `grants.id_token = true`.
>
> 1. 32 random bytes, base64url-no-pad, is the returned nonce; its SHA-256 hex is the key.
> 2. `put_single_use("nonce:<hash>", now + grants.nonce_ttl)`. A `false` return is a 256-bit
>    collision and is surfaced as `StoreError` rather than retried.
> 3. Respond `{ nonce, expires_in }`.

### `.specs/service/specs/03-service-flows.md` → Decisions (Add)

> - *Binding lives in the core, not the providers.* **`nonce`, `azp`, `at_hash` and
>   single-use are enforced once in `AppService::exchange`, reading `IdentityClaims.raw_claims`.**
>   The same four controls were omitted twice in two independent validators; a control that
>   every provider must inherit belongs above the provider boundary, not inside it. A Tier 3
>   provider sharing no code with either OIDC validator is covered by construction.
> - *Burn the nonce before claiming the assertion marker.* **The nonce is consumed first; the
>   single-use marker is claimed second.** The reverse order lets an attacker holding a
>   victim's assertion but no valid nonce pin the marker and deny the legitimate client its
>   own first use. In this order a partial failure costs the honest client one `POST /nonce`
>   round trip and never admits a replay.
> - *A lifetime ceiling instead of a capped marker TTL.* **An assertion whose remaining
>   lifetime exceeds `grants.max_assertion_lifetime` (default 1h) is refused.** Capping the
>   marker's TTL instead would leave the assertion replayable after the cap. Real ID tokens
>   live 5–60 minutes, so the ceiling rejects nothing legitimate and bounds the state a
>   single assertion can pin.

### `.specs/service/specs/02-ports-and-adapters.md` → SessionRepository (Modify)

> ```rust
> async fn put_single_use(&self, key: &str, expires_at: DateTime<Utc>) -> Result<bool>;
> async fn take_single_use(&self, key: &str) -> Result<bool>;
> ```
>
> The single-use pair backs nonces and assertion-replay markers. `put_single_use` is an
> atomic insert-if-absent returning `true` when the record was written and `false` when a
> live record already held the key; `take_single_use` is an atomic remove-and-report
> returning `true` when a live record was found and is now gone. **Both treat a record whose
> `expires_at` has passed as absent**, so correctness does not depend on the reaper having
> run: an expired nonce cannot be taken, and an expired marker's key is reusable.
> `cleanup_expired_sessions` also sweeps expired single-use records; its return count covers
> both. Nonces and markers are short-lived and high-churn, exactly like sessions, so they
> live wherever sessions live — the `[session_repository]` store when one is configured,
> otherwise the `[repository]` store — with no new configuration surface.

### `.specs/service/specs/02-ports-and-adapters.md` → IdentityProvider (Modify)

> ```rust
> async fn exchange_code(&self, code: &str, redirect_uri: &str) -> Result<ProviderTokens>;
> async fn validate_id_token(&self, id_token: &str) -> Result<IdentityClaims>;
> async fn revoke_token(&self, token: &str) -> Result<()>;
> fn provider_id(&self) -> &str;
> fn client_id(&self) -> &str;
> ```
>
> `client_id` reports the audience the provider pins, so the core's `azp` check does not have
> to reach into `[providers.<name>]` config. `validate_id_token`'s signature is unchanged —
> the binding controls read the claims it already returns.

### `.specs/service/specs/05-provider-system.md` → OidcProvider behaviour (Modify)

Append to the `validate_id_token` bullet:

> The returned `IdentityClaims` carries `signing_alg` — the algorithm the JWK actually
> verified with, not the header's — so the core's `at_hash` check can select the matching
> digest without re-deciding the algorithm. Both validators report it; neither performs any
> replay or binding check itself.

### `.specs/service/specs/05-provider-system.md` → Decisions (Add)

> - *Replay binding is above the provider boundary.* **No `IdentityProvider` implementation
>   checks `nonce`, `azp`, `at_hash` or one-time use; `AppService::exchange` does, for all of
>   them.** Two independent validators omitted the same four controls; adding them twice
>   would leave a third implementation free to omit them again. This is consistent with the
>   direction in `hardening/proposals/provider-response-boundary.md`: whether the two
>   validators later consolidate behind a shared `ProviderTransport`/`VerificationKeySet` or
>   collapse into one profile-driven validator, the binding does not move. `signing_alg` is
>   the same algorithm-as-data that a `VerificationKeySet` would carry, surfaced early.

### `.specs/service/specs/01-domain-model.md` → Token types (Modify)

> - **`IdentityClaims`** — verified claims from a provider ID token: `subject`, optional
>   `email`, `email_verified`, `name`, `is_private_email` (Apple private-relay flag; `None`
>   for other providers), `signing_alg` (the algorithm the resolved JWK verified with, e.g.
>   `"ES256"`), and `raw_claims`.

### `.specs/service/specs/01-domain-model.md` → Entities (Add)

> ### SingleUseRecord (`domain/single_use.rs`)
>
> ```rust
> struct SingleUseRecord {
>     key: String,                  // "nonce:<sha256hex>" | "assertion:<provider>:…"
>     expires_at: DateTime<Utc>,
> }
> ```
>
> A presence-only record: the key is all the information there is. Nonce values and
> assertions are stored only as SHA-256 hex digests, as refresh tokens are. Records are
> removed by `take_single_use`, by store-native expiry, or by `cleanup_expired_sessions`.

### `.specs/service/specs/01-domain-model.md` → Required query patterns (Add)

> | Claim a single-use key | `SessionRepository::put_single_use(key, expires_at)` |
> | Burn a single-use key | `SessionRepository::take_single_use(key)` |

### `.specs/service/specs/08-persistence.md` → Single-use records (Add, new section between Session-only stores and Logical schema)

> Single-use records use each adapter's natural atomic primitive, so `put_single_use` and
> `take_single_use` are one round trip everywhere:
>
> | Adapter | Layout | `put_single_use` | `take_single_use` |
> |---|---|---|---|
> | DynamoDB | `pk = SINGLEUSE#<key>`, `sk = SINGLEUSE`, numeric `ttl` | `PutItem` conditioned on `attribute_not_exists(pk) OR expires_at < :now` | `DeleteItem` with `ReturnValues=ALL_OLD`, `expires_at` checked on the returned item |
> | Postgres / SQLite | `single_use(key PRIMARY KEY, expires_at)` | `INSERT … ON CONFLICT (key) DO UPDATE … WHERE single_use.expires_at < now()`, rows affected reports the result | `DELETE … WHERE key = $1 AND expires_at > now() RETURNING 1` |
> | Valkey | `{prefix}single_use:{key}` | `SET … NX EX <ttl>` | `GETDEL` |
> | LMDB | `single_use` named DB | one write txn: read, treat an expired value as absent, write | one write txn: read, delete, report whether the value was live |
>
> DynamoDB and Valkey expire records natively; Postgres, SQLite and LMDB rely on the
> `cleanup_expired_sessions` sweep for space reclamation only — both operations already
> evaluate `expires_at`, so an unswept record is never mistaken for a live one.

### `.specs/service/specs/08-persistence.md` → Logical schema (Modify)

The opening sentence's entity list gains the new record (the rest of the section is
unchanged):

> The adapter-agnostic contract every store satisfies, defining `User`, `Session`,
> `AuditEvent`, and `SingleUseRecord` with their required fields and the `status` /
> `severity` / `outcome` enums.

### `.specs/service/specs/00-overview.md` → Scope summary (Modify)

> | Token exchange (`code` and `id_token` grants) | Yes | `crates/core/src/service/exchange.rs`; the `id_token` grant is off by default (`grants.id_token`) and requires a server-issued nonce |

### `.specs/service/specs/00-overview.md` → Decisions (Modify)

> - *Two grant inputs, one of them opt-in.* **`/token` accepts a provider `code` always, and a
>   raw `id_token` only when `grants.id_token = true`.** The direct grant lets browser SDKs
>   post a credential they already hold, but an ID token is a transferable bearer assertion
>   with no back-channel redemption behind it, so it is bound to a service-issued nonce, made
>   single-use, and served only where an operator asks for it.

---

## Type changes

`IdentityClaims` gains a required `signing_alg`; `SingleUseRecord` is new. Both fold into
[`canonical-types.schema.json`](../service/specs/canonical-types.schema.json).

```json
{
  "$comment": "Fragment for 2026-08-05-bind_id_token_grant_replay_protection. Folds into .specs/service/specs/canonical-types.schema.json on merge.",
  "$defs": {
    "IdentityClaims": {
      "required": ["subject", "signing_alg", "raw_claims"],
      "properties": {
        "signing_alg": {
          "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString",
          "description": "JWS algorithm the resolved JWK verified the ID token with (never the untrusted header's). Selects the at_hash digest."
        }
      }
    },
    "SingleUseRecord": {
      "type": "object",
      "required": ["key", "expires_at"],
      "additionalProperties": false,
      "properties": {
        "key": {
          "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString",
          "description": "Namespaced digest: 'nonce:<sha256hex>' or 'assertion:<provider>:[d:]<sha256hex>'. Never a raw nonce or assertion."
        },
        "expires_at": { "$ref": "../../canonical-types.schema.json#/$defs/Timestamp" }
      }
    }
  }
}
```

---

## Implementation notes

Order matters: the port and adapter work must land before the flow can call it.

1. `crates/core/src/config.rs` — add `GrantsConfig { id_token: bool, nonce_ttl: String,
   max_assertion_lifetime: String }` to `AppConfig` (`config.rs:9-23`) with `#[serde(default)]`
   and `Default` giving `false` / `"10m"` / `"1h"`; validate both durations in
   `AppConfig::validate` (`config.rs:45-93`) via `prefix_config_error`/`parse_duration_secs`.
   Leave `config/default.toml` unchanged — the compiled defaults already express the intent,
   and adding an explicit `[grants] id_token = false` is optional documentation.
2. `crates/core/src/ports/repository.rs:24-33` — add `put_single_use` / `take_single_use` to
   `SessionRepository`; extend `cleanup_expired_sessions`' doc comment to cover the sweep.
   Implement in `adapters/{dynamo,postgres,sqlite,lmdb,valkey}` per the 08-persistence table,
   and in `test-utils::MockRepository`. Postgres and SQLite need a `single_use` table in the
   inline idempotent DDL (`CREATE TABLE IF NOT EXISTS`, plus an index on `expires_at` for the
   sweep); LMDB needs a third named database, so `max_dbs(2)` at
   `crates/adapters/src/lmdb/mod.rs:29` becomes `max_dbs(3)`.
3. `crates/core/src/ports/identity_provider.rs:18` — add `fn client_id(&self) -> &str`;
   implement in `crates/adapters/src/oidc/mod.rs` and `crates/providers/src/apple.rs` (both
   already hold the field) and in `MockIdentityProvider`.
4. `crates/core/src/domain/token.rs:74-84` — add `signing_alg: String` to `IdentityClaims`.
   Populate it at `crates/adapters/src/oidc/mod.rs:215` from the `jwk_alg` resolved at
   `:176-192`, and at `crates/providers/src/apple.rs:309` from the `jwk_alg` resolved at
   `:276-286`. Neither `Validation` block changes.
5. New `crates/core/src/service/assertion.rs` — `mint_nonce` and `bind(claims, ctx)` running
   the five checks in the order in 03-service-flows. Read `exp`, `nonce`, `azp`, `at_hash`,
   `jti` and `aud` from `claims.raw_claims`; compare the nonce with a constant-time equality
   only where a value is compared directly (the nonce path compares digests through the
   store, so no timing surface). Wire it into `crates/core/src/service/exchange.rs` after the
   grant branch at `:74-94`.
6. `crates/core/src/service/exchange.rs:13-27` — `ExchangeRequest` gains
   `provider_access_token: Option<String>`; on the code path pass `ProviderTokens.access_token`
   into the same slot.
7. `crates/server/src/routes/` — new `nonce.rs` handler; mount `POST /nonce` in
   `routes/mod.rs:15-19` only when `grants.id_token`; add `provider_access_token` to
   `TokenForm` (`routes/token.rs:14-22`); reject a request carrying an `id_token` field, up
   front and whatever `grant_type` declares, when `grants.id_token` is false — the gate lives
   in the handler because `UnsupportedGrantType` is a server-layer `ApiError` variant
   (`crates/server/src/error.rs:20`), not a domain `Error` the core could return, and the
   handler is shared by the server, Lambda and FFI paths so no interface bypasses it; make
   `grant_types_supported` (`routes/well_known.rs:16`) conditional.
8. Tests, asserted independently for the generic OIDC and Apple providers so neither can
   regress alone: an assertion with no `nonce` is rejected; a `nonce` this service never
   issued is rejected; the same assertion presented twice succeeds once; an `azp` naming a
   sibling client is rejected; a multi-`aud` token with no `azp` is rejected; a mismatched
   `at_hash` is rejected while a correct one passes; an assertion with more than
   `max_assertion_lifetime` remaining is rejected; with `grants.id_token = false` an
   `id_token` field is rejected under both `grant_type=id_token` and
   `grant_type=authorization_code`. Add store-level conformance tests for
   `put_single_use`/`take_single_use` across all five adapters, including the
   expired-record-is-absent case and two concurrent claims of one key succeeding exactly once.

The scan's PoC harnesses (`findings/g1-id-token-grant-replayable/poc/`,
`findings/g1-id-token-grant-replayable-apple/poc/`) are directly reusable as regression
suites: on a fixed build their replay probes must flip from asserting acceptance to
asserting rejection, while their control cases continue to pass.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**`.
2. Fold the `Type changes` `$defs` into
   [`canonical-types.schema.json`](../service/specs/canonical-types.schema.json)
   (`IdentityClaims` properties and `required`; new `SingleUseRecord`).
3. Mirror `SingleUseRecord` into `schemas/datamodel.schema.json`, the cross-adapter logical
   contract 08-persistence names as the source of truth for stored shapes.
4. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, move it to
   `.specs/changes/merged/`.
5. Update `.specs/README.md`'s Change specs table.

---

## Assumptions and open questions

### Assumptions

- Clients of the direct grant can perform an extra round trip before authenticating and can
  pass a value into the provider's `nonce` parameter. Google Identity Services and Sign in
  with Apple both expose it; a client that cannot is not one this grant can safely serve.
- Every supported provider echoes the `nonce` it was given into the ID token's `nonce` claim.
  This is required by OIDC Core §3.1.3.7 and is not an extension.
- No supported provider mints ID tokens with more than an hour to live, so the
  `max_assertion_lifetime` ceiling rejects nothing legitimate at its default.
- Rate limiting stays a gateway concern ([00-overview](../service/specs/00-overview.md)
  Non-goals). `POST /nonce` writes one fixed-size TTL'd record per call, so the state a burst
  can pin is bounded by request rate × `grants.nonce_ttl`, but nothing in the service caps
  that rate.

### Decisions

- *The direct grant defaults off.* **`grants.id_token = false`.** Three reasons, in order of
  weight. It is the only grant whose credential is a transferable bearer assertion with no
  back-channel redemption, so it is the only one whose safety depends entirely on controls
  this service must synthesise. The service's own discovery document has never advertised it
  (`well_known.rs:16`), so an off default makes the shipped metadata true rather than
  merely less wrong. And the nonce requirement is already a breaking client-contract change:
  every existing direct-grant client must fetch a nonce before authenticating, so a
  deployment that keeps the grant is editing its clients anyway and one config line is not
  the cost that matters. The honest counter-argument is that this turns an upgrade into a
  hard failure for deployments using the grant today, rather than a degraded one — which is
  the intended direction for a security default, and belongs in the release note rather than
  in a softer default.
- *Nonce state lives in the session store.* **The single-use methods hang off
  `SessionRepository`, not a new port.** Nonces and replay markers have the same shape as
  sessions — short-lived, high-churn, keyed by a digest — so they land in whichever store
  already holds sessions (`[session_repository]` when set, otherwise `[repository]`), with no
  seventh port to implement five times and no new configuration surface. The cost is that
  `SessionRepository` now holds a concern that is not literally a session.
- *One atomic operation is the whole nonce check.* **`take_single_use` both verifies the
  nonce and consumes it.** An absent nonce, an expired one, and an already-burned one are
  indistinguishable to the caller and all reject, so there is no branch in which a nonce is
  verified but not consumed.
- *`at_hash` on an EdDSA assertion is rejected, not skipped.* **OIDC Core defines no digest
  for EdDSA, so the claim cannot be verified and the assertion is refused.** Accepting a
  binding claim we cannot check would be worse than having none, because it reads as
  enforcement in the audit trail.

### Open questions

- Does any provider a deployment might configure sign ID tokens with EdDSA *and* set
  `at_hash`? If one does, the rejection above becomes a compatibility break and the rule
  needs a per-provider exception rather than a global one.
- `cleanup_expired_sessions` still has no production caller
  ([08-persistence](../service/specs/08-persistence.md) Assumptions). Single-use records
  inherit that gap for space reclamation on Postgres, SQLite and LMDB. Scheduling the reaper
  is a separate change; this one only has to be correct without it, which the
  expired-is-absent rule gives it.
- Should the code-exchange path also require a nonce when the client supplies one? It gains
  nothing over single-use code redemption today, but a provider that reuses codes would make
  it worth having. Left out for now.
- Two sibling change specs edit the same canonical sections; whichever merges later
  reconciles.
  [`2026-08-05-bind_grant_type_at_token_endpoint.md`](2026-08-05-bind_grant_type_at_token_endpoint.md)
  rewrites 04's *POST /token request*, 03's *Token exchange* step 2, and 00's *Two grant
  inputs* Decision, and replaces `ExchangeRequest`'s optional fields with an
  `ExchangeCredential` enum (its Open questions already name this spec as the counterpart).
  Under its rules the field-presence branch selection this spec's `unsupported_grant_type`
  wording guards against no longer exists: a stray `id_token` on
  `grant_type=authorization_code` dies at the parse as `invalid_request`, so the off-switch
  rejection collapses to the `grant_type=id_token` case, and `provider_access_token`
  (implementation note 6) becomes a field of the `IdTokenAssertion` variant rather than of
  the flat struct.
  [`2026-08-05-rotate_refresh_tokens_with_reuse_detection.md`](2026-08-05-rotate_refresh_tokens_with_reuse_detection.md)
  reprints the full `SessionRepository` listing (without the single-use pair), extends
  `cleanup_expired_sessions`, and edits the same 01 query-pattern table and adjacent 08
  sections; merged together, the trait carries both method sets, the sweep's count covers
  sessions, retirement records and single-use records, and LMDB's `max_dbs` is raised once
  for the union of new named databases.
- The discovery document never names the nonce endpoint: `grant_types_supported` gains
  `id_token` when the grant is on, but a direct-grant client must learn `POST /nonce` out of
  band. OIDC discovery registers no member for it; whether to publish a custom one (e.g.
  `nonce_endpoint`) alongside the conditional grant list, or leave the endpoint to client
  documentation, is open.
