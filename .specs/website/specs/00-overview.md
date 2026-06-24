# Documentation Website — Overview

**Status:** Implemented · **Date:** 2026-06-24 · **Owner:** Ant Stanley · **Scope:** apps/website

The public documentation site at `oidc-exchange.dev`, built with Astro and Starlight. It
renders the canonical prose docs in `/docs` and additionally serves their raw markdown for
agent consumption.

> **Read first:** [.specs/architecture-principles.md](../../architecture-principles.md). The
> website is a static documentation app; it does not call the service.

## Responsibilities

- Render the `/docs` content tree (getting-started, guides, deployment, architecture,
  contributing) as a Starlight documentation site.
- Serve a stripped-frontmatter `.md` twin of every page, and honour `Accept: text/markdown`
  content negotiation, so agents can fetch raw markdown.

## Structure

- `astro.config.mjs` — Starlight + Svelte integrations, site metadata, sidebar sections, a
  redirect from `/` to `/getting-started/introduction/`, and the custom `markdownApi()`
  integration.
- `src/content/docs/` — a symlink to the repository's `/docs`, so the canonical docs are the
  single source. `content.config.ts` defines the collection schema.
- `integrations/markdown-api.js` — emits the `.md` twins at build time and injects markdown
  content negotiation in dev/SSR.
- `src/middleware-markdown.ts` — strips frontmatter and negotiates markdown vs HTML.
- `src/components/TokenFlow.svelte` — a token-flow diagram component.
- `serve.mjs` — a small Node static server (port 4321, `PORT` overridable) that serves the
  built site, `/[path].md` for raw markdown, and `Accept: text/markdown` on the HTML URL.

## Build

`npm run build` produces `dist/` with HTML plus the `.md` twins; `npm run serve` runs
`serve.mjs`.

## Assumptions and open questions

### Assumptions

- `/docs` is the canonical documentation source; the website never holds its own copy (the
  `src/content/docs` symlink enforces this).

### Decisions

- *Docs symlinked, not copied.* **`src/content/docs` symlinks `/docs`.** One source of truth;
  editing `/docs` updates the site with no sync step.
- *Markdown twin + content negotiation.* **Every page is also served as raw markdown.** Lets
  agents and tools consume the docs without HTML parsing.

### Open questions

- (None at this stage.)
