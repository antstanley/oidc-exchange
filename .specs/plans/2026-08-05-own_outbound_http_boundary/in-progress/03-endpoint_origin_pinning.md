# Task 03 — Pin discovery endpoint origins and wire config

**Plan:** [plan.md](../plan.md) · **Status:** Backlog · **Depends on:** [01](01-prerequisite_contracts_and_guard_lint.md), [02](02-provider_transport.md)

**Implements:** source `05-provider-system.md`/`06-configuration.md` deltas; type fragment `OidcProviderConfig`; implementation note 8; origin-pinning tests.

**Scope:** Add `endpoint_origins` to the domain/config/bootstrap/provider construction path, calculate each provider's allowed origin set from issuer, configured endpoints, and config extras, and validate discovery endpoint origins under the externally confirmed `HttpsUrl` contract. Ship warning mode first, with a separately gated enforcement flip. Update every shipped Google configuration/documentation occurrence, not only the source note's partial file list.

## Steps

- [ ] Extend `OidcProviderConfig`, parser fixtures, and `provider_config_to_oidc`; define config validation for HTTPS origin-only values and preserve redacted debug behavior.
- [ ] Pass permitted origins into discovery for OIDC and the Apple construction path where applicable; reject/warn with endpoint, observed origin, and permitted set as specified.
- [ ] Implement warning-mode telemetry and an explicit enforcement configuration/release switch or documented follow-up boundary; do not enforce without the one-release warning decision.
- [ ] Update all in-repo Google stanzas surfaced by repository search: examples, README files, deployment/guides, and config tests; preserve placeholders and no-secret rules.
- [ ] Add tests for undeclared rejection/enforcement, declared acceptance, issuer/configured endpoint inclusion, invalid origin syntax/scheme, and Google's multiple cross-origin discovery shape.

## Definition of done

- [ ] Discovery cannot introduce an unpinned origin once enforcement is enabled; issuer/configured/declared origins behave exactly as documented.
- [ ] Warning mode produces structured actionable output without rejecting the same deployment, and the enforcement transition has release-owner approval.
- [ ] Every shipped Google sample names both required Google API origins and remains parseable.
- [ ] Canonical type/prose updates are deferred to 07 unless this change is approved for merge; no done certificate is produced.
