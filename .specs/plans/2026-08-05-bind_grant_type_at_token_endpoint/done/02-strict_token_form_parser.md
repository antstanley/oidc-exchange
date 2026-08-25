# 02 · Strict token form parser

**Plan:** [plan.md](../plan.md) · **Source:** [.specs/changes/merged/2026-08-05-bind_grant_type_at_token_endpoint.md](../../../changes/merged/2026-08-05-bind_grant_type_at_token_endpoint.md)

**Implements:** source-spec `POST /token request` table and errors, implementation notes 3–8, and grant-boundary behavior for [04-http-api.md](../../../service/specs/04-http-api.md).

**Depends on:** 01 (contract — parser constructs the new `ExchangeCredential` / `ExchangeRequest` types)

**Produces:** an HTTP-boundary parser that makes declared `grant_type` binding and returns every specified malformed-grant case as a 400 OAuth error envelope before core or a provider is called.

**Pointers:** `crates/server/src/routes/token.rs:14-65`; `crates/server/src/error.rs:16-38`; `crates/server/tests/routes.rs:68-146`; `crates/test-utils/src/lib.rs:493-554`.

## Steps

- [ ] Keep `TokenForm` as the untrusted flattened wire shape and add a private `TokenGrant` discriminated representation for authorization-code, ID-token assertion, and refresh grants. Parse explicitly; do not use serde tagged enums/flattening, which is incompatible with the axum 0.8 form path described by the source spec.
- [ ] Ensure a missing `grant_type` reaches `ApiError` as `400 invalid_request` with `missing required parameter: grant_type`, rather than axum's default form-rejection response. Prefer a `FromRequest` wrapper with an `ApiError` rejection mapping that preserves `TokenForm.grant_type: String`; validate the exact pinned-axum rejection API while implementing.
- [ ] Implement `TryFrom<TokenForm> for TokenGrant` (or equivalent isolated parser) with the source table exactly: `authorization_code` requires `provider`, `code`, `redirect_uri` and rejects `id_token`, `refresh_token`; `id_token` requires `provider`, `id_token` and rejects `code`, `redirect_uri`, `refresh_token`; `refresh_token` requires `refresh_token` and rejects `provider`, `code`, `redirect_uri`, `id_token`.
- [ ] Return `InvalidRequest` for each missing required member with `missing required parameter: <name>`, reject a known field assigned to another grant with `<name> is not a parameter of the <grant_type> grant`, and retain `ApiError::UnsupportedGrantType` for present unsupported or empty values. Ignore parameters entirely outside the known form set.
- [ ] Dispatch `token_handler` on `TokenGrant`, constructing `ExchangeRequest { credential, provider, audit context }` only for exchange grants and `RefreshRequest` only for refresh. Keep handlers as parse/validate/call-core/map-response only.
- [ ] Add parser/route assertions and tests in `crates/server/tests/routes.rs`. Extend `MockIdentityProvider` with safe, deterministic call counters or use a local observing double to prove rejected payloads do not call either provider method.
- [ ] Cover: valid authorization-code, ID-token, and refresh requests; existing unknown grant and missing-code behavior; missing `grant_type` → JSON 400 invalid_request; empty/unknown `grant_type` → 400 unsupported_grant_type; each missing required member; every cross-grant field rejection; unknown unrelated form parameter ignored; and the regression where declared authorization-code includes code plus ID token, which must fail before either provider call.
- [ ] Preserve existing server E2E code-and-refresh flow and audit-context tests, updating only their constructor/import expectations as task 01 requires.

## Definition of done

- [ ] The declared, non-empty-supported `grant_type` is the sole selector of the executed token flow.
- [ ] A field belonging to another known grant is rejected at the HTTP boundary; it is never silently ignored or used to choose a service branch.
- [ ] Missing `grant_type` is a `400` JSON OAuth `invalid_request`, not axum's `422` plain-text/form rejection.
- [ ] Empty and unrecognized grant values are `400 unsupported_grant_type` with the existing stable description.
- [ ] Required-field error descriptions and cross-grant error descriptions exactly match the source specification.
- [ ] Regression tests prove malformed mixed requests invoke neither provider method; valid id-token and refresh flows retain their documented behavior.
- [ ] No certificate file is created; the user explicitly prohibited done certificates.
