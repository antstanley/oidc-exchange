/**
 * Deterministic stand-in for SvelteKit's `$env/dynamic/private`.
 *
 * `src/lib/api.ts` reads `INTERNAL_API_URL` once at module load; keeping every
 * key unset here pins tests to the documented default base URL no matter what
 * the invoking shell exports.
 */
export const env: Record<string, string | undefined> = {};
