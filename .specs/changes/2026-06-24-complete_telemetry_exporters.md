# Change: Complete the OTLP and X-Ray telemetry exporters

**Status:** Proposed · **Date:** 2026-06-24 · **Owner:** Ant Stanley · **Target:** crates/server

Wire the `otlp` and `xray` telemetry exporters and the tower OTEL request-span layer so the
`[telemetry]` configuration that already accepts these values actually exports spans, instead of
warning and falling back to JSON logging.

---

## Motivation

`[telemetry].exporter` accepts `otlp` and `xray`, but `telemetry::init_telemetry` currently logs
a warning and falls back to the JSON `tracing_subscriber` formatter for both. The canonical spec
records this as a divergence in [07-telemetry-and-audit.md](../service/specs/07-telemetry-and-audit.md)
and [04-http-api.md](../service/specs/04-http-api.md). Operators configuring OTLP get logs, not
traces.

Completing the exporters lets the service emit real distributed traces to an OTEL collector
(production) or X-Ray (Lambda-native), with HTTP request spans and trace-context propagation
from the incoming `traceparent` header — the observability the design always intended.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/07-telemetry-and-audit.md`](../service/specs/07-telemetry-and-audit.md) | Replace the "fall back to JSON" behaviour with implemented OTLP/X-Ray export; remove the Open question |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | Remove the telemetry Open question; note the tower OTEL span layer in the middleware stack |

---

## Proposed changes

### `.specs/service/specs/07-telemetry-and-audit.md` → Telemetry (Modify)

> Current behaviour by `[telemetry].exporter`:
>
> - `none` → no exporter; a `tracing_subscriber` JSON formatter handles logs only.
> - `stdout` → spans printed to stdout as JSON.
> - `otlp` → spans exported to the configured `endpoint` over the configured `protocol`
>   (`grpc` or `http`) via `opentelemetry-otlp`, sampled at `sample_rate`.
> - `xray` → spans exported using the AWS X-Ray id generator and propagator
>   (`opentelemetry-aws`); the X-Ray trace header is propagated in Lambda.
>
> Instrumentation continues to use `tracing`; the server bridges spans to OTEL with a
> `tracing-opentelemetry` layer installed at startup.

### `.specs/service/specs/04-http-api.md` → Middleware stack (Modify)

> A tower OTEL layer wraps all routes: it opens a span per request (method, path, status,
> latency) and adopts the trace context from an incoming `traceparent` header so traces stitch
> across services.

---

## Type changes

None. No domain entity or config field changes (`endpoint`, `protocol`, `sample_rate`,
`service_name` already exist on `TelemetryConfig`).

---

## Implementation notes

1. Add `opentelemetry`, `opentelemetry-otlp`, `opentelemetry_sdk`, `tracing-opentelemetry`, and
   `opentelemetry-aws` (xray) to `crates/server`.
2. In `crates/server/src/telemetry.rs`, replace the `otlp`/`xray` warning branches: build the
   exporter, a sampler from `sample_rate`, a `Resource` from `service_name`, and register a
   `tracing-opentelemetry` layer alongside the existing fmt layer.
3. For `xray`, install the X-Ray id generator and propagator.
4. Add the tower OTEL HTTP-span layer to the router in `crates/server/src/routes/mod.rs`,
   outermost in the stack, reading `traceparent`.
5. Ensure clean shutdown flushes the exporter (tracer provider shutdown) in both server and
   Lambda paths.

References: `opentelemetry-otlp`, `tracing-opentelemetry`, `opentelemetry-aws` crate docs.

---

## Merge plan

1. Apply both `Proposed changes` blocks to their canonical pages; bump their `**Date:**`.
2. Remove the telemetry Open questions from 07-telemetry-and-audit and 04-http-api.
3. No schema change.
4. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
5. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- An OTEL collector endpoint (or X-Ray in Lambda) is reachable from the deployment; export is
  best-effort and never blocks a request.

### Decisions

- *Keep `tracing` as the API.* **Exporters are added only in `crates/server`.** Core and adapters
  stay OTEL-free; only the bridge layer changes.

### Open questions

- Whether metrics (not just traces) are in scope for OTLP is undecided; this change covers traces.
- Default `sample_rate` for production (currently `1.0`) may need lowering; left to per-deployment
  config.
