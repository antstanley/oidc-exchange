# Task 07 — Canonical merge and deployment documentation

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/service/specs/00-overview.md](../../../service/specs/00-overview.md) §Goals, §Scope summary, §Non-goals, §Detail pages, and §Crate map; [.specs/service/specs/01-domain-model.md](../../../service/specs/01-domain-model.md) §Entities; [.specs/service/specs/02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §Introduction, §Port traits, and §Adapter inventory; [.specs/service/specs/03-service-flows.md](../../../service/specs/03-service-flows.md) §Token exchange, §Token refresh, §Revocation, §Audit emission and blocking, §Admin operations, and §Decisions; [.specs/service/specs/04-http-api.md](../../../service/specs/04-http-api.md) §Middleware stack, §Error mapping, §Bootstrap, and §Assumptions; [.specs/service/specs/06-configuration.md](../../../service/specs/06-configuration.md) §Validation at load, §Committed default, §Sections, §Defaults summary, and §Assumptions; [.specs/service/specs/07-telemetry-and-audit.md](../../../service/specs/07-telemetry-and-audit.md) §Overview table, §Audit, and §Decisions; [.specs/service/README.md](../../../service/README.md) §Pages; [.specs/service/specs/canonical-types.schema.json](../../../service/specs/canonical-types.schema.json); source spec §Merge plan
**Depends on:** 06
**Produces:** Canonical documentation, schema, deployment guidance, and change-spec/index lifecycle state that truthfully describe the reviewed audit and throttle implementation.
**Pointers:** `.specs/changes/2026-08-05-audit_and_throttle_authentication_failures.md:715`; `.specs/service/specs/00-overview.md`; `.specs/service/specs/01-domain-model.md`; `.specs/service/specs/02-ports-and-adapters.md`; `.specs/service/specs/03-service-flows.md`; `.specs/service/specs/04-http-api.md`; `.specs/service/specs/06-configuration.md`; `.specs/service/specs/07-telemetry-and-audit.md`; `.specs/service/specs/canonical-types.schema.json`; `.specs/service/README.md`; `docs/deployment/linux-server.md:144`; `.specs/README.md`

## Steps

- [ ] Apply every source-spec Proposed-change block to the affected canonical pages and update page dates, preserving unresolved questions that this implementation did not settle.
- [ ] Update the service canonical type schema for `ClientAddrSource`, `ThrottleExceeded`, and `AuditEvent.ip_address_source`; validate schema/prose field and enum parity.
- [ ] Document the default stdout audit, durability modes, rate-limit/trusted-proxy configuration, mandatory/best-effort channels, 429 semantics, middleware ordering, and port/crate counts.
- [ ] Correct Linux deployment proxy guidance so forwarded-address behavior matches the trusted-proxy model and access-log/audit provenance documentation.
- [ ] After implementation and documentation verification, perform only this source spec's merge housekeeping: status/merged date, move into `changes/merged/`, and update `.specs/README.md` Change specs index; retain the plan index and do not merge sibling specs.

## Definition of done

- [ ] Every canonical page and service README section listed in the source spec reflects shipped audit, address, limiter, durability, and HTTP behavior without describing sibling work as implemented.
- [ ] The canonical type schema matches prose and Rust serialization for `ClientAddrSource`, `ThrottleExceeded`, and audit address provenance.
- [ ] Deployment documentation describes a forwarding configuration compatible with right-to-left trusted-hop selection and never presents an asserted client header as trusted.
- [ ] Source-spec merge/status/index updates occur only after implementation verification; sibling change specs remain proposed and untouched.
- [ ] Meets the repo definition of done (documentation/schema checks, Rust checks where touched, and link validation — see plan.md baseline).
- [ ] Reviewable: compare canonical pages, schema, default config, and Linux deployment guide against the passing public-route behavior, then verify the source spec moved and indexes resolve.

## Open questions

- Reconcile overlapping canonical text only when sibling revoke-validation and refresh-rotation work is merged; this task must not fold either implementation into this PR.
