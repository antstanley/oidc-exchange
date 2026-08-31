# Task 03 — StoreError joins the audit vocabulary and datamodel schema

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-store_error_audit_vocabulary-certificate.md](03-store_error_audit_vocabulary-certificate.md)

**Implements:** [Change spec §The delta → G2](../../../changes/2026-08-31-embedded_telemetry_and_store_fault_audit.md#g2--record-the-exchange-flows-infrastructure-store-fault), first bullet (the enum variants, `SecurityEvent` deliberately unchanged) and fourth bullet (`schemas/datamodel.schema.json` plus the mirror-test builders). Makes true the `AuditEventType` variant list of [01-domain-model.md §AuditEvent](../../../service/specs/01-domain-model.md); the sidecar `canonical-types.schema.json` fold-in stays with the change spec's Merge plan.
**Depends on:** —
**Produces:** The audit vocabulary can name an infrastructure store fault: `AuditEventType::StoreError` and `AuditFailure::StoreError` (both serialized `store_error`) exist, `schemas/datamodel.schema.json` carries `store_error` in both its `event_type` and `outcome.reason` enums, and the mirror test proves code and schema agree. The closed `SecurityEvent` set is untouched.
**Pointers:** `crates/core/src/domain/audit.rs:56-81` (`AuditEventType` — add the variant with a doc comment stating the operational, non-security classification); `crates/core/src/domain/audit.rs:360-376` (`AuditFailure` — add the variant); `crates/core/src/domain/audit.rs:165-190` (`SecurityEvent` — deliberately unchanged); `schemas/datamodel.schema.json:69` (`event_type` enum) and `:85` (`outcome.reason` enum); `crates/core/tests/datamodel_schema_mirror.rs:26` (`all_event_types`) and `:61` (`all_failures`) — the exhaustive builders that make the schema edit compile-enforced.

## Steps

- [ ] Add `StoreError` to `AuditEventType` (serde `snake_case` renders it `store_error`), with a doc comment recording why it is an operational event and never a `SecurityEvent`.
- [ ] Add `StoreError` to `AuditFailure` so an outcome can name the reason.
- [ ] Add `store_error` to the `event_type` enum and the `outcome.reason` enum in `schemas/datamodel.schema.json`.
- [ ] Add the new variants to the mirror test's exhaustive `all_event_types` and `all_failures` builders — the compiler forces this once the enums grow; the mirror equality then forces the schema edit. Append `StoreError`/`store_error` at the END of both Rust enums and both schema arrays — the mirror asserts equality exactly, in declaration order (`datamodel_schema_mirror.rs:99-103`).
- [ ] Leave `SecurityEvent` and every `into_audit_event` mapping unchanged — the closed security-outcome set this change deliberately does not extend.

## Definition of done

- [ ] `cargo nextest run -p oidc-exchange-core -E 'binary(datamodel_schema_mirror)'` passes with `store_error` present in both enum mirrors — the schema and the code cannot drift.
- [ ] `AuditEventType::StoreError` and `AuditFailure::StoreError` serialize to `store_error` (covered by the mirror test's serde rendering; no bespoke serializer added).
- [ ] Negative space: `SecurityEvent` gains no store-fault variant and no existing enum value or serialized name changes — the existing audit, exchange, refresh, and revoke suites pass unmodified.
- [ ] Meets the repo definition of done (`cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [ ] Reviewable: run the mirror test and inspect the `schemas/datamodel.schema.json` diff — exactly two enum entries added, nothing else.
