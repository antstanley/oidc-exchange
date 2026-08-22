# Task 07 — Complete compatibility, docs, and spec integration

**Plan:** [plan.md](../plan.md) · **Status:** Backlog · **Depends on:** [03](03-endpoint_origin_pinning.md), [05](05-jwks_cache_single_flight.md), [06](06-webhook_delivery_binding.md)

**Implements:** source affected canonical pages, type changes, compatibility, and merge plan.

**Scope:** Once the implementation and external-prerequisite ownership are approved, synchronize canonical specs/schema and user documentation, enumerate all compatibility effects, and perform source change-spec merge housekeeping. This is not permission to assume sibling specs have merged.

## Steps

- [ ] Update `.specs/service/specs/02-ports-and-adapters.md`, `05-provider-system.md`, `06-configuration.md`, and `.specs/development-guidelines.md` exactly for the approved behavior; bump dates only at merge.
- [ ] Fold `OidcProviderConfig.endpoint_origins` and `WebhookDelivery` into `.specs/service/specs/canonical-types.schema.json`, resolving the source fragment's refs against the actual canonical schema.
- [ ] Update all affected docs and config reference content found by search, including webhook architecture docs and Google samples; validate TOML/JSON snippets where feasible.
- [ ] Record breaking impacts: webhook receiver signature/header migration, `JwksCache::get_keys` public return-type change, and cross-origin endpoint warning/enforcement rollout. Add release notes and receiver migration instructions.
- [ ] Verify the source's sibling changes are actually merged or their prerequisite pieces are shipped. If not, keep this change Proposed and leave source/canonical merge moves blocked rather than fabricating merged state.
- [ ] When approved, move this change spec to `changes/merged/`, set `Status: Merged` and a merge date, and update `.specs/README.md` pending/merged listings; also repair its missing sibling table references called out by the source.

## Definition of done

- [ ] Canonical prose, schema, implementation behavior, docs, and configuration examples agree.
- [ ] All links and schema `$ref`s resolve from their containing files.
- [ ] Every breaking deployment/embedding consequence has a release note and migration action.
- [ ] Merge bookkeeping occurs only after its explicit prerequisites are satisfied; no done certificate is produced.
