# Telemetry and Audit

**Status:** Implemented · **Date:** 2026-08-22 · **Owner:** Ant Stanley · **Scope:** crates/server/src/telemetry.rs, crates/core audit

Two independent observability systems with different purposes.

| | Telemetry | Audit |
|---|---|---|
| Purpose | operational monitoring | compliance / security record |
| Data | tracing spans / structured logs | `AuditEvent`s with syslog severity |
| Failure mode | best-effort, never blocks | mandatory channel per `audit.durability`; best-effort per configured threshold |
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

Audit has two channels. The **mandatory** channel carries `SecurityEvent`s
([01-domain-model.md](01-domain-model.md)) through `AppService::emit_security_event`. No
configured threshold filters it: sink failures are governed by `audit.durability` —
`enforce` fails the operation and `observe` records degradation while allowing it. The
**best-effort** channel is `emit_audit`, retaining `emit_threshold` and
`blocking_threshold` for operational events. Shipped flows use the mandatory channel.

Severity remains on both channels because sinks and SIEMs route and alert on it; on the
mandatory channel it never determines whether a security event exists.

Adapters: `stdout_audit` writes JSON lines with locked handles, and `Auto` routes
error-and-above to stderr; a write failure (e.g. EPIPE from a restarted log collector)
returns `AuditError` rather than panicking. `sqs_audit` sends one JSON message per event
with a `severity` attribute and detects FIFO queues from the `.fifo` suffix; on FIFO queues
it sets `message_group_id` to the event id — each event is its own group, so FIFO ordering
never serializes throughput — with the event's ULID as the deduplication id. `noop` drops
events. `stdout` is the committed default.

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
- *Audit fallback before durability handling.* **A failed audit emit is logged via tracing
  first.** The fallback is diagnostic evidence, while `audit.durability` governs the
  mandatory channel.
- *Stdout audit by default.* **The committed default uses stdout.** It requires no
  credentials or network and is collected by Lambda, container log drivers, and journald;
  `noop` remains available for tests and local development.

### Open questions

- OTLP and X-Ray exporters are configurable but not yet implemented (they fall back to JSON
  logging). Completing them — plus the tower OTEL request-span layer — is pending and belongs
  in a change spec.
