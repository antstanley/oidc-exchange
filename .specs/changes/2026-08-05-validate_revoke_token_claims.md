# Change: Validate every claim on the token `/revoke` acts on, and bound revocation to the session it names

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** crates/core (service)

Replace `revoke.rs`'s signature-only `verify_and_extract_sub` with a single first-party token
validator — `AppService::validate_access_token` — that establishes type, issuer, audience and
validity window before any claim is readable, and narrow what a validated access token
authorises from "every session belonging to this `sub`" to "the one session this token was
minted for", identified by a new `sid` claim. Today the `access_token` branch of `POST /revoke`
checks the JWS signature and nothing else, so any access token the deployment ever signed —
including one that expired months ago — is a repeatable, unauthenticated primitive that
destroys every session of its subject.

---

## Motivation

`crates/core/src/service/revoke.rs:131-155` splits the JWT, verifies the signature with
`keys.verify`, then reads `sub` out of an untyped `serde_json::Value` and hands it to
`revoke_all_user_sessions` (`revoke.rs:44-53`). `exp` is never read, so the 15-minute nominal
lifetime the service stamps in `build_access_token` (`service/mod.rs:70-77`) provides no
protection whatever; `iss` and `aud` are never read, so a token minted by a sibling deployment
sharing the signing key is equally acceptable; and the header — authenticated, but never
decoded — is never checked, so nothing binds the presented artifact to being an *access token*
rather than any other JWT the same key might one day sign. A signature establishes origin. It
does not establish that the bytes are still a credential for this service, right now. That is
the project's own threat-model invariant I1, and `revoke.rs::verify_and_extract_sub` is named
in it.

The asymmetry is stark: every JWT this service receives from somebody else is validated
properly — `crates/adapters/src/oidc/mod.rs:193-197` and `crates/providers/src/apple.rs:287-291`
pin the issuer and audience and set `exp`/`iss`/`aud` as required spec claims, per
[2026-07-01-require_iss_aud_in_token_validation](merged/2026-07-01-require_iss_aud_in_token_validation.md)
— and the only JWT validated carelessly is the one this service minted itself. Beyond the
missing checks there is a second defect the checks alone do not reach: an access token is not
bound to any session, so "revoke this token" has been implemented as "revoke everything this
subject owns". That blast radius is what turns a scavenged token from a stale artifact into a
sustained lockout weapon, and it is the part worth fixing structurally rather than temporally.

---

## Affected spec pages

