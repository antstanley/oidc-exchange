# oidc-exchange specifications

Canonical design specs for `oidc-exchange`. Each page describes what exists in the current
branch; aspirational and deferred work lives only in the closing
`Assumptions and open questions` block of each page (or in a change spec under `changes/`,
once one is drafted).

Start with the [architecture principles](architecture-principles.md), then the service
[overview](service/specs/00-overview.md).

## Global specs (repo-wide)

| Spec | Purpose |
|---|---|
| [architecture-principles.md](architecture-principles.md) | Hexagonal layering, monorepo layout, dependency rules, runtime modes, stack baseline |
| [development-guidelines.md](development-guidelines.md) | Toolchain, Tiger Style discipline, per-language conventions, testing, version control, definition of done |
| [canonical-types.schema.json](canonical-types.schema.json) | Repo-wide shared types (ids, timestamps, the HTTP request/response envelope, OAuth error body) |

## Per-package specs

| Package | Specs | Covers |
|---|---|---|
| Service (Rust) | [service/](service/README.md) | the OIDC exchange service: domain, ports/adapters, flows, HTTP API, providers, config, telemetry/audit, persistence |
| Bindings & distribution | [bindings/](bindings/README.md) | the FFI core, Node/Python/Lambda bindings, install/Docker/release |
| Admin UI | [admin-ui/](admin-ui/README.md) | the SvelteKit admin console |
| Website | [website/](website/README.md) | the Astro/Starlight documentation site |

## Conventions

- Per-package specs may reference global specs; global specs never reference per-package ones.
- A page that shadows a global topic opens with a **Read first** pointer and states only the
  per-package deltas.
- Each package's `specs/canonical-types.schema.json` `$ref`s the global schema for shared
  primitives.
