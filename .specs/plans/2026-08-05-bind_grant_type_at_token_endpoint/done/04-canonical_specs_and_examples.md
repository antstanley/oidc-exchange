# 04 · Canonical specs and binding examples

**Plan:** [plan.md](../plan.md) · **Source:** [.specs/changes/merged/2026-08-05-bind_grant_type_at_token_endpoint.md](../../../changes/merged/2026-08-05-bind_grant_type_at_token_endpoint.md)

**Implements:** all affected canonical pages and the source-spec compatibility/documentation requirements.

**Depends on:** 01 (contract), 02 (contract), 03 (review)

**Produces:** canonical service prose/schema and binding README examples that exactly describe the implemented strict grant parsing and credential-response cache policy.

**Pointers:** `.specs/service/specs/{00-overview,01-domain-model,03-service-flows,04-http-api}.md`; `.specs/service/specs/canonical-types.schema.json`; `bindings/nodejs/README.md:37-42`; `bindings/python/README.md:48-53`; source-spec merge plan and proposed blocks.

## Steps

- [ ] Update `04-http-api.md`: replace the `/token` request text with the binding `grant_type` per-grant required/rejected parameter table; document ignored unknown parameters, exact OAuth error table, and all-response `Cache-Control: no-store` / `Pragma: no-cache` behavior for the `/token` + `/revoke` route group. Bump its date.
- [ ] Update `03-service-flows.md`: state that the handler passes a typed `ExchangeRequest` selected by declared grant; describe exhaustive `ExchangeCredential` matching; add the declared-grant decision. Bump its date.
- [ ] Update `01-domain-model.md`: add the `ExchangeCredential` and non-default `ExchangeRequest` entity block after token types, including context fields and the separate `RefreshRequest` boundary. Bump its date.
- [ ] Update `00-overview.md`: revise the two-grant-input decision so `grant_type`, not field presence, selects code vs ID-token exchange. Bump its date.
- [ ] Add `ExchangeCredential` and `ExchangeRequest` definitions to `.specs/service/specs/canonical-types.schema.json`, using closed `oneOf` variants with the implementation's naming and all required fields. Validate JSON and `$ref` paths; retain the repo-wide OAuth error-envelope schema unchanged.
- [ ] Correct the Node and Python binding README request snippets to use `application/x-www-form-urlencoded`, encode an authorization-code request with `redirect_uri`, and retain `provider`; ensure examples match the endpoint’s required shape rather than the current JSON/missing-redirect form.
- [ ] Review all internal Markdown links and source spec references. Do not mark the source spec Merged, move it, update the change-spec table, or add done certificates; those merge lifecycle actions are explicitly outside this unstacked PR's plan implementation.

## Definition of done

- [ ] Each of the five source-identified canonical targets is updated and says no more or less than the implementation/tests prove.
- [ ] Schema definitions represent the closed exchange credential variants and a required credential/provider request without adding invented API fields.
- [ ] Node/Python examples are valid form-encoded authorization-code requests including `redirect_uri`.
- [ ] Discovery/id-token grant gating remains explicitly out of scope; no unrelated proposed spec is absorbed.
- [ ] All local Markdown links resolve, JSON parses, source-spec requirements map to one of tasks 01–04, and the README plan index is updated by the planning change (not deferred).
- [ ] No certificate file is created; the user explicitly prohibited done certificates.
