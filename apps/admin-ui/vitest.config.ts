import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

/**
 * Standalone Vitest config for the admin console's client tests.
 *
 * Why not reuse `vite.config.ts`: the SvelteKit plugin binds the build to
 * `.svelte-kit` sync state, which would make plain unit tests depend on
 * generated artifacts. The server environment module (`#lib/server/env`) is aliased to a deterministic stub below, so
 * client tests run anywhere Node runs.
 */
export default defineConfig({
  resolve: {
    alias: {
      "#lib/server/env": fileURLToPath(new URL("./tests/env-stub.ts", import.meta.url)),
    },
  },
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
  },
});
