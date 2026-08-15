# Task 07 — Prove structural and runtime non-leakage

**Status:** Backlog · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §Implementation notes step 7](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`07-telemetry-and-audit.md` Telemetry hygiene](../../../service/specs/07-telemetry-and-audit.md)
**Depends on:** 01, 02, 05, 06
**Produces:** compile-fail proof that `Secret<T>` cannot be formatted and a cross-boundary capture corpus proving sensitive sentinels never reach logs, span fields, or public error bodies.
**Pointers:** core/adapters/providers/server test manifests; session repository implementations; request-ID module tests; provider wiremock tests; server error/routes tests.

## Steps

- [ ] Add `trybuild` as a core dev-dependency and UI cases that fail for `tracing::info!(?secret)`, `%secret`, `format!("{secret}")`, and default `#[instrument]` argument capture; commit expected compiler diagnostics as test fixtures.
- [ ] Add an explicit capturing subscriber with span-close emission and percent-decoded matching helpers; drive store, refresh, revoke, and upstream-error paths using distinct hash/token/config/assertion sentinels.
- [ ] Exercise every `SessionRepository` backend (Dynamo, LMDB, Postgres, SQLite, Valkey, and mocks where applicable) and assert neither event nor close-span output contains secret/hash/provenance sentinels; assert permitted field names remain schema-compatible.
- [ ] Consolidate request-ID boundary/leak assertions and public error-oracle checks from tasks 02 and 06 into end-to-end request paths.
- [ ] Run and record actual Rust quality gates: `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace`; fix failures before task completion.

## Task-specific definition of done

- [ ] Compile-fail tests prove the type-system control rather than merely redaction convention.
- [ ] Runtime corpus covers all required stores plus refresh/revoke/upstream error paths and detects plain or percent-encoded sentinels with close spans enabled.
- [ ] Request-ID limits and generic error bodies are exercised on an HTTP path, including positive and negative boundary cases.
- [ ] Full Rust gates pass with actual output captured in the PR/task discussion, not a done certificate.
- [ ] No certificate file is created; test output is the completion evidence.
