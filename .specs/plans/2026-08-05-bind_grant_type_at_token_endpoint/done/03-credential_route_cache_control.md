# 03 · Credential route cache control

**Plan:** [plan.md](../plan.md) · **Source:** [.specs/changes/merged/2026-08-05-bind_grant_type_at_token_endpoint.md](../../../changes/merged/2026-08-05-bind_grant_type_at_token_endpoint.md)

**Implements:** source-spec `POST /token response headers` section and implementation note 9; the cache-directive portion of [04-http-api.md](../../../service/specs/04-http-api.md).

**Depends on:** 02 (build — validates route error responses after token parsing has the required OAuth envelope)

**Produces:** route-scoped `Cache-Control: no-store` and `Pragma: no-cache` headers on all `/token` and `/revoke` responses, without changing cache behavior for public metadata endpoints.

**Pointers:** `crates/server/src/routes/mod.rs:13-23`; `crates/server/src/middleware/mod.rs`; existing `from_fn` middleware patterns in `crates/server/src/middleware/{audit_context,request_id}.rs`; `crates/server/tests/routes.rs` public-router helper and token/discovery tests.

## Steps

- [ ] Add `crates/server/src/middleware/cache_control.rs` with a small `from_fn` middleware that runs the next service, inserts `Cache-Control: no-store` and `Pragma: no-cache` into the resulting response, and returns it. Export it from `middleware/mod.rs`.
- [ ] In `public_routes()`, build a merged credential route group containing only `POST /token` and `POST /revoke`, apply the cache layer to that group, then merge it with `/health`, `/keys`, and discovery. Do not apply the layer router-wide and do not add a `tower-http` feature/dependency just for header insertion.
- [ ] Preserve behavior for handler-produced successes and `ApiError::into_response` errors: since the route-scoped middleware observes `next.run()`'s response, both must carry the headers. Do not claim coverage for router-wide timeout/catch-panic responses, which are outside this route group and do not return credentials.
- [ ] Add server route tests proving: a successful `/token` response has both exact headers; an unsupported-grant `/token` OAuth error has both; `/revoke` has both; `/keys` and `/.well-known/openid-configuration` have neither. Keep the current success/error body and status assertions.
- [ ] Run focused server route and E2E tests, then workspace format/lint/tests as appropriate. If full `cargo test --workspace` still reports the known three missing-`providers.*.adapter` config failures, record them as pre-existing and do not modify config code.

## Definition of done

- [ ] Every successful and handler-error `/token` response contains `Cache-Control: no-store` and `Pragma: no-cache`.
- [ ] `/revoke` receives the same headers through the shared credential route group.
- [ ] `/keys`, discovery, and health are not blanket-marked no-store.
- [ ] The layer is route-scoped and mechanically inherited by any route intentionally added to the credential group.
- [ ] No new dependency or feature bump is introduced solely for cache headers.
- [ ] No certificate file is created; the user explicitly prohibited done certificates.
