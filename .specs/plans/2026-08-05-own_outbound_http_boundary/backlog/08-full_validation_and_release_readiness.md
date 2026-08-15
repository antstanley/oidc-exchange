# Task 08 — Run full validation and release-readiness review

**Plan:** [plan.md](../plan.md) · **Status:** Backlog · **Depends on:** [01](01-prerequisite_contracts_and_guard_lint.md), [02](02-provider_transport.md), [03](03-endpoint_origin_pinning.md), [04](04-verification_key_set.md), [05](05-jwks_cache_single_flight.md), [06](06-webhook_delivery_binding.md), [07](07-compatibility_docs_and_spec_integration.md)

**Implements:** repository definition of done and source compatibility/merge gates.

**Scope:** Execute Rust quality gates and review the complete boundary invariants, source coverage, documentation, configuration, and release decisions. This task reports evidence in review output/task status and intentionally creates no done certificate.

## Steps

- [ ] Run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace`; resolve failures without bypasses.
- [ ] Run focused adapter/provider wiremock tests for body limits, status ordering, origin pinning, C12 corpus, cache concurrency, and webhook signing/retry identity.
- [ ] Audit all outbound provider HTTP call sites and JWK conversion paths by repository search; verify webhook remains the only separate outbound owner.
- [ ] Recheck source coverage table, task links, DAG ordering, canonical/schema refs, TOML/JSON examples, and status placement in the kanban index.
- [ ] Confirm warning-to-enforcement rollout and webhook receiver migration/release notes have an owner and a deployment window; block release if either is unresolved.

## Definition of done

- [ ] All required Rust gates are clean with actual command output recorded in review context.
- [ ] Every source test requirement has a passing positive and negative test or an explicitly approved, tracked exception.
- [ ] The task graph remains acyclic and all required task links resolve.
- [ ] No done certificate is produced.
