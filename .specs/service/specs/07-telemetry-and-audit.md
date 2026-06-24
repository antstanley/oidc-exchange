# Telemetry and Audit

**Status:** Implemented · **Date:** 2026-06-24 · **Owner:** Ant Stanley · **Scope:** crates/server/src/telemetry.rs, crates/core audit

Two independent observability systems with different purposes.

| | Telemetry | Audit |
|---|---|---|
| Purpose | operational monitoring | compliance / security record |
| Data | tracing spans / structured logs | `AuditEvent`s with syslog severity |
| Failure mode | best-effort, never blocks | blocks per configured threshold |
| Backend | tracing subscriber (JSON) → log pipeline | stdout / SQS / noop adapters |

## Telemetry (`telemetry::init_telemetry`)

Instrumentation uses the `tracing` ecosystem: `crates/core` and the adapters emit spans and
events with no OTEL awareness; `crates/server` installs the subscriber at startup, before any
other work, so all subsequent spans are captured.

Current behaviour by `[telemetry].exporter`:

- `none` / `stdout` → a `tracing_subscriber` JSON formatter (structured logs to stdout).
- `otlp` / `xray` → accepted, but currently emit a warning and **fall back to the JSON
  formatter**; the OTLP/X-Ray exporters and the tower OTEL HTTP-span layer are not yet wired.

The env filter honours `RUST_LOG`, defaulting to `info`. In Lambda these JSON lines are
captured by CloudWatch Logs; in containers by the log driver.

## Audit

`AuditEvent`s ([01-domain-model.md](01-domain-model.md)) are emitted through the
[`AuditLog`](02-ports-and-adapters.md) port and gated by `AppService::emit_audit`
([03-service-flows.md](03-service-flows.md)): on backend failure the event is first written
to a tracing log, then — if its severity is at or above `audit.blocking_threshold` — the
operation fails; otherwise it proceeds with a warning.

Adapters: `stdout_audit` (JSON lines; `Auto` sends error-and-above to stderr, the rest to
stdout), `sqs_audit` (one JSON message per event with a `severity` attribute, FIFO detected by
a `.fifo` queue suffix), and `noop` (drops events; the default).

A single request can produce both a telemetry trace and an audit event; they correlate
through the request id but are otherwise independent.

## Relationship to the rest of the spec

- Audit event types and severities: [01-domain-model.md](01-domain-model.md).
- The blocking algorithm and which flows audit what: [03-service-flows.md](03-service-flows.md).
- Audit adapter configuration: [06-configuration.md](06-configuration.md).

## Assumptions and open questions

### Assumptions

- The log pipeline (CloudWatch, container log driver, etc.) ingests stdout/stderr JSON; the
  service does not ship logs itself.
- Audit and telemetry can be reasoned about separately because audit never depends on the
  telemetry exporter being healthy.

### Decisions

- *Tracing as the instrumentation API.* **Core and adapters use `tracing`, only the server
  bridges to exporters.** Keeps `crates/core` free of OTEL dependencies.
- *Audit fallback before blocking.* **A failed audit emit is logged via tracing first.** A
  compliance event is never lost even when the dedicated backend is down.
- *Noop audit by default.* **The committed default uses the noop audit adapter.** A fresh boot
  is silent until an operator opts into stdout or SQS auditing.

### Open questions

- OTLP and X-Ray exporters are configurable but not yet implemented (they fall back to JSON
  logging). Completing them — plus the tower OTEL request-span layer — is pending and belongs
  in a change spec.
