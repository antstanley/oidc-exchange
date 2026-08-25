# Change: Make `grant_type` binding at `POST /token`

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** crates/core (service)

Make the declared `grant_type` the sole selector of the flow `POST /token` executes. Each grant
gains a closed parameter set — its members are mandatory, and a parameter belonging to another
grant is rejected rather than ignored — and an absent or unrecognised `grant_type` is an error
in the RFC 6749 §5.2 envelope. The enforcement is structural: `ExchangeRequest` stops being a
bag of `Option<String>` and becomes a struct carrying an `ExchangeCredential` enum whose
variants own their own fields, so a request that mixes two grants is a parse failure at the HTTP
boundary rather than a branch the service can take.

---

## Motivation

`grant_type` is read in exactly one place — the `match` at `crates/server/src/routes/token.rs:29`
— and never travels further. `ExchangeRequest` has no `grant_type` field, and
`crates/core/src/service/exchange.rs:73-94` picks the credential path by asking a different
question: line 74 is `if let Some(ref id_token) = request.id_token`. The
`"authorization_code" | "id_token"` arm forwards `code`, `redirect_uri` and `id_token`
unconditionally, so a request declaring `grant_type=authorization_code` that also carries an
`id_token` field takes the direct-assertion path. The code is never redeemed at the provider,
the client secret is never presented, and the `redirect_uri` requirement — whose own error
string at `exchange.rs:90` names the `authorization_code` grant — is never reached. The declared
grant is documentation; the payload shape is the decision. This is the scan finding
[`g1-grant-type-confusion-token-endpoint`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/findings/g1-grant-type-confusion-token-endpoint/g1-grant-type-confusion-token-endpoint.md)
(CWE-841/843/287), and it also runs in the other direction: a deployment that configured,
documented and advertised only the authorization-code grant still accepts a bare provider ID
token as a complete credential.

