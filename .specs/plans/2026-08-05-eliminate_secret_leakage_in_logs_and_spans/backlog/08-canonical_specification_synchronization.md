# Task 08 — Synchronize canonical specifications

**Status:** Backlog · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §Merge plan steps 1–4](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#merge-plan); canonical targets [`01-domain-model.md`](../../../service/specs/01-domain-model.md), [`02-ports-and-adapters.md`](../../../service/specs/02-ports-and-adapters.md), [`04-http-api.md`](../../../service/specs/04-http-api.md), [`05-provider-system.md`](../../../service/specs/05-provider-system.md), [`06-configuration.md`](../../../service/specs/06-configuration.md), [`07-telemetry-and-audit.md`](../../../service/specs/07-telemetry-and-audit.md), and [`08-persistence.md`](../../../service/specs/08-persistence.md)
**Depends on:** 07
**Produces:** all affected canonical documentation precisely describes demonstrated behavior and records no schema change; the proposed source change remains for merge-process ownership.
**Pointers:** source spec proposed-change blocks and type-change rationale; `.specs/service/specs/`; `.specs/README.md` merge-owned change table.

## Steps

- [ ] Apply each source-spec canonical block after implementation is demonstrably complete; bump each affected canonical page date in the same change.
- [ ] Add `Telemetry hygiene` to 07 with type-enforced formatting, public-description separation, and explicit session-instrumentation rules.
- [ ] Replace—not append beside—the superseded per-type redacting-`Debug` decision in 06; update `[user_sync]` and `[internal_api]` wording for `Secret<String>`.
- [ ] Document `Session`/repository signatures, request-ID bounds, provider error flow, Apple assertion type, and session-adapter span rules exactly as shipped.
- [ ] Confirm `canonical-types.schema.json` and `schemas/datamodel.schema.json` remain untouched because serde/wire/store shape is unchanged; validate all internal Markdown links.

## Task-specific definition of done

- [ ] Exactly the seven source-listed canonical pages are updated and their dates bumped; no unrelated canonical page changes are introduced.
- [ ] Canonical prose matches tests/code, including named bounds and the no-schema-change rationale.
- [ ] The source change remains `Proposed`; moving/stamping it and updating `.specs/README.md` are explicitly left to the merge process.
- [ ] No certificate file is created; review plus link validation is the completion evidence.
