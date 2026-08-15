# 06 · Generated internal API client

**Status:** Backlog  
**Implements:** [source spec](../../../changes/2026-08-05-harden_admin_plane.md) §"Implementation notes" step 6; [admin-ui overview](../../../admin-ui/specs/00-overview.md) Internal API client/Environment/Decisions targets  
**Depends on:** 01 · tactical console path/status fixes; 04 · separate public and admin listeners; 05 · operator principal and attribution  
**Produces:** `schemas/internal-api.schema.json` is the published contract and build generation produces the admin client/types with encoded paths, service wire enums, operator credentials, and cursor paging.

**Pointers:** `schemas/datamodel.schema.json`; `apps/admin-ui/src/lib/api.ts`; `apps/admin-ui/src/lib/types.ts`; `apps/admin-ui/package.json`; root workspace scripts; `apps/admin-ui/src/routes/(app)/users`; internal routes/schema types.

## Work

- Publish an internal API schema covering the paths, `UserStatus`, `UserPage`, `OperatorPrincipal`-relevant credential contract, cursor query/response, and 429 semantics needed by the console.
- Add deterministic build-time generation and freshness checking; replace the handwritten `api()` and handwritten types rather than layering generated fragments over them.
- Generate percent-encoded path parameters and canonical snake_case enums by construction; update list UI/loaders to use cursor/`next_cursor` completion rather than offset or short-page assumptions.
- Select credentials server-side in the documented preference order: token, client certificate/key, then compatibility secret. Keep them out of browser code and preserve default `INTERNAL_API_URL` admin port.

## Definition of done

- [ ] A clean build regenerates client/types from the checked-in schema and a stale generated artifact fails a reproducible freshness check.
- [ ] Generated user paths encode a segment; generated status types accept only service wire values; tests exercise both properties without hand-written workaround code.
- [ ] Cursor pagination follows non-null `next_cursor`, including a short page with a next cursor; it no longer sends `offset`.
- [ ] Browser-facing output contains no internal credential, while server-side env selection observes the documented precedence and handles missing credentials safely.
- [ ] `pnpm format:check`, `pnpm lint`, `pnpm typecheck`, and relevant `pnpm test` pass; Rust/schema checks needed by the generator also pass; unrelated failures are recorded but not fixed.
- [ ] Reviewable: a service contract rename breaks generation/build rather than silently changing an operator control.