The defect is representational, not a missing `if`. Because the request is a struct of
independent optional fields, "which grant is executing" is a property of control flow that any
refactor can silently move. `.specs/development-guidelines.md` already states the rule this
change makes real — *"parse/validate before the service sees it; reject unknown `grant_type`"* —
and the hardening proposal
[`credential-lifecycle-contract.md`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/hardening/proposals/credential-lifecycle-contract.md)
reaches the same conclusion in its Option 2: *"represent the grant as an enum whose variants own
their fields"*, so that invariant CL1 — *"the grant a request executes is determined by its
declared `grant_type` alone"* — "stops being a check and becomes a parse". That proposal names
`grant_type` binding as one of three fixes to land immediately and independently of the wider
port-contract and refresh-rotation work; this change spec is that piece, plus the smallest of
the other two: the `/revoke` validation has its own spec
([2026-08-05-validate_revoke_token_claims.md](2026-08-05-validate_revoke_token_claims.md)),
while the RFC 6749 §5.1 `Cache-Control: no-store` directives
([`g1-token-response-missing-no-store`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/findings/g1-token-response-missing-no-store/g1-token-response-missing-no-store.md))
had no spec at all and land here, because this spec owns the `/token` request and response
shape.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | Rewrite `POST /token request` with the per-grant parameter table, the rejection rule, and the token-endpoint error table; add the RFC 6749 §5.1 response cache directives |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | `Token exchange (exchange.rs)` step 2 selects on `ExchangeCredential`, not field presence; add a Decision |
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md) | Add an `Exchange request types` entity block for `ExchangeRequest` / `ExchangeCredential` |
| [`.specs/service/specs/00-overview.md`](../service/specs/00-overview.md) | Sharpen the *Two grant inputs* Decision to say the grant is declared, not inferred |
| [`canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | Add `ExchangeCredential` and `ExchangeRequest` `$defs` |

[02-ports-and-adapters.md](../service/specs/02-ports-and-adapters.md) is unaffected — no port
signature changes. The bindings specs are unaffected: `crates/ffi/src/lib.rs:69,112` builds and
drives the same router, so the Node, Python and Lambda bindings inherit the new behaviour
without a delta of their own.

---

## Proposed changes

### `.specs/service/specs/04-http-api.md` → POST /token request (Modify)

> ### POST /token request
>
> `application/x-www-form-urlencoded`. `grant_type` is required and **binding**: it alone
> selects the flow, and a request may carry only the parameters its declared grant defines.
>
> ```
> # code exchange:  grant_type=authorization_code & code=… & redirect_uri=… & provider=google
> # direct token:   grant_type=id_token & id_token=… & provider=google
> # refresh:        grant_type=refresh_token & refresh_token=…
> ```
>
> | `grant_type` | Required parameters | Rejected if present |
> |---|---|---|
> | `authorization_code` | `provider`, `code`, `redirect_uri` | `id_token`, `refresh_token` |
> | `id_token` | `provider`, `id_token` | `code`, `redirect_uri`, `refresh_token` |
> | `refresh_token` | `refresh_token` | `provider`, `code`, `redirect_uri`, `id_token` |
>
> A parameter this server knows but that belongs to another grant is **rejected**, not ignored
> — RFC 6749 §3.2's "MUST ignore unrecognized request parameters" covers parameters the server
> does not recognise, which these are not. Parameters outside this set entirely are ignored.
>
> The client names the provider (`provider=google`), not a raw issuer URL. The handler parses
> the form into a `TokenGrant` before calling the service, so a request whose fields do not
> match its declared grant never reaches `AppService`. Response body is `TokenResponse`
> ([01-domain-model.md](01-domain-model.md)).
>
> Token-endpoint errors, in the RFC 6749 §5.2 envelope:
>
> | Condition | HTTP | `error` | `error_description` |
> |---|---|---|---|
> | `grant_type` absent | 400 | `invalid_request` | `missing required parameter: grant_type` |
> | `grant_type` present but not one of the three (including empty) | 400 | `unsupported_grant_type` | `The grant_type parameter is not supported` |
> | a required parameter of the declared grant absent | 400 | `invalid_request` | `missing required parameter: <name>` |
> | a parameter of another grant present | 400 | `invalid_request` | `<name> is not a parameter of the <grant_type> grant` |

### `.specs/service/specs/04-http-api.md` → POST /token response headers (Add)

Appended to the `POST /token request` section:

> Every `/token` response — success and error alike — carries `Cache-Control: no-store` and
> `Pragma: no-cache` (RFC 6749 §5.1 and §5.2; OpenID Connect Core §3.1.3.3). The body of a
> successful response *is* the credential — the signed access token and, on exchange, the
> plaintext refresh token, whose only copy in flight is that response — and the header is
> the origin's sole mechanism for marking it non-storable: a `200` to a `POST` is
> heuristically cacheable under RFC 9111 §3, so without the directive a conforming shared
> cache is *permitted to store* the credential even though it may never reuse it. The
> directives are applied by a route-scoped layer (`middleware/cache_control.rs`) on the
> credential-bearing route group — `/token` and `/revoke` — not per handler, so the next
> credential-returning route inherits them by being mounted in the group. `/revoke`'s
> responses carry no token and RFC 7009 imposes no cache requirement; it is in the group
> because its *requests* carry credentials and because a group-level property survives the
> refactors that a per-handler memory does not. `/keys` and
> `/.well-known/openid-configuration` sit outside the group and keep their own (cacheable)
> policy.

### `.specs/service/specs/03-service-flows.md` → Token exchange (`exchange.rs`) (Modify)

The opening line and step 2 become:

> `POST /token` with `grant_type=authorization_code` or `grant_type=id_token`. The handler has
> already parsed the form into a `TokenGrant`, so `AppService::exchange` receives an
> `ExchangeRequest` whose `credential` names the grant that was declared
> ([04-http-api.md](04-http-api.md)).
>
> 1. **Resolve provider** — look up `request.provider` in the `providers` map; missing →
>    `UnknownProvider`.
> 2. **Obtain verified claims** — match on `request.credential`:
>    - `ExchangeCredential::AuthorizationCode { code, redirect_uri }` → `provider.exchange_code`
>      to get `ProviderTokens`, then `validate_id_token` on the returned `id_token`.
>    - `ExchangeCredential::IdTokenAssertion { id_token }` → `provider.validate_id_token`.
>
>    Both fields of the authorization-code variant are non-optional, so the `redirect_uri`
>    binding is a property of the type rather than a runtime check: there is no field
>    combination that reaches this step carrying a credential for one grant while executing
>    another.

### `.specs/service/specs/03-service-flows.md` → Decisions (Add)

> - *The declared grant is the flow selector.* **`ExchangeRequest` carries an
>   `ExchangeCredential` enum parsed at the HTTP boundary; the service matches on it and never
>   inspects field presence.** An incoherent grant/field combination fails to parse at the edge
>   instead of choosing a branch, so a later refactor cannot re-flatten the decision without
>   deleting the type.

### `.specs/service/specs/01-domain-model.md` → Entities, after `Token types` (Add)

> ### Exchange request types (`service/exchange.rs`)
>
> ```rust
> enum ExchangeCredential {
>     AuthorizationCode { code: String, redirect_uri: String },
>     IdTokenAssertion { id_token: String },
> }
>
> struct ExchangeRequest {
>     credential: ExchangeCredential,
>     provider: String,
>     ip_address: Option<String>,
>     user_agent: Option<String>,
>     device_id: Option<String>,
> }
> ```
>
> `ExchangeCredential` is the typed form of the declared `grant_type`: one variant per exchange
> grant, each owning that grant's required parameters as non-optional fields. The refresh grant
> has its own input type, `RefreshRequest`. `ExchangeRequest` derives no `Default` — a request
> with no credential is not constructible. The three trailing fields are client context captured
> by the audit-context middleware, not grant parameters.

### `.specs/service/specs/00-overview.md` → Decisions (Modify)

> - *Two grant inputs, each explicitly declared.* **`/token` accepts both a provider `code` and
>   a raw `id_token`, and the declared `grant_type` selects which.** Browser SDKs (Google
>   Identity Services) can post the credential they already hold without a second server-side
>   code exchange, while which grant runs stays something the caller declares rather than
>   something inferred from the fields they happened to send.

---

## Type changes

Two new `$defs` on the service schema. `ExchangeCredential` models the closed per-grant
parameter sets directly — `additionalProperties: false` on each variant is the schema-level
statement of the rejection rule.

```json
{
  "$comment": "Fragment for 2026-08-05-bind_grant_type_at_token_endpoint. Adds two $defs to .specs/service/specs/canonical-types.schema.json on merge.",
  "$defs": {
    "ExchangeCredential": {
      "description": "The typed form of the declared grant_type on POST /token. Exactly one variant; each owns its grant's required parameters.",
      "oneOf": [
        {
          "type": "object",
          "title": "AuthorizationCode",
          "required": ["grant_type", "code", "redirect_uri"],
          "additionalProperties": false,
          "properties": {
            "grant_type": { "const": "authorization_code" },
            "code": { "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString" },
            "redirect_uri": { "$ref": "../../canonical-types.schema.json#/$defs/Url" }
          }
        },
        {
          "type": "object",
          "title": "IdTokenAssertion",
          "required": ["grant_type", "id_token"],
          "additionalProperties": false,
          "properties": {
            "grant_type": { "const": "id_token" },
            "id_token": { "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString" }
          }
        }
      ]
    },
    "ExchangeRequest": {
      "type": "object",
      "description": "Input to AppService::exchange. No default construction: credential and provider are always present.",
      "required": ["credential", "provider"],
      "additionalProperties": false,
      "properties": {
        "credential": { "$ref": "#/$defs/ExchangeCredential" },
        "provider": { "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString" },
        "ip_address": { "type": ["string", "null"] },
        "user_agent": { "type": ["string", "null"] },
        "device_id": { "type": ["string", "null"] }
      }
    }
  }
}
```

The error codes this change emits — `invalid_request`, `unsupported_grant_type` — are already in
`OAuthErrorEnvelope` in the repo-wide [`canonical-types.schema.json`](../canonical-types.schema.json);
no change there.

---

## Implementation notes

1. `crates/core/src/service/exchange.rs:12-27` — replace the `code` / `redirect_uri` /
   `id_token` fields with `credential: ExchangeCredential`, add the `ExchangeCredential` enum,
   and **delete `#[derive(Default)]`**. The derive is what currently permits an `ExchangeRequest`
   with no credential at all; keeping it would leave half the hole open.
2. `crates/core/src/service/exchange.rs:73-94` — replace the `if let Some(ref id_token)` selector
   with a `match request.credential`. Both arms shrink: the code arm no longer unwraps
   `Option`s, so its two `InvalidRequest` constructions move to the HTTP boundary and the
   `"either 'code' or 'id_token' is required"` message disappears (nothing can reach the service
   in that state).
3. `crates/server/src/routes/token.rs:14-22` — keep `TokenForm` as the untrusted wire shape and
   add `TokenGrant` (the three grants) plus a `TryFrom<TokenForm> for TokenGrant` that applies
   the per-grant table: unwrap the required members, reject the non-members. `serde_urlencoded`
   (which axum 0.8's `Form` uses, `axum-0.8.9/src/form.rs:87`) supports neither `#[serde(flatten)]`
   nor tagged enums, so this is a hand-written parse, not a serde attribute.
4. `crates/server/src/routes/token.rs:29-64` — give `authorization_code` and `id_token` separate
   arms and dispatch on the `TokenGrant`. Keep the existing `_ => ApiError::UnsupportedGrantType`
   arm for unrecognised values.
5. Absent `grant_type` currently escapes the error envelope entirely: `TokenForm.grant_type` is a
   bare `String`, so a body without it fails deserialization and axum returns
   `FailedToDeserializeFormBody` — **422 with a plain-text body**
   (`axum-0.8.9/src/extract/rejection.rs:76-82`), not `{"error": …}`. Two ways to reach the
   specified `400 invalid_request`: derive `FromRequest` on a wrapper with
   `rejection(ApiError)` and map `FormRejection`, keeping `grant_type: String`; or make
   `grant_type: Option<String>` on the wire type and reject `None` in `TryFrom`. Prefer the
   first — the finding flags "moved a required parameter from `String` to `Option<String>`" as
   the exact shape of the original regression, and the wire type is the wrong place to relax it.
6. `crates/core/tests/exchange.rs` — every `ExchangeRequest { … ..Default::default() }` literal
   (269, 327, 340, 381, and the rest) must be rewritten to name a credential variant and the
   three context fields explicitly. If the churn is unwelcome, group `ip_address` / `user_agent`
   / `device_id` into a `ClientContext` struct that keeps `Default`; that is a mechanical
   refactor and does not weaken the credential invariant.
7. Regression tests, in `crates/server/tests/routes.rs` (all five would have caught the defect or
   a near variant; the scan's `poc/grant_confusion.rs` `ProbeProvider` is the observation point
   for the first two):
   1. `grant_type=authorization_code` carrying both `code` and `id_token` is rejected
      `invalid_request` — asserted on a provider double that neither `exchange_code` nor
      `validate_id_token` was called: the request dies at the parse, and in particular the
      direct-assertion path never runs.
   2. `grant_type=authorization_code` without `redirect_uri` is rejected *even when* an
      `id_token` is supplied.
   3. `grant_type=id_token` without an `id_token` parameter is rejected rather than falling
      through to a code redemption.
   4. `grant_type=refresh_token` carrying `provider` or `code` is rejected.
   5. A body with no `grant_type` at all returns `400 {"error":"invalid_request"}`, not 422.
8. The existing tests at `crates/server/tests/routes.rs:99` (unknown grant → 400
   `unsupported_grant_type`) and `:130` (`authorization_code` with no `code` → 400
   `invalid_request`) keep passing unchanged; they already assert the post-change behaviour.
9. `crates/server/src/middleware/cache_control.rs` — a `from_fn` `no_store_layer` inserting
   `Cache-Control: no-store` and `Pragma: no-cache`, applied in `routes::public_routes()`
   (`crates/server/src/routes/mod.rs:13-23`) to a merged route group containing `/token` and
   `/revoke` only. No new dependency (`tower-http`'s `set-header` feature is not enabled, and
   this avoids the feature bump). `Router::layer` wraps the route's endpoint, so the layer
   runs after `ApiError::into_response` and the §5.2 error envelope is covered; responses
   manufactured by the router-wide layers (the timeout `408`, the catch-panic `500`) carry no
   credential and stay unmarked. Tests in `crates/server/tests/routes.rs`: a successful
   exchange response carries both headers; an `unsupported_grant_type` error response
   carries both; `/keys` and the discovery document carry neither — the fix must not
   blanket-mark the cacheable routes. The finding's probe
   (`.security/oidc-exchange/53cbdec9_20260804T102454Z/findings/g1-token-response-missing-no-store/poc/`)
   exits non-zero once every token response carries both headers, so `make run` failing there
   is the signal the fix landed.

---

## Compatibility and migration

This change rejects requests that succeed today. That is the point, and it is worth stating
plainly rather than burying.

**What breaks.** Any client that relies on field presence rather than its declared grant:

- `grant_type=authorization_code` with an `id_token` field — today runs the direct path, after
  this change is `400 invalid_request`. This is the bypass itself; there is no safe way to keep
  it working.
- `grant_type=id_token` with a stray `code` or `redirect_uri` — today ignored, after this change
  rejected.
- `grant_type=refresh_token` with a `provider` field — today ignored, after this change rejected.
  This is the one rejection with no security value behind it (see Decisions).

**What does not break.** The three shapes documented in
[04-http-api.md](../service/specs/04-http-api.md) work unchanged, and every caller in this
repository that reaches the endpoint already uses one of them: `crates/server/tests/{routes,e2e}.rs`
and `examples/aws-web/demo-app/src/routes/api/login/+server.ts` (which posts exactly
`grant_type=id_token & id_token & provider`). `bindings/lambda/__tests__/adapters.test.ts`
builds abbreviated bodies (`grant_type=authorization_code&code=abc`) that are not among the
three, but they are event-translation fixtures asserted byte-for-byte and never routed, so
those tests are unaffected. No configuration key changes and no stored data changes, so the
change is a straight revert if it goes wrong.

**Migration.** There is no compatibility shim and none is proposed: a mode that keeps accepting
mismatched requests keeps the bypass. Instead —

1. Ship behind a release note that names the three rejected shapes above and their replacements,
   since a caller reading only a 400 cannot tell which parameter offended. The
   `error_description` strings specified in the error table exist for exactly this reason: they
   name the offending parameter and the grant it belongs to.
2. Before release, grep deployment logs or the audit trail for exchanges whose declared grant and
   supplied fields disagree. There is no audit field recording this today, so the practical
   pre-flight is a staging deployment with the change on and the rejection logged at `warn`.
3. Fix the two binding READMEs (`bindings/nodejs/README.md:41`, `bindings/python/README.md:52`)
   while here — they show `authorization_code` with `code` and `provider` but no `redirect_uri`,
   which this endpoint already rejects today, so the snippets are wrong before as well as after.

---

## Merge plan

1. Apply the two [04-http-api.md](../service/specs/04-http-api.md) blocks — the
   `POST /token request` rewrite and the appended response cache directives; bump its
   `**Date:**`. The `Error mapping`
   table on that page already covers `InvalidRequest` and `UnsupportedGrantType` and needs no
   edit.
2. Apply the two blocks to [03-service-flows.md](../service/specs/03-service-flows.md) — the
   exchange-flow rewrite and the new Decision; bump its `**Date:**`.
3. Apply the `Exchange request types` block to
   [01-domain-model.md](../service/specs/01-domain-model.md) after the `Token types` block;
   bump its `**Date:**`.
4. Apply the Decision rewrite to [00-overview.md](../service/specs/00-overview.md); bump its
   `**Date:**`.
5. Fold the `Type changes` `$defs` into
   [`canonical-types.schema.json`](../service/specs/canonical-types.schema.json).
6. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, and move it to
   `.specs/changes/merged/`.
7. Update `.specs/README.md`'s Change specs table — this file is not currently in the pending
   list, so add its row directly under the `changes/merged/` entries.

---

## Assumptions and open questions

### Assumptions

- No shipped client depends on the lax behaviour. Verified for every caller inside this
  repository (see Compatibility and migration); external deployments cannot be verified from
  here, which is why the release note is part of the change rather than an afterthought.
- The `id_token` grant stays a supported grant. This change makes it explicitly declared, not
  optional or disabled; whether it should be gated by configuration is a separate change.
- `crates/ffi` continues to dispatch through `build_router`, so no binding-side parsing needs to
  learn the grant rules.

### Decisions

- *Reject, do not ignore.* **A parameter belonging to another grant is a `400 invalid_request`.**
  A caller sending `grant_type=authorization_code&id_token=…` has a wrong mental model of what
  they are authenticating with, and telling them so is more useful than silently dropping the
  field. RFC 6749 §3.2's obligation to ignore unrecognised parameters covers parameters the
  server does not know; these are ones it knows and has assigned to a different grant.
- *One rule, applied uniformly, including `provider` on refresh.* **`provider` is a member of
  the two exchange grants only, so a refresh request carrying it is rejected.** This single
  rejection carries no security value — `provider` is not a credential and the session already
  records it — but a rule with a carve-out is harder to state, to test, and to keep true through
  a refactor than a rule without one. It is the cheapest thing in this spec to relax if it
  causes real friction.
- *Structural, not a boundary check.* **`ExchangeCredential` is an enum whose variants own their
  fields, and `ExchangeRequest` loses its `Default`.** The previous version of this code also had
  boundary checks and a refactor removed them without anything complaining; a check can be
  deleted, a type has to be replaced. This follows Option 2 of
  [`credential-lifecycle-contract.md`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/hardening/proposals/credential-lifecycle-contract.md),
  which lands `grant_type` binding immediately and independently of that proposal's port
  contract and refresh-rotation phases.
- *No compatibility mode.* **The strict behaviour ships on, with no switch to restore the lax
  one.** A switch that keeps accepting mismatched requests keeps the bypass, and an operator
  cannot tell from the outside whether anyone is relying on it.
- *Present-but-unrecognised is `unsupported_grant_type`; absent is `invalid_request`.* **An empty
  `grant_type` value counts as present.** RFC 6749 §5.2 assigns a missing required parameter to
  `invalid_request` and an unsupported grant to `unsupported_grant_type`; treating empty as a
  value keeps the rule "read the parameter, then classify it" with no third case.
- *`no-store` rides with the `grant_type` binding.* **The RFC 6749 §5.1 response directives
  land in this change, as a route-group layer over `/token` and `/revoke`.** The hardening
  proposal names three fixes to land immediately — the `/revoke` validation, the
  `grant_type` binding, and `Cache-Control: no-store` — and the first two have change specs
  while the third had none; it belongs here because this spec owns the `/token` request and
  response shape. A layer on the route group rather than in the handlers keeps the property
  one of mounting, which is the same structural instinct as the credential enum: a control a
  handler must remember is a control a refactor can forget.

### Open questions

- The discovery document advertises `["authorization_code", "refresh_token"]` as a hand-written
  literal (`crates/server/src/routes/well_known.rs:16`) while the endpoint accepts a third grant
  unconditionally. Gating the `id_token` grant on a config key (the finding proposes
  `token.allow_id_token_grant`, defaulting to `false`) and generating the advertised list from
  the same value is a real change with its own compatibility story — it turns off a grant a
  deployment may be using. It belongs in its own change spec, and now has one:
  [`2026-08-05-bind_id_token_grant_replay_protection.md`](2026-08-05-bind_id_token_grant_replay_protection.md)
  adds a `[grants] id_token` switch (default `false` — the finding's proposed key under a
  different name) and derives the advertised list from it. This change does not depend on that
  one landing; whichever merges second reconciles the `POST /token request` section.
- Should a rejected grant/field mismatch emit an audit event? `ValidationFailed` exists as an
  `AuditEventType` and this is precisely a validation failure at the trust boundary, but the
  rejection happens in the HTTP handler, which has no `AppService` audit path today. Left out of
  this change to keep it to the fix the hardening proposal says to land immediately.
