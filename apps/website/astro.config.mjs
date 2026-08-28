// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import svelte from "@astrojs/svelte";
import markdownApi from "./integrations/markdown-api.js";
import mermaid from "astro-mermaid";

// https://astro.build/config
export default defineConfig({
  site: "https://oidc-exchange.iamstan.dev",
  redirects: {
    "/": "/getting-started/introduction/",
  },
  integrations: [
    // Must precede starlight so its rehype plugin transforms mermaid code blocks.
    mermaid({ autoTheme: true }),
    starlight({
      title: "oidc-exchange",
      description:
        "A Rust service that validates OIDC tokens and exchanges them for self-issued JWTs.",
      customCss: ["./src/styles/mermaid.css"],
      social: [
        { icon: "github", label: "GitHub", href: "https://github.com/antstanley/oidc-exchange" },
      ],
      editLink: {
        baseUrl:
          "https://github.com/antstanley/oidc-exchange/edit/main/apps/website/src/content/docs/",
      },
      sidebar: [
        {
          label: "Getting Started",
          items: [
            { slug: "getting-started/introduction" },
            { slug: "getting-started/why-oidc-exchange" },
            { slug: "getting-started/installation" },
            { slug: "getting-started/quick-start" },
          ],
        },
        {
          label: "Guides",
          items: [
            { slug: "guides/configuration" },
            { slug: "guides/providers" },
            { slug: "guides/api-reference" },
            { slug: "guides/client-integration" },
            { slug: "guides/nodejs" },
            { slug: "guides/python" },
            { slug: "guides/docker" },
          ],
        },
        {
          label: "Deployment",
          items: [
            { slug: "deployment/overview" },
            {
              label: "AWS",
              collapsed: false,
              items: [{ slug: "deployment/aws-lambda" }, { slug: "deployment/ecs-fargate" }],
            },
            {
              label: "Linux Server",
              collapsed: false,
              items: [
                { slug: "deployment/linux-server" },
                { slug: "deployment/linux-postgres" },
                { slug: "deployment/linux-sqlite" },
              ],
            },
            { slug: "deployment/container" },
          ],
        },
        {
          label: "Architecture",
          items: [{ slug: "architecture/overview" }, { slug: "architecture/adapters" }],
        },
        {
          label: "Contributing",
          items: [{ slug: "contributing/development" }],
        },
      ],
    }),
    svelte(),
    markdownApi(),
  ],
});
