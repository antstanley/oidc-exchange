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
| [changes/merged/2026-07-01-complete_config_loading.md](changes/merged/2026-07-01-complete_config_loading.md) | Merged | service: config overlay merge, env overrides, fail-closed `${VAR}` placeholders, startup validation |
| [changes/merged/2026-07-01-fix_kms_ecdsa_and_jwk_encoding.md](changes/merged/2026-07-01-fix_kms_ecdsa_and_jwk_encoding.md) | Merged | service: KMS ES* DER→raw JWS signatures, RFC 7518 JWK `n`/`e`, ES512 JWK |
| [changes/merged/2026-07-01-valkey_session_store_conformance.md](changes/merged/2026-07-01-valkey_session_store_conformance.md) | Merged | service: Valkey session count, atomic TTL'd writes, expired-index cleanup |
| [changes/merged/2026-07-01-release_gil_in_python_binding.md](changes/merged/2026-07-01-release_gil_in_python_binding.md) | Merged | bindings: release the GIL around the blocking FFI call |
| [changes/merged/2026-07-01-require_iss_aud_in_token_validation.md](changes/merged/2026-07-01-require_iss_aud_in_token_validation.md) | Merged | service: require `iss`/`aud` presence, `nbf`, JWK alg inference, Apple `email_verified` coercion |
| [changes/merged/2026-07-01-harden_outbound_provider_http.md](changes/merged/2026-07-01-harden_outbound_provider_http.md) | Merged | service: shared HTTP client + timeouts, JWKS status/rotation handling, token-endpoint and discovery error checks |
| [changes/merged/2026-07-01-fix_user_creation_race_and_dynamo_integrity.md](changes/merged/2026-07-01-fix_user_creation_race_and_dynamo_integrity.md) | Merged | service: `(provider, external_id)` uniqueness on DynamoDB, JIT-create race, batch-write retry, update concurrency |
| [changes/merged/2026-07-01-run_postgres_migrations_on_startup.md](changes/merged/2026-07-01-run_postgres_migrations_on_startup.md) | Merged | service: execute Postgres `MIGRATIONS` in `create_pool` |
| [changes/merged/2026-07-01-enforce_user_lifecycle_transitions.md](changes/merged/2026-07-01-enforce_user_lifecycle_transitions.md) | Merged | service: lifecycle enforcement in admin updates, session revocation on delete, 4xx on unknown id |
| [changes/merged/2026-07-01-wire_audit_event_emission.md](changes/merged/2026-07-01-wire_audit_event_emission.md) | Merged | service: `emit_audit` call sites in every flow, client-context plumbing, stdout/SQS audit hardening |
| [changes/merged/2026-07-01-webhook_user_sync_conformance.md](changes/merged/2026-07-01-webhook_user_sync_conformance.md) | Merged | service: JIT `user.created` webhook, 2xx-only delivery, redirect and backoff limits |
| [changes/merged/2026-07-01-server_error_handling_and_shutdown.md](changes/merged/2026-07-01-server_error_handling_and_shutdown.md) | Merged | service: revoke 503 on backend failure, `server_error` logging, per-request span, graceful shutdown |
| [changes/merged/2026-07-01-implement_lambda_runtime.md](changes/merged/2026-07-01-implement_lambda_runtime.md) | Merged | service: serve the axum router via `lambda_http` in Lambda mode |
| [changes/merged/2026-06-24-cleanup_stale_references.md](changes/merged/2026-06-24-cleanup_stale_references.md) | Merged | docs/examples: remove stale cloudtrail/atproto references |
| [changes/merged/2026-06-24-add_local_enforcement_gates.md](changes/merged/2026-06-24-add_local_enforcement_gates.md) | Merged | tooling: pre-push hook, Python type checker, limit lints |
| [changes/merged/2026-06-29-migrate_enforcement_to_lefthook_pyright.md](changes/merged/2026-06-29-migrate_enforcement_to_lefthook_pyright.md) | Merged | tooling: lefthook hook, pyright, TS workspace lint/format/typecheck |
| [changes/merged/2026-06-29-add_npm_trusted_publishing.md](changes/merged/2026-06-29-add_npm_trusted_publishing.md) | Merged | distribution: npm publish job, platform packages, OIDC trusted publishing |
| [changes/merged/2026-06-29-add_pypi_trusted_publishing.md](changes/merged/2026-06-29-add_pypi_trusted_publishing.md) | Merged | distribution: PyPI publish job, abi3/manylinux wheels, OIDC trusted publishing |

## Plans

Implementation plans decompose a spec (canonical or change) into a dependency-ordered, reviewable
task graph. Each lives under `plans/YYYY-MM-DD-snake_case_title/` as a `plan.md` plus a kanban
board (`backlog/` · `in-progress/` · `blocked/` · `done/`).

