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

## Change specs

Proposed deltas to the canonical spec live under `changes/` as single documents
(`YYYY-MM-DD-snake_case_title.md`) until they ship, then move to `changes/merged/`.

| Change spec | Status | Targets |
|---|---|---|
| [changes/2026-06-24-add_atproto_provider.md](changes/2026-06-24-add_atproto_provider.md) | Proposed | service: Tier 3 atproto provider |
| [changes/2026-06-24-complete_telemetry_exporters.md](changes/2026-06-24-complete_telemetry_exporters.md) | Proposed | service: OTLP/X-Ray exporters + OTEL span layer |
| [changes/merged/2026-06-24-cleanup_stale_references.md](changes/merged/2026-06-24-cleanup_stale_references.md) | Merged | docs/examples: remove stale cloudtrail/atproto references |
| [changes/merged/2026-06-24-add_local_enforcement_gates.md](changes/merged/2026-06-24-add_local_enforcement_gates.md) | Merged | tooling: pre-push hook, Python type checker, limit lints |
| [changes/merged/2026-06-29-migrate_enforcement_to_lefthook_pyright.md](changes/merged/2026-06-29-migrate_enforcement_to_lefthook_pyright.md) | Merged | tooling: lefthook hook, pyright, TS workspace lint/format/typecheck |
| [changes/2026-06-29-add_npm_trusted_publishing.md](changes/2026-06-29-add_npm_trusted_publishing.md) | Proposed | distribution: npm publish job, platform packages, OIDC trusted publishing |
| [changes/2026-06-29-add_pypi_trusted_publishing.md](changes/2026-06-29-add_pypi_trusted_publishing.md) | Proposed | distribution: PyPI publish job, abi3/manylinux wheels, OIDC trusted publishing |

## Plans

Implementation plans decompose a spec (canonical or change) into a dependency-ordered, reviewable
task graph. Each lives under `plans/YYYY-MM-DD-snake_case_title/` as a `plan.md` plus a kanban
board (`backlog/` · `in-progress/` · `blocked/` · `done/`).

| Plan | Status | Source spec |
|---|---|---|
| [plans/2026-06-29-cleanup_stale_references/plan.md](plans/2026-06-29-cleanup_stale_references/plan.md) | Done | docs/examples/config sweep of stale cloudtrail/atproto references + 06-configuration Open-question removal + merge housekeeping |
| [plans/2026-06-29-add_local_enforcement_gates/plan.md](plans/2026-06-29-add_local_enforcement_gates/plan.md) | Done | [changes/merged/2026-06-24-add_local_enforcement_gates.md](changes/merged/2026-06-24-add_local_enforcement_gates.md) |

## Conventions

- Per-package specs may reference global specs; global specs never reference per-package ones.
- A page that shadows a global topic opens with a **Read first** pointer and states only the
  per-package deltas.
- Each package's `specs/canonical-types.schema.json` `$ref`s the global schema for shared
  primitives.