| Canonical page | Nature of change |
| --- | --- |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | Rewrite `Revocation (revoke.rs)` around the shared validator and session-scoped authority; add a `Validate access token` section; add `sid`/`typ` to `Build access token`; extend the reserved-claim set under `Custom claims`; add five Decisions |
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md) | `AccessTokenClaims` gains `sid`; note that `Session.refresh_token_hash` doubles as the session identifier the access token carries |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | Correct the `KeyManager::verify` note — the signature check is the first step of validation, not the whole of it |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | `/revoke` row in the public-routes table; add a Decision recording the revocation authority model |
| [`canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | Add `sid` to `AccessTokenClaims` `properties` and `required` |

No new canonical page. [08-persistence.md](../service/specs/08-persistence.md) is unaffected —
no store schema changes and no new port method; `revoke_all_user_sessions` keeps its adapter
implementations and its admin-path caller.

---

## Proposed changes

### `.specs/service/specs/03-service-flows.md` → Revocation (`revoke.rs`) (Modify)

> `POST /revoke` (RFC 7009 — token-state failures still succeed toward the client; backend
> failures propagate). Revocation authority comes from the credential the caller presents, and
> reaches exactly the session that credential names. `/revoke` never removes a session the
> caller presented no credential for.
>
> - hint `refresh_token`, absent, or unknown → SHA-256 hex the token,
>   `get_session_by_refresh_token(hash)`, and on a match `revoke_session(hash)` (audited
>   `TokenRevocation`). A missing session is `Ok` (idempotent delete, 200) and emits nothing; a
>   store error propagates, and the server maps it to 503.
> - hint `access_token` → `validate_access_token(token)` (below). The returned claims carry
>   `sid`, the `refresh_token_hash` of the session the token was minted for; `revoke_session(sid)`
>   removes that one session (audited `TokenRevocation`). The subject's other sessions are
>   untouched. `revoke_all_user_sessions` is not reachable from this endpoint.
> - Any validation failure — malformed, wrong type, bad signature, expired, wrong issuer or
>   audience — revokes nothing and emits one `AuthenticationFailed` event (rendered
>   `ValidationFailed`) with a fixed reason string, then returns 200 like every other
>   token-state outcome. The client cannot distinguish a rejected token from an accepted one
>   (RFC 7009 §2.2); an operator can see the attempt. Both branches emit exactly one event,
>   which is what keeps them indistinguishable under `audit.durability = "enforce"` as well as
>   in normal operation.

This block lands on the Revocation section as
[2026-08-05-audit_and_throttle_authentication_failures.md](2026-08-05-audit_and_throttle_authentication_failures.md)
leaves it — that spec merges first, and its Revocation delta still has the access-token path
audit `SessionsRevoked` (revoke-all semantics), because it documents the flow before this
narrowing. This spec merges after it and **supersedes** that wording: once this block is
applied, the access-token path reaches one session and audits `TokenRevocation`, and no
shipped flow emits `AllSessionsRevoked` (see Assumptions).

### `.specs/service/specs/03-service-flows.md` → Validate access token (`service/mod.rs::validate_access_token`) (Add)

A new section immediately after `Build access token`, so mint and verify read together.

> The only path by which a claim of a service-minted JWT becomes readable. It returns
> `AccessTokenClaims` or a fixed rejection reason; a caller cannot reach `sub` without having
> proved everything below.
>
> 1. Split on `.` — exactly three segments, each base64url-no-pad decodable.
> 2. Header: `alg == keys.algorithm()`, `kid == keys.key_id()`, `typ == "at+jwt"`. The header is
>    covered by the signature but is not self-authenticating, so it is pinned to what this
>    service mints rather than read for direction (threat-model I2).
> 3. `keys.verify(signing_input, signature)` over `header.payload`. No claim is read before this
>    step succeeds.
> 4. Deserialize the payload into `AccessTokenClaims`. `sub`, `iss`, `aud`, `iat`, `exp` and
>    `sid` are required fields, so a missing claim is a parse failure rather than a check that
>    can be omitted.
> 5. `iss == server.issuer`; `aud == token.audience` (the empty string when unset — the same
>    value `build_access_token` stamps, so the two agree by construction).
> 6. `exp > now`; `iat <= now`; `nbf <= now` when the payload carries one. Each comparison
>    allows 60 seconds of clock skew.
> 7. `sub` and `sid` are non-empty.

### `.specs/service/specs/03-service-flows.md` → Build access token (Modify)

Steps 2 and 3 of the existing list:

> 2. Assemble `AccessTokenClaims { sub: user.id, iss: server.issuer, aud: token.audience or "",
>    iat, exp, sid, custom }`, where `sid` is the `refresh_token_hash` of the session this token
>    is minted for — supplied by the caller, from the session `exchange` has just stored or the
>    one `refresh` has just read — and `custom` comes from `resolve_custom_claims`.
> 3. Header `{ alg: keys.algorithm(), typ: "at+jwt", kid: keys.key_id() }` — the RFC 9068 media
>    type for a JWT access token, which `validate_access_token` requires.

### `.specs/service/specs/03-service-flows.md` → Custom claims (Modify)

> Reserved names `sub`, `iss`, `aud`, `iat`, `exp`, `nbf` and `sid` are silently dropped from
> both sources. `sid` carries revocation authority and `nbf` bounds validity, so neither may be
> set from a config template or a per-user claim.

### `.specs/service/specs/03-service-flows.md` → Decisions (Add)

> - *One validator for first-party tokens.* **Every read of a claim from a JWT this service
>   minted goes through `AppService::validate_access_token`.** `exchange` delegates JWT
>   validation to the provider adapters and `refresh` validates an opaque token against the
>   session store, so neither validates a first-party JWT; `revoke`'s hand-rolled check was the
>   only one in the workspace, and hand-rolling is what made stopping after the signature
>   possible.
> - *Required claims are parse-enforced.* **`sub`, `iss`, `aud`, `iat`, `exp` and `sid` are
>   required fields of `AccessTokenClaims`, so presence is a deserialization outcome, not a
>   check.** The same discipline `set_required_spec_claims` gives the provider paths, in a crate
>   that carries no `jsonwebtoken` dependency.
> - *A credential revokes only its own session.* **The access-token branch revokes the single
>   session named by `sid`.** A stateless access token is not a session credential; treating it
>   as authority over every session of its subject gave any holder of any leaked token an
>   account-wide logout. Account-wide revocation remains on the authenticated admin path —
>   `apply_validated_patch` revokes every session when a status patch moves a user into
>   `Suspended` or `Deleted`, on behalf of both `admin_update_user` and `admin_delete_user`.
> - *`sid` is the session's refresh-token hash.* **The access token carries the session's
>   existing primary key rather than a new identifier.** `revoke_session` already takes that
>   hash, so no `Session` field, port method or store migration is needed. The digest becomes
>   visible to any holder of the access token; it cannot be replayed as a refresh token (both
>   the refresh and the refresh-revoke paths hash the *presented* value before lookup) and it is
>   a SHA-256 of 256 CSPRNG bits, so the authority it confers is exactly the authority the
>   access token already implies. What `sid` *means* is fixed independently of what it
>   *contains*: it denotes **the current session identifier** — whatever value names the one
>   session the token was minted for — and the hash is merely the value that identifier takes
>   while refresh does not rotate, which is true at this spec's merge point.
>   [2026-08-05-rotate_refresh_tokens_with_reuse_detection.md](2026-08-05-rotate_refresh_tokens_with_reuse_detection.md)
>   merges later and supersedes this binding with the rotation-independent `family_id`;
>   `/revoke`'s access-token arm must resolve whichever identifier is current — the hash
>   before that sibling lands, the `family_id` after.
> - *Failed revocation is recorded, not silent.* **A rejected `/revoke` emits one
>   `AuthenticationFailed` event and still returns 200.** RFC 7009 §2.2 constrains what the
>   caller observes, not what the operator records — and an unauthenticated endpoint that
>   answers 200 regardless is precisely the one whose abuse is invisible without a record. On
>   the mandatory `SecurityEvent` channel this spec inherits from
>   [2026-08-05-audit_and_throttle_authentication_failures.md](2026-08-05-audit_and_throttle_authentication_failures.md)
>   no threshold gates emission, so severity no longer decides whether the event survives or
>   whether a sink outage fails the request; `audit.durability` decides, identically for both
>   branches. That symmetry is the control: emitting only on success would answer 503 for a
>   token that existed and 200 for one that did not whenever the sink is down — reintroducing,
>   as degraded-mode behaviour, the existence oracle the silence was meant to prevent.

### `.specs/service/specs/01-domain-model.md` → Entities, Session (Modify)

Appended to the paragraph following the `Session` struct:

> `refresh_token_hash` is also the session's identifier: it is the key every
> `SessionRepository` lookup and revocation takes, and it is the value minted access tokens
> carry as their `sid` claim so a presented access token names the session it belongs to.

### `.specs/service/specs/01-domain-model.md` → Token types (`domain/token.rs`) (Modify)

> - **`AccessTokenClaims`** — JWT payload: `sub` (internal user id), `iss`, `aud`, `iat`, `exp`,
>   `sid` (the `refresh_token_hash` of the session the token was minted for), plus a flattened
>   `custom: HashMap<String, Value>` of resolved claims. All six registered fields are required
>   on both serialization and deserialization.

### `.specs/service/specs/02-ports-and-adapters.md` → KeyManager (Modify)

> `verify` exists so `AppService::validate_access_token` can authenticate a service-minted
> access token before any of its claims is read. The signature check is the first step of that
> validation, not the whole of it: origin is established here, and validity — type, issuer,
> audience and window — by the claim checks that follow
> ([03-service-flows.md](03-service-flows.md)).

### `.specs/service/specs/04-http-api.md` → Routes, Public (Modify)

> | POST | `/revoke` | `revoke` | RFC 7009 revocation of the session the presented credential names: 200 for invalid/unknown tokens, 503 on backend failure |

### `.specs/service/specs/04-http-api.md` → Decisions (Add)

> - *Revocation reaches one session.* **`/revoke` removes the session named by the credential
>   presented and nothing else.** The endpoint is unauthenticated by design (RFC 7009 §2.1
>   permits it, and the token is the credential), so its blast radius must be the credential's
>   own; a public endpoint that can end every session of a named subject is a denial-of-service
>   primitive for anyone who scavenges one token. Ending all of a user's sessions is an
>   operator action and lives behind internal auth.

---

## Type changes

`AccessTokenClaims` gains a required `sid`. Folds into the existing definition in
[`canonical-types.schema.json`](../service/specs/canonical-types.schema.json).

```json
{
  "$comment": "Fragment for 2026-08-05-validate_revoke_token_claims. Folds into .specs/service/specs/canonical-types.schema.json#/$defs/AccessTokenClaims on merge; `sid` also joins that definition's `required` array.",
  "$defs": {
    "AccessTokenClaims": {
      "required": ["sub", "iss", "aud", "iat", "exp", "sid"],
      "properties": {
        "sid": {
          "type": "string",
          "pattern": "^[0-9a-f]{64}$",
          "description": "Session identifier: the SHA-256 hex refresh_token_hash of the session this access token was minted for. Names the only session the token may revoke."
        }
      }
    }
  }
}
```

---

## Implementation notes

1. `crates/core/src/domain/token.rs:35-46` — add `pub sid: String` to `AccessTokenClaims`, above
   the flattened `custom`. Required on both directions; `#[serde(flatten)]` keeps unknown
   payload keys working.
2. `crates/core/src/service/mod.rs:66-100` — `build_access_token(&self, user: &User, sid: &str)`;
   set `sid: sid.to_string()` in the claim struct and change the header `typ` from `"JWT"` to
   `"at+jwt"`.
3. Call sites: `crates/core/src/service/exchange.rs:316` — the local `token_hash` is moved into
   `Session.refresh_token_hash` at line 305, so pass `&session.refresh_token_hash`;
   `crates/core/src/service/refresh.rs:107` — pass `&session.refresh_token_hash` from the
   session read at line 27.
4. `crates/core/src/service/mod.rs` — add `pub(crate) async fn validate_access_token(&self,
   token: &str) -> std::result::Result<AccessTokenClaims, &'static str>` directly below
   `build_access_token`. The `Err` carries a fixed, non-attacker-derived reason used only as the
   audit `reason`; it never reaches the client. Steps in the order given in the
   `Validate access token` block above — signature before any claim read (I1).
5. `crates/core/src/service/revoke.rs:131-155` — delete `verify_and_extract_sub` entirely, along
   with its `base64`/`serde_json` imports if nothing else in the file uses them.
6. `crates/core/src/service/revoke.rs:35-77` — both branches converge on `revoke_session(hash)`:
   the `access_token` arm calls `validate_access_token`, takes `claims.sid`, and reuses the same
   lookup-then-revoke-then-audit body as `revoke_refresh_token` (extract it into one helper
   taking a session hash). The existing non-empty-`sub` assertion at lines 49-52 becomes a
   postcondition the validator guarantees rather than a hope — the old code returned `Some("")`
   for a blank `sub` and would have tripped that assertion.
7. `crates/core/src/service/claims.rs:8` — extend `RESERVED_CLAIMS` to
   `["sub", "iss", "aud", "iat", "exp", "nbf", "sid"]` and update the doc comment above
   `resolve_custom_claims`. Without this a per-user claim named `sid` collides with the struct
   field in the flattened payload.
8. Tests in `crates/core/tests/revoke.rs`, beside the existing
   `revoke_forged_access_token_does_not_revoke_sessions` and
   `revoke_failed_verification_access_token_emits_nothing` (today's negative cases cover only
   malformed shapes and bad signatures, which is why the missing claim checks looked
   covered): expired `exp`; future `nbf`; wrong `iss`; wrong
   `aud`; header `alg` other than the key manager's; header `typ` other than `at+jwt`; missing
   `exp`; missing `sid`. Each must revoke nothing and emit one `ValidationFailed`. Rework
   `revoke_access_token_removes_all_user_sessions` and
   `revoke_valid_access_token_emits_all_sessions_revoked` into the new semantics — a valid token
   revokes its own session, emits `TokenRevocation`, and leaves the subject's other sessions
   live.
9. `crates/core/tests/exchange.rs:296,364-369,1484` and `crates/core/tests/refresh.rs:127`
   deserialize `AccessTokenClaims`; add `sid` assertions there (exchange: equals the SHA-256 hex
   of the returned refresh token; refresh: unchanged across a refresh, since refresh does not
   rotate).

References — the finding and the structural proposal this delta implements the immediate half
of, in the sealed scan bundle at `.security/oidc-exchange/53cbdec9_20260804T102454Z/`:
[`findings/g1-revoke-accepts-signature-only-tokens/`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/findings/g1-revoke-accepts-signature-only-tokens/g1-revoke-accepts-signature-only-tokens.md)
(with a runnable PoC whose assertions invert into regression tests),
[`hardening/proposals/credential-lifecycle-contract.md`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/hardening/proposals/credential-lifecycle-contract.md)
(Option 2; the `/revoke` fix is listed as shippable immediately and independently), and
[`artifacts/01_context/threat_model.md`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/artifacts/01_context/threat_model.md)
invariants I1, I2, I4, I17, I21. External: RFC 7009 §2.1–§2.2, RFC 9068 §2.1 (`at+jwt`),
RFC 8725 §3.1.

---

## Merge plan

1. Apply the five `Proposed changes` blocks for
   [03-service-flows.md](../service/specs/03-service-flows.md) — the rewritten Revocation
   section (replacing the access-token `SessionsRevoked` wording the audit sibling's earlier
   merge left there), the new `Validate access token` section, the two `Build access token`
   steps, the reserved-claim sentence, and the five Decisions; bump its `**Date:**`.
2. Apply the two blocks for [01-domain-model.md](../service/specs/01-domain-model.md) (Session
   prose, `AccessTokenClaims` bullet); bump its `**Date:**`.
3. Apply the `KeyManager` block to
   [02-ports-and-adapters.md](../service/specs/02-ports-and-adapters.md) and the routes-table row
   plus Decision to [04-http-api.md](../service/specs/04-http-api.md); bump both `**Date:**`
   fields.
4. Fold the `Type changes` fragment into
   [`canonical-types.schema.json`](../service/specs/canonical-types.schema.json)
   (`$defs/AccessTokenClaims` — the `sid` property and its `required` entry).
5. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, and move it to
   `.specs/changes/merged/`.
6. Update `.specs/README.md`'s Change specs table.

---

## Assumptions and open questions

### Assumptions

- Every session an access token can name is keyed by its `refresh_token_hash`, and refreshing
  does not rotate it (the *Refresh does not rotate* Decision in
  [03-service-flows.md](../service/specs/03-service-flows.md)), so a `sid` minted at exchange
  stays valid for the session's whole life. This holds at this spec's merge point and stops
  holding when the rotation sibling merges; the *`sid` is the session's refresh-token hash*
  Decision above records the semantics that survive that change.
- `token.audience` unset renders as the empty string in both `build_access_token` and
  `validate_access_token`, so the comparison is self-consistent. A deployment that *adds* an
  audience after issuing tokens will find previously issued tokens rejected for revocation —
  correct behaviour, and a release-note line.
- Access tokens minted before this change carry `typ: "JWT"` and no `sid` and are therefore
  rejected at `/revoke`. The exposure window is one `token.access_token_ttl` (default 15
  minutes) after deploy, and the failure is fail-closed.
- Downstream resource servers do not require the header `typ` to be exactly `"JWT"`; RFC 9068
  names `at+jwt` for this artifact and most verifiers do not inspect `typ` at all.
- After this change nothing emits `AllSessionsRevoked`: the `/revoke` access-token branch
  (`revoke.rs:55`) was its only emitter, and the admin paths emit `UserSuspended`/`UserDeleted`
  around their `revoke_all_user_sessions` call. The variant stays in `AuditEventType`; an
  operator alerting on it should watch `TokenRevocation` and the admin events instead.

### Decisions

- *Land the validation and the scope change together.* **One change spec, not two.** Validating
  claims without narrowing scope leaves a live captured token as an account-wide logout for its
  full TTL; narrowing scope without validating claims leaves an expired token able to end a
  session forever. Neither half is complete alone, and both touch the same twenty lines.
- *No `jsonwebtoken` dependency in `crates/core`.* **`validate_access_token` is hand-written
  against `keys.verify` and `serde`.** Adding the crate to core to validate a token core itself
  minted with the `KeyManager` port would pull a JWT stack into the hexagon's centre; typing the
  payload as `AccessTokenClaims` gets the required-claims guarantee that
  `set_required_spec_claims` gets on the adapter side.
- *60 seconds of clock skew.* **`exp`, `iat` and `nbf` comparisons allow 60 s in each
  direction.** Multi-node deployments and Lambda cold starts drift; a tighter bound rejects
  legitimate tokens, and 60 s is negligible against a 15-minute TTL.

### Open questions

- Should an authenticated "sign out everywhere" exist on the internal API now that the public
  endpoint no longer offers one? After this change `revoke_all_user_sessions` is reached only
  from `apply_validated_patch`, when an admin status patch moves a user into `Suspended` or
  `Deleted`. If operators need the capability without suspending or deleting the account, it
  wants an `/internal/users/{id}/sessions` DELETE and its own change spec.
- **Settled:** refresh-token rotation
  ([2026-08-05-rotate_refresh_tokens_with_reuse_detection.md](2026-08-05-rotate_refresh_tokens_with_reuse_detection.md))
  would orphan a hash-valued `sid` on the first refresh — every outstanding access token
  would name a hash the store had just retired, and access-token revocation would silently
  become a no-op. The resolution is a staged migration matching the merge order. This spec
  keeps `sid = refresh_token_hash`, which is correct while no rotation exists, and defines
  `sid` as the current session identifier (the Decision above). The rotation spec, in the
  same change that introduces rotation, re-points `sid` to the rotation-independent
  `family_id`, re-targets `/revoke`'s access-token arm at `revoke_family`, and fails closed
  on hash-valued `sid`s minted before its deploy rather than letting them silently match
  nothing. Neither spec merges a state in which a live token's `sid` cannot be resolved.
- **Settled:** [2026-08-05-audit_and_throttle_authentication_failures.md](2026-08-05-audit_and_throttle_authentication_failures.md)
  originally recorded the opposite decision for this endpoint — *Revocation stays silent on
  failure* — arguing that a per-token event reconstructs the existence oracle RFC 7009 §2.2
  forbids. That argument does not survive its own durability model: on the mandatory channel
  under `durability = "enforce"`, emitting only on success answers 503 for a token that
  existed and 200 for one that did not whenever the sink is down, so the silence *creates* the
  oracle rather than preventing it. That spec now records *Revocation records both outcomes*,
  and both specs specify symmetric emission with an unchanged 200. The residual concern the
  silence was really carrying — unbounded audit volume from unauthenticated probing — is
  handled by the per-IP limiter that same spec introduces.
- Whether `/revoke` should also reject a token whose `sid` names a session that has already
  expired but not yet been reaped, rather than relying on `revoke_session` being idempotent, is
  unsettled; it changes nothing observable but would make the audit record more truthful.
