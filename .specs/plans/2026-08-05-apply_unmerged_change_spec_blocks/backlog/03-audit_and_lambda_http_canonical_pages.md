# 03 · Audit and Lambda HTTP canonical pages

**Plan:** [plan.md](../plan.md) · **Status:** Backlog · **Certificate:** intentionally omitted (planning backlog only)

**Implements:** [2026-07-01-wire_audit_event_emission.md](../../../changes/merged/2026-07-01-wire_audit_event_emission.md) and [2026-07-01-implement_lambda_runtime.md](../../../changes/merged/2026-07-01-implement_lambda_runtime.md) — their `Proposed changes` blocks targeting [01-domain-model.md](../../../service/specs/01-domain-model.md), [03-service-flows.md](../../../service/specs/03-service-flows.md), [04-http-api.md](../../../service/specs/04-http-api.md), [06-configuration.md](../../../service/specs/06-configuration.md), and [07-telemetry-and-audit.md](../../../service/specs/07-telemetry-and-audit.md).
**Depends on:** —
**Produces:** canonical audit client-context/emission/config/adapter semantics and Lambda/base-path bootstrap semantics.

## Steps

- [ ] Verify session client context and the resolved session wiring open question in `01-domain-model.md`.
- [ ] Verify `03-service-flows.md` covers exchange, refresh, revoke, and admin audit events/context/threshold behavior and removes the resolved suspension-event open question.
- [ ] Verify `04-http-api.md` documents handler consumption of `AuditContext` and Lambda serving of the identical router with base-path stripping.
- [ ] Verify `06-configuration.md` documents `audit.emit_threshold`, its default, `server.base_path`, and both related defaults/semantics.
- [ ] Verify `07-telemetry-and-audit.md` documents pre-dispatch filtering, fallible locked stdout writes, and FIFO SQS group/deduplication behavior.
- [ ] Verify only the five owned pages are touched for this task and their metadata dates are `2026-08-05`.

## Definition of done

- [ ] All audit and Lambda source blocks are represented with equivalent semantics; `AuditEvent` still has no `device_id`, and no canonical schema change is introduced.
- [ ] Negative-space documentation is explicit: default `info` suppresses debug validation failures; failed/unknown revoke token paths emit nothing; audit writes return errors rather than panic; configured base path is stripped before routing in both runtime modes.
- [ ] Every local and source link resolves, including all service-page cross-references and the plan/source links above.
- [ ] No code, schema, change-spec, README-index, certificate, or unrelated canonical-page changes are introduced.
