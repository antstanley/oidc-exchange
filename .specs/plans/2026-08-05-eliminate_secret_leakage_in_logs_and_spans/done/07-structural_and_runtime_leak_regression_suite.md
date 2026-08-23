# Task 07 — Prove structural and runtime non-leakage

**Status:** Done · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §Implementation notes step 7](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`07-telemetry-and-audit.md` Telemetry hygiene](../../../service/specs/07-telemetry-and-audit.md)
**Depends on:** 01, 02, 05, 06
**Produces:** compile-fail proof that `Secret<T>` cannot be formatted and a cross-boundary capture corpus proving sensitive sentinels never reach logs, span fields, or public error bodies.
**Pointers:** core/adapters/providers/server test manifests; session repository implementations; request-ID module tests; provider wiremock tests; server error/routes tests.

## Steps

- [x] Add `trybuild` as a core dev-dependency and UI cases that fail for `tracing::info!(?secret)`, `%secret`, `format!("{secret}")`, and default `#[instrument]` argument capture; commit expected compiler diagnostics as test fixtures.
- [x] Add an explicit capturing subscriber with span-close emission and percent-decoded matching helpers; drive store, refresh, revoke, and upstream-error paths using distinct hash/token/config/assertion sentinels.
- [x] Exercise every `SessionRepository` backend (Dynamo, LMDB, Postgres, SQLite, Valkey, and mocks where applicable) and assert neither event nor close-span output contains secret/hash/provenance sentinels; assert permitted field names remain schema-compatible.
- [x] Consolidate request-ID boundary/leak assertions and public error-oracle checks from tasks 02 and 06 into end-to-end request paths.
- [x] Run and record actual Rust quality gates: `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace`; fix failures before task completion.

## Task-specific definition of done

- [x] Compile-fail tests prove the type-system control rather than merely redaction convention.
- [x] Runtime corpus covers all required stores plus refresh/revoke/upstream error paths and detects plain or percent-encoded sentinels with close spans enabled.
- [x] Request-ID limits and generic error bodies are exercised on an HTTP path, including positive and negative boundary cases.
- [x] Full Rust gates pass with actual output captured in the PR/task discussion, not a done certificate.
- [x] No certificate file is created; test output is the completion evidence.

**Evidence:** commits `test(core)` (trybuild compile-fail suite), `test(test-utils)` + `fix(adapters)`/`feat(providers)`/`feat(core)` corpus suites, `test(server)` oracle alignment, `test(server)` request-id consolidation — see `jj log` on this branch. Structural: `crates/core/tests/ui/` pins six compile-fail cases (`?secret`, `%secret`, `format!`/`to_string`, default `#[instrument]` capture, tracing debug/display field) with committed `.stderr` diagnostics. Runtime corpora under a capturing subscriber with `FmtSpan::NEW | CLOSE` and plain-plus-percent-decoded absence assertions: `crates/adapters/tests/session_span_leak_corpus.rs` drives the full session lifecycle across LMDB, SQLite, mock, Valkey, Postgres, and DynamoDB backends (the last three live in the integration tier), asserting hash/provenance sentinels never render while permitted fields stay schema-compatible via declared-field capture; `crates/core/tests/service_leak_corpus.rs` covers store/refresh/revoke plus configured-secret and audit-fallback paths; `crates/providers/tests/upstream_error_leak_corpus.rs` covers exchange and both revocation upstream-error paths with echoed-form sentinels; `crates/server/tests/http_leak_regression.rs` drives token lifecycle over real HTTP against LMDB with provenance headers; `crates/server/tests/request_leak_oracle.rs` proves unknown-kid non-echo with operator-log retention under the request span, signature/expiry/audience body indistinguishability, hostile-upstream echo redaction, and consolidated request-ID boundaries (at-limit reuse, silent UUIDv4 replacement above limit and at 64 KiB). Gates: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo nextest run --workspace` 471 passed / 0 failed / 31 skipped; integration tier `--run-ignored only`: 30 passed / 1 failed — the failure is `kms::tests::test_kms_sign_integration`, environmental only (LocalStack :4566 unreachable), all store/span/provider leak-corpus tests passed against live Valkey :6379, Postgres :5432, and DynamoDB Local :8000.