| Plan | Status | Source spec |
|---|---|---|
| [plans/2026-06-29-cleanup_stale_references/plan.md](plans/2026-06-29-cleanup_stale_references/plan.md) | Done | docs/examples/config sweep of stale cloudtrail/atproto references + 06-configuration Open-question removal + merge housekeeping |
| [plans/2026-06-29-add_local_enforcement_gates/plan.md](plans/2026-06-29-add_local_enforcement_gates/plan.md) | Done | [changes/merged/2026-06-24-add_local_enforcement_gates.md](changes/merged/2026-06-24-add_local_enforcement_gates.md) |
| [plans/2026-06-30-add_npm_trusted_publishing/plan.md](plans/2026-06-30-add_npm_trusted_publishing/plan.md) | Done | [changes/merged/2026-06-29-add_npm_trusted_publishing.md](changes/merged/2026-06-29-add_npm_trusted_publishing.md) |
| [plans/2026-06-30-add_pypi_trusted_publishing/plan.md](plans/2026-06-30-add_pypi_trusted_publishing/plan.md) | Done | [changes/merged/2026-06-29-add_pypi_trusted_publishing.md](changes/merged/2026-06-29-add_pypi_trusted_publishing.md) |
| [plans/2026-07-02-complete_config_loading/plan.md](plans/2026-07-02-complete_config_loading/plan.md) | Done | [changes/merged/2026-07-01-complete_config_loading.md](changes/merged/2026-07-01-complete_config_loading.md) |
| [plans/2026-07-02-fix_kms_ecdsa_and_jwk_encoding/plan.md](plans/2026-07-02-fix_kms_ecdsa_and_jwk_encoding/plan.md) | Done | [changes/merged/2026-07-01-fix_kms_ecdsa_and_jwk_encoding.md](changes/merged/2026-07-01-fix_kms_ecdsa_and_jwk_encoding.md) |
| [plans/2026-07-02-valkey_session_store_conformance/plan.md](plans/2026-07-02-valkey_session_store_conformance/plan.md) | Done | [changes/merged/2026-07-01-valkey_session_store_conformance.md](changes/merged/2026-07-01-valkey_session_store_conformance.md) |
| [plans/2026-07-02-release_gil_in_python_binding/plan.md](plans/2026-07-02-release_gil_in_python_binding/plan.md) | Done | [changes/merged/2026-07-01-release_gil_in_python_binding.md](changes/merged/2026-07-01-release_gil_in_python_binding.md) |
| [plans/2026-07-02-require_iss_aud_in_token_validation/plan.md](plans/2026-07-02-require_iss_aud_in_token_validation/plan.md) | Done | [changes/merged/2026-07-01-require_iss_aud_in_token_validation.md](changes/merged/2026-07-01-require_iss_aud_in_token_validation.md) |
| [plans/2026-07-02-harden_outbound_provider_http/plan.md](plans/2026-07-02-harden_outbound_provider_http/plan.md) | Done | [changes/merged/2026-07-01-harden_outbound_provider_http.md](changes/merged/2026-07-01-harden_outbound_provider_http.md) |
| [plans/2026-07-02-fix_user_creation_race_and_dynamo_integrity/plan.md](plans/2026-07-02-fix_user_creation_race_and_dynamo_integrity/plan.md) | Done | [changes/merged/2026-07-01-fix_user_creation_race_and_dynamo_integrity.md](changes/merged/2026-07-01-fix_user_creation_race_and_dynamo_integrity.md) |
| [plans/2026-07-02-run_postgres_migrations_on_startup/plan.md](plans/2026-07-02-run_postgres_migrations_on_startup/plan.md) | Done | [changes/merged/2026-07-01-run_postgres_migrations_on_startup.md](changes/merged/2026-07-01-run_postgres_migrations_on_startup.md) |
| [plans/2026-07-02-enforce_user_lifecycle_transitions/plan.md](plans/2026-07-02-enforce_user_lifecycle_transitions/plan.md) | Done | [changes/merged/2026-07-01-enforce_user_lifecycle_transitions.md](changes/merged/2026-07-01-enforce_user_lifecycle_transitions.md) |
| [plans/2026-07-02-wire_audit_event_emission/plan.md](plans/2026-07-02-wire_audit_event_emission/plan.md) | Done | [changes/merged/2026-07-01-wire_audit_event_emission.md](changes/merged/2026-07-01-wire_audit_event_emission.md) |
| [plans/2026-07-02-webhook_user_sync_conformance/plan.md](plans/2026-07-02-webhook_user_sync_conformance/plan.md) | Done | [changes/merged/2026-07-01-webhook_user_sync_conformance.md](changes/merged/2026-07-01-webhook_user_sync_conformance.md) |
| [plans/2026-07-02-server_error_handling_and_shutdown/plan.md](plans/2026-07-02-server_error_handling_and_shutdown/plan.md) | Done | [changes/merged/2026-07-01-server_error_handling_and_shutdown.md](changes/merged/2026-07-01-server_error_handling_and_shutdown.md) |
| [plans/2026-07-02-implement_lambda_runtime/plan.md](plans/2026-07-02-implement_lambda_runtime/plan.md) | Done | [changes/merged/2026-07-01-implement_lambda_runtime.md](changes/merged/2026-07-01-implement_lambda_runtime.md) |
| [plans/2026-08-05-own_outbound_http_boundary/plan.md](plans/2026-08-05-own_outbound_http_boundary/plan.md) | Review | [changes/2026-08-05-own_outbound_http_boundary.md](changes/2026-08-05-own_outbound_http_boundary.md) |

## Conventions

- Per-package specs may reference global specs; global specs never reference per-package ones.
- A page that shadows a global topic opens with a **Read first** pointer and states only the
  per-package deltas.
- Each package's `specs/canonical-types.schema.json` `$ref`s the global schema for shared
  primitives.
