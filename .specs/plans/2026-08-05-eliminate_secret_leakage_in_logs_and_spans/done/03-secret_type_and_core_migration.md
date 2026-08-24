# Task 03 — Add `Secret<T>` and migrate core contracts

**Status:** Done · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §Type changes](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#type-changes); [§Implementation notes step 6](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`01-domain-model.md` Session](../../../service/specs/01-domain-model.md), [`02-ports-and-adapters.md` SessionRepository](../../../service/specs/02-ports-and-adapters.md), [`06-configuration.md`](../../../service/specs/06-configuration.md)
**Depends on:** —
**Produces:** core-owned serde-transparent `Secret<T>` with deliberate exposure APIs, constant-time `Secret<String>` equality, and compiler-enforced migration of the enumerated credential-derived core/config/session/repository values.
**Pointers:** `crates/core/src/lib.rs`, new `secret.rs`, `error.rs`, `config.rs`, `domain/session.rs`, `domain/token.rs`, `domain/provider.rs`, `ports/repository.rs`, `service/exchange.rs`; every SessionRepository adapter and test fixture.

## Steps

- [x] Add and re-export `Secret<T>` with `new`, `expose`, and `into_inner`; derive only `Clone`, `Serialize`, and `Deserialize` with `#[serde(transparent)]`, never `Debug`/`Display`/generic `PartialEq`.
- [x] Add `subtle` to core and implement constant-time `PartialEq` only for `Secret<String>`; cover equal and unequal values without exposing values in assertions/logging.
- [x] Convert `Session.refresh_token_hash`, refresh-token issuance/`TokenResponse.refresh_token`, `WebhookConfig.secret`, `InternalApiConfig.shared_secret`, and `OidcProviderConfig.client_secret`; preserve serde storage/wire shapes and existing redacting enclosing `Debug` behavior.
- [x] Replace `Session` derived `Debug` with a manual implementation that redacts only `refresh_token_hash` and preserves the non-sensitive fields.
- [x] Change `SessionRepository` lookup/revoke signatures to `&Secret<String>` and migrate Dynamo, LMDB, Postgres, SQLite, Valkey, mocks, callers, store-key construction, and tests to use `expose()` only at deliberate storage/constant-time comparison boundaries.
- [x] Convert adapter OIDC `client_secret` storage to `Option<Secret<String>>`; leave Apple assertion conversion to task 05, which owns its producer and provider call sites.

## Task-specific definition of done

- [x] Every value in the source table except upstream response bodies and Apple’s generated assertion is `Secret<String>`/`Option<Secret<String>>` at the stated boundary.
- [x] Session JSON round trips and token response serialization remain string-identical; no schema or migration changes occur.
- [x] All repository implementations compile under `&Secret<String>` and their instrumentation uses explicit skips.
- [x] `Session` debug output contains `<redacted>` and excludes a hash sentinel.
- [x] No certificate file is created; test output and compile checks are the completion evidence.

**Evidence:** full workspace — 408 nextest tests pass (`cargo nextest run --workspace --no-fail-fast`), `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean. New tests: constant-time equality (equal/unequal/symmetric length-mismatch/empty), serde transparency for `Secret<String>` and for Session/TokenResponse/OidcProviderConfig shapes, `Session` Debug redaction, config Debug redaction, TokenResponse wire-shape identity including absent-refresh_token skip. Dynamo/LMDB/Postgres/SQLite/Valkey adapters, mocks (test-utils), core services, server bootstrap/auth, and all fixtures migrated; lookup/revoke instruments now name the argument in `skip(self, token_hash)` explicitly. `trybuild` compile-fail proof is deliberately deferred to task 07 per the plan.
