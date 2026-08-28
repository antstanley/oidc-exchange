# Documentation Website — Overview

**Status:** Implemented · **Date:** 2026-06-24 · **Owner:** Ant Stanley · **Scope:** apps/website

The public documentation site at `oidc-exchange.iamstan.dev`, built with Astro and Starlight. It
renders the canonical prose docs in `src/content/docs/` and additionally serves their raw markdown for
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
- `src/content/docs/` — the canonical documentation content, the Starlight collection itself.
  `content.config.ts` defines the collection schema.
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

- `src/content/docs/` is the canonical documentation source; it is the single copy of the
  prose docs in the repository.

### Decisions

- *Docs live in the content collection.* **The prose docs are `src/content/docs/` directly.**
  They were moved out of a repo-root `/docs` reached by a committed symlink, because
  blogwright's deploy source-zipper cannot read a symlinked directory. A real directory keeps
  one source of truth with no symlink.
- *Markdown twin + content negotiation.* **Every page is also served as raw markdown.** Lets
  agents and tools consume the docs without HTML parsing.

### Open questions

- (None at this stage.)
