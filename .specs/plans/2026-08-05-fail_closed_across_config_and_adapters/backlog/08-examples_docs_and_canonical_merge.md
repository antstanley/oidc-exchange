# Task 08 — Examples, docs, and canonical merge

**Plan:** [plan.md](../plan.md)  
**Status:** Backlog  
**Implements:** [source spec](../../../changes/2026-08-05-fail_closed_across_config_and_adapters.md) §§Affected spec pages, Proposed changes, Type changes, Implementation notes, and Merge plan 1–4  
**Depends on:** 02, 03, 04, 05, 06, 07  
**Produces:** all source-spec canonical prose/schema deltas, required default/example changes, and shipped documentation reconcile to implementation; no target remains stale.  
**Pointers:** `.specs/architecture-principles.md`; `.specs/service/specs/{02-ports-and-adapters,03-service-flows,05-provider-system,06-configuration,08-persistence,canonical-types.schema.json}`; `.specs/bindings/specs/05-distribution.md`; `config/default.toml`; `examples/`; `docs/`; `README.md`.

## Steps

- [ ] Apply every Proposed changes block to its named canonical page exactly once, bumping each
  edited canonical page date to merge date. Preserve existing checks/prose where the source spec
  says the section is replaced but behavior is subsumed.
- [ ] Fold `AccessTokenClaims.iss` and `.aud` `minLength: 1` and descriptions into the service
  canonical schema; validate schema syntax and any schema-generation/consumer workflow.
- [ ] Update `config/default.toml` to declare issuer/audience placeholders and ensure the prose
  states the default is deliberately not startable without deployment values.
- [ ] Correct all source-spec-named KMS examples and documentation from AWS
  `SigningAlgorithmSpec` terms to JWS `ES256`; sweep related in-scope references so no shipped
  guide teaches a value resolution rejects.
- [ ] Update examples that use production `http://` issuer/provider/webhook URLs to valid HTTPS
  deployment values or clearly test-only fixtures outside shipped runtime config. Run config
  check over every shipped example feasible in the repository and classify missing secrets versus
  invalid value domains.
- [ ] Confirm `04-http-api.md` is unchanged as instructed: its bootstrap wording is owned by the
  placeholder-resolution sibling. Do not add new canonical pages.
- [ ] Perform the source spec's merge housekeeping only after implementation is accepted: move the
  source spec to `changes/merged/`, mark it Merged/date it, and update `.specs/README.md` once.
  The planning task itself does not perform this move.

## Definition of done

- [ ] Each of the eight affected canonical targets named in the source spec has either the exact
  required edit or an explicit no-edit verification where the spec permits one; all Markdown and
  schema references resolve.
- [ ] Canonical prose, runtime config types, examples, and user-facing docs agree on required
  issuer/audience, JWS algorithm vocabulary, HTTPS-only URLs, policy behavior, migration probe,
  and mandatory installer verification.
- [ ] Every shipped example has a documented config-check result; a fixture cannot normalize a
  forbidden production `http://` bypass.
- [ ] The audit/throttle sibling's future `[audit]` changes are not absorbed; this task retains
  the source spec's `noop` snapshot.
- [ ] Canonical merge status/index changes occur only when code is ready to merge, not during
  planning, and no done certificate is created.

## Sibling boundaries

- The audit/throttle spec merges after this one and owns `audit.adapter = "stdout"`, durability,
  and rate-limit additions.
- Placeholder-resolution owns `04-http-api.md` rewording; release-supply-chain owns signature and
  attestation documentation.
