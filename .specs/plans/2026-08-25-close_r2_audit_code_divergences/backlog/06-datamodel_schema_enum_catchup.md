# Task 06 — Catch `schemas/datamodel.schema.json` up with the code

**Plan:** [plan.md](../plan.md) · **Certificate:** [06-datamodel_schema_enum_catchup-certificate.md](06-datamodel_schema_enum_catchup-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §`datamodel.schema.json` (cross-adapter source of truth); change spec §The delta → S6-code
**Depends on:** —
**Produces:** `schemas/datamodel.schema.json`'s `AuditEvent` mirrors the shipped `AuditEventType` (18) and `AuditFailure` (9) variants plus an optional `operator`, guarded by a mirror test so the next enum addition fails a test instead of drifting.
**Pointers:** `schemas/datamodel.schema.json:62-85` (`AuditEvent`, `event_type` at `:69`, `outcome.reason` at `:81`, `definitions` at `:4`); `crates/core/src/domain/audit.rs:56-81` (`AuditEventType`), `:344-360` (`AuditFailure`); `schemas/internal-api.schema.json:114-126` (operator shape)

## Steps

- [ ] Extend the `event_type` enum (`:69`) with `refresh_token_reuse`, `missing_credential`, `invalid_credential`, `not_configured` — the 18 `AuditEventType` variants.
- [ ] Extend the `outcome.reason` enum (`:81`) with the same four values (plus the existing `null`) — the 9 `AuditFailure` variants.
- [ ] Add optional `operator` to `AuditEvent.properties`, with `OperatorPrincipal` (`{id: non-empty string, mechanism}`, both required) and `OperatorAuthMechanism` (`enum: ["mtls","operator_token","shared_secret"]`) definitions under this file's `definitions` key; leave `required` unchanged.

## Definition of done

- [ ] A mirror test (no new dependencies) reads the schema and asserts its `event_type` and `outcome.reason` enum arrays equal the serde-rendered variant lists of `AuditEventType` and `AuditFailure`.
- [ ] Negative-space: the test fails if a variant is added to either enum without updating the schema (verified by a scratch variant or an inline assertion of exact-set equality, not subset).
- [ ] Meets the repo definition of done (test, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the mirror test and confirms it passes against the updated schema and would fail on an un-mirrored enum addition.
