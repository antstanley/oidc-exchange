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
| [changes/merged/2026-08-05-verify_admin_ui_session_jwt.md](changes/merged/2026-08-05-verify_admin_ui_session_jwt.md) | Merged | admin UI: verified discovery/JWKS session JWTs, hardened host cookie, security tests and CI gate |
| [changes/2026-06-24-complete_telemetry_exporters.md](changes/2026-06-24-complete_telemetry_exporters.md) | Proposed | service: OTLP/X-Ray exporters + OTEL span layer |
| [changes/2026-08-05-baseline_reference_deployments.md](changes/2026-08-05-baseline_reference_deployments.md) | Proposed | examples, distribution: a named deployment security baseline enforced by a CI conformance gate |
| [changes/merged/2026-08-05-validate_revoke_token_claims.md](changes/merged/2026-08-05-validate_revoke_token_claims.md) | Merged | service: full claim validation on `/revoke`, `sid` claim binding revocation to one session |
| [changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md](changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md) | Merged | service: server-issued nonce, `azp`/`at_hash`, single-use assertions, `[grants] id_token` switch |
| [changes/merged/2026-08-05-bind_grant_type_at_token_endpoint.md](changes/merged/2026-08-05-bind_grant_type_at_token_endpoint.md) | Merged | service: `grant_type` selects the flow, `ExchangeCredential` enum, closed per-grant parameter sets |
| [changes/merged/2026-08-05-rotate_refresh_tokens_with_reuse_detection.md](changes/merged/2026-08-05-rotate_refresh_tokens_with_reuse_detection.md) | Merged | service: rotating refresh tokens, reuse detection, session-family persistence, and owned cleanup |
| [changes/merged/2026-08-05-audit_and_throttle_authentication_failures.md](changes/merged/2026-08-05-audit_and_throttle_authentication_failures.md) | Merged | service: audit, authentication-failure visibility, trusted client provenance, public-route throttling |
| [changes/merged/2026-08-05-fail_closed_across_config_and_adapters.md](changes/merged/2026-08-05-fail_closed_across_config_and_adapters.md) | Merged | repo-wide: closed config domains, HTTPS URLs, verified key metadata, migration probes, and mandatory installer checks |
| [changes/merged/2026-08-05-harden_admin_plane.md](changes/merged/2026-08-05-harden_admin_plane.md) | Merged | service, admin-ui: named operator principal, separate admin listener, bounded admin queries |
| [changes/merged/2026-08-05-runtime_parity_across_interfaces.md](changes/merged/2026-08-05-runtime_parity_across_interfaces.md) | Merged | service, bindings: one owned request normaliser, async total FFI, differential conformance corpus |
| [changes/merged/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md](changes/merged/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md) | Merged | service: `Secret<T>` newtype, bounded upstream bodies, split client/operator error surface, bounded request id |
| [changes/merged/2026-08-05-own_outbound_http_boundary.md](changes/merged/2026-08-05-own_outbound_http_boundary.md) | Merged | service: shared `ProviderTransport` + purpose-filtered `VerificationKeySet`, single-flight JWKS cache, webhook replay binding |
| [changes/merged/2026-07-01-complete_config_loading.md](changes/merged/2026-07-01-complete_config_loading.md) | Merged | service: config overlay merge, env overrides, fail-closed `${VAR}` placeholders, startup validation |
| [changes/merged/2026-08-05-resolve_config_placeholders_all_channels.md](changes/merged/2026-08-05-resolve_config_placeholders_all_channels.md) | Merged | service/bindings: one shared config resolve, fail-closed placeholders, FFI parity, and `config check` |
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
| [changes/merged/2026-08-05-harden_release_supply_chain.md](changes/merged/2026-08-05-harden_release_supply_chain.md) | Merged | distribution: frozen least-privilege releases, provenance, installer verification, advisory/signing-path policy |

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
| [plans/2026-08-05-resolve_config_placeholders_all_channels/plan.md](plans/2026-08-05-resolve_config_placeholders_all_channels/plan.md) | Done | [changes/merged/2026-08-05-resolve_config_placeholders_all_channels.md](changes/merged/2026-08-05-resolve_config_placeholders_all_channels.md) |
| [plans/2026-08-05-fail_closed_across_config_and_adapters/plan.md](plans/2026-08-05-fail_closed_across_config_and_adapters/plan.md) | Done | [changes/merged/2026-08-05-fail_closed_across_config_and_adapters.md](changes/merged/2026-08-05-fail_closed_across_config_and_adapters.md) |
| [plans/2026-08-05-validate_revoke_token_claims/plan.md](plans/2026-08-05-validate_revoke_token_claims/plan.md) | Done | [changes/merged/2026-08-05-validate_revoke_token_claims.md](changes/merged/2026-08-05-validate_revoke_token_claims.md) |
| [plans/2026-08-15-bind_id_token_grant_replay_protection/plan.md](plans/2026-08-15-bind_id_token_grant_replay_protection/plan.md) | Done | [changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md](changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md) |
| [plans/2026-08-05-bind_grant_type_at_token_endpoint/plan.md](plans/2026-08-05-bind_grant_type_at_token_endpoint/plan.md) | Done | [changes/merged/2026-08-05-bind_grant_type_at_token_endpoint.md](changes/merged/2026-08-05-bind_grant_type_at_token_endpoint.md) |
| [plans/2026-08-05-rotate_refresh_tokens_with_reuse_detection/plan.md](plans/2026-08-05-rotate_refresh_tokens_with_reuse_detection/plan.md) | Done | [changes/merged/2026-08-05-rotate_refresh_tokens_with_reuse_detection.md](changes/merged/2026-08-05-rotate_refresh_tokens_with_reuse_detection.md) |
| [plans/2026-08-05-audit_and_throttle_authentication_failures/plan.md](plans/2026-08-05-audit_and_throttle_authentication_failures/plan.md) | Done | [changes/merged/2026-08-05-audit_and_throttle_authentication_failures.md](changes/merged/2026-08-05-audit_and_throttle_authentication_failures.md) |
| [plans/2026-08-05-eliminate_secret_leakage_in_logs_and_spans/plan.md](plans/2026-08-05-eliminate_secret_leakage_in_logs_and_spans/plan.md) | Done | [changes/merged/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md](changes/merged/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md) |
| [plans/2026-08-05-own_outbound_http_boundary/plan.md](plans/2026-08-05-own_outbound_http_boundary/plan.md) | Done | [changes/merged/2026-08-05-own_outbound_http_boundary.md](changes/merged/2026-08-05-own_outbound_http_boundary.md) |
| [plans/2026-08-05-runtime_parity_across_interfaces/plan.md](plans/2026-08-05-runtime_parity_across_interfaces/plan.md) | Done | [changes/merged/2026-08-05-runtime_parity_across_interfaces.md](changes/merged/2026-08-05-runtime_parity_across_interfaces.md) |
| [plans/2026-08-05-harden_admin_plane/plan.md](plans/2026-08-05-harden_admin_plane/plan.md) | Done | [changes/merged/2026-08-05-harden_admin_plane.md](changes/merged/2026-08-05-harden_admin_plane.md) |
| [plans/2026-08-05-harden_release_supply_chain/plan.md](plans/2026-08-05-harden_release_supply_chain/plan.md) | Done | [changes/merged/2026-08-05-harden_release_supply_chain.md](changes/merged/2026-08-05-harden_release_supply_chain.md) |
| [plans/2026-08-15-verify_admin_ui_session_jwt/plan.md](plans/2026-08-15-verify_admin_ui_session_jwt/plan.md) | Done | [changes/merged/2026-08-05-verify_admin_ui_session_jwt.md](changes/merged/2026-08-05-verify_admin_ui_session_jwt.md) |
| [plans/2026-08-05-baseline_reference_deployments/plan.md](plans/2026-08-05-baseline_reference_deployments/plan.md) | Draft | [changes/2026-08-05-baseline_reference_deployments.md](changes/2026-08-05-baseline_reference_deployments.md) |
| [plans/2026-08-05-index_change_specs/plan.md](plans/2026-08-05-index_change_specs/plan.md) | Done | index-only documentation change (this README's change/plan tables); no change spec |

## Conventions

- Per-package specs may reference global specs; global specs never reference per-package ones.
- A page that shadows a global topic opens with a **Read first** pointer and states only the
  per-package deltas.
- Each package's `specs/canonical-types.schema.json` `$ref`s the global schema for shared
  primitives.
