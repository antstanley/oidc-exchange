# 01 · Tactical console path/status fixes

**Status:** Done  
**Implements:** [source spec](../../../changes/2026-08-05-harden_admin_plane.md) §"Implementation notes" step 1; [admin-ui overview](../../../admin-ui/specs/00-overview.md) target  
**Depends on:** — (external integration gate: `2026-08-05-verify_admin_ui_session_jwt` owns session/login changes)  
**Produces:** The existing server-only admin client encodes every user-id path segment; UI types and status controls use `active | suspended | deleted`.

**Pointers:** `apps/admin-ui/src/lib/api.ts`; `apps/admin-ui/src/lib/types.ts`; `apps/admin-ui/src/routes/(app)/+page.svelte`; `apps/admin-ui/src/routes/(app)/users/+page.svelte`; `apps/admin-ui/src/routes/(app)/users/[id]/+page.svelte`; sibling UI changes in `.specs/changes/2026-08-05-verify_admin_ui_session_jwt.md`.

## Work

- Encode the `id` segment inside each existing user-specific API helper (`getUser`, `updateUser`, `deleteUser`, claims get/set/merge/clear); do not scatter encoding among page-server call sites.
- Replace the title-cased status union and every display comparison/select value with the service wire representation.
- Add focused client/UI tests covering exact hostile ids `x%2f..%2fstats` and `%2e%2e%2fstats`: requests must address only a literal `/internal/users/<encoded-id>` path, never `/internal/stats` or another route.
- Integrate rather than duplicate the session-JWT sibling's authentication-model changes; this task owns no login/session verification.

## Definition of done

- [x] Every user-id helper encodes exactly one path segment before composing its request path; query parameters remain separately encoded.
- [x] Status badges and edit submission use only `active`, `suspended`, and `deleted`; TypeScript catches any title-cased value.
- [x] Positive and negative tests prove both hostile payloads cannot redirect credentialed traffic, while a normal id still reaches its intended endpoint.
- [x] `pnpm format:check`, `pnpm lint`, `pnpm typecheck`, and relevant `pnpm test` pass; unrelated failures are recorded but not repaired.
- [x] Reviewable: one client boundary owns path encoding and the UI’s status spelling matches `UserStatus` serialization.
