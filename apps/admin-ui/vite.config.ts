import tailwindcss from "@tailwindcss/vite";
import adapter from "@sveltejs/adapter-node";
import { sveltekit } from "@sveltejs/kit/vite";
import { relative, sep } from "node:path";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [
    tailwindcss(),
    sveltekit({
      adapter: adapter(),
      compilerOptions: {
        runes: ({ filename }) => {
          const relativePath = relative(import.meta.dirname, filename);
          const pathSegments = relativePath.toLowerCase().split(sep);
          const isExternalLibrary = pathSegments.includes("node_modules");
          return isExternalLibrary ? undefined : true;
        },
      },
    }),
  ],
});
