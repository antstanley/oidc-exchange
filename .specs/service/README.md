# Service specs

The Rust OIDC exchange service (`crates/*`). These per-package specs build on the global
specs — read those first:

- [../architecture-principles.md](../architecture-principles.md) — layering and dependency rules
- [../development-guidelines.md](../development-guidelines.md) — toolchain and coding discipline
- [../canonical-types.schema.json](../canonical-types.schema.json) — shared primitive types

## Pages

| Page | Covers |
|---|---|
| [specs/00-overview.md](specs/00-overview.md) | problem, goals, system shape, crate map, scope |
| [specs/01-domain-model.md](specs/01-domain-model.md) | entities, ids, lifecycles, query patterns |
| [specs/02-ports-and-adapters.md](specs/02-ports-and-adapters.md) | the six ports and every adapter |
| [specs/03-service-flows.md](specs/03-service-flows.md) | exchange, refresh, revoke, admin, claims, audit blocking |
| [specs/04-http-api.md](specs/04-http-api.md) | routes, middleware, roles, bootstrap, error mapping |
| [specs/05-provider-system.md](specs/05-provider-system.md) | provider tiers, OIDC and Apple, the registry |
| [specs/06-configuration.md](specs/06-configuration.md) | the full `AppConfig`, loading order, defaults |
| [specs/07-telemetry-and-audit.md](specs/07-telemetry-and-audit.md) | tracing/telemetry vs the audit trail |
| [specs/08-persistence.md](specs/08-persistence.md) | DynamoDB single-table and the SQL/embedded session stores |
| [specs/canonical-types.schema.json](specs/canonical-types.schema.json) | JSON Schema for every service entity |
