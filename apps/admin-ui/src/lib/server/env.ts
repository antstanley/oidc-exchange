/**
 * Server-side runtime environment.
 *
 * SvelteKit 3 removed `$env/dynamic/private`. The admin console runs on
 * adapter-node and reads plain runtime variables (including names resolved
 * dynamically), so this re-exports `process.env` from a server-only module.
 * Tests alias this module to a deterministic stub (see `vitest.config.ts`).
 */
export const env: Record<string, string | undefined> = process.env;
