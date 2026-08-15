# Task 04 — Canonical spec and schema sync

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/changes/2026-08-05-validate_revoke_token_claims.md](../../../changes/2026-08-05-validate_revoke_token_claims.md) §Affected spec pages, Proposed changes, Type changes, and merge plan steps 1–4.
**Depends on:** 01, 02, 03 (review — canonical artifacts must describe the implemented session-bound minting, strict validation, and narrowed revoke behavior)
**Produces:** all five affected canonical artifacts accurately describe the merged code and `AccessTokenClaims.sid` schema, while change-spec status/move/README merge bookkeeping remains with the orchestrator.
**Pointers:** `.specs/service/specs/01-domain-model.md:54-84`; `.specs/service/specs/02-ports-and-adapters.md:52-71`; `.specs/service/specs/03-service-flows.md:55-88,141-153`; `.specs/service/specs/04-http-api.md:10-18,146-157`; `.specs/service/specs/canonical-types.schema.json:73-85`; source spec merge plan lines 295-313.

## Steps

- [ ] Apply the source spec’s `03-service-flows.md` changes against the code that tasks 01–03
  shipped: replace the access-token revoke-all wording with validator → `sid` → one-session
  revocation; add `Validate access token` after minting; document required `sid` and `at+jwt` in
  minting; reserve `nbf`/`sid`; and add the five decisions. Bump the page date.
- [ ] Apply the `01-domain-model.md` updates: state that `refresh_token_hash` is presently the
  session identifier and that `AccessTokenClaims` includes required `sid`. Bump the page date.
- [ ] Apply the `02-ports-and-adapters.md` `KeyManager::verify` correction: it authenticates the
  signature as the first validation step, not the whole revoke authorization. Bump the page date.
- [ ] Apply the `04-http-api.md` public `/revoke` table and decision changes: the public credential
  reaches only the session it names, invalid/unknown token state remains 200, and backend failure
  remains 503. Bump the page date.
- [ ] Update `canonical-types.schema.json` `$defs.AccessTokenClaims`: add `sid` to `properties`
  with the exact 64-lowercase-hex pattern and session-identifier description from the change spec,
  and add it to `required`. Validate JSON syntax and ensure schema/prose/code use the same name and
  semantics.
- [ ] Reconcile external proposed specs in prose only: audit/throttling is a prerequisite for the
  source wording that names `AuthenticationFailed`/security durability, and refresh rotation later
  supersedes hash-valued `sid` with `family_id`. Do not implement either proposal or claim it has
  merged; retain their ordering/supersession caveats accurately.
- [ ] Do **not** flip the change spec to Merged, add a `Merged:` date, move it to `changes/merged/`,
  or modify `.specs/README.md` as part of implementation completion. Those source-spec merge-plan
  actions remain for the orchestrator after code and canonical review. The planning README entry
  was created with this plan and is not merge bookkeeping.

## Definition of done

- [ ] The five canonical artifacts match the code: a typed required `sid`, `at+jwt` minting, header
  and claim validation before claim use, one-session access-token revocation, and unchanged
  200-token-state/503-backend HTTP contract.
- [ ] Schema validation/parsing succeeds; `sid` is required and constrained to the current
  SHA-256-hex session identifier without introducing a persistence schema migration.
- [ ] No stale prose says access-token `/revoke` calls `revoke_all_user_sessions`, signature-only
  verification is sufficient, or a custom claim can set `sid`/`nbf`.
- [ ] External sibling ordering is represented as an external dependency/open question, not silently
  absorbed into this unstacked PR.
- [ ] No done certificate or any `*-certificate.md` file is created. Change-spec merge/move and
  README change-spec registry updates are intentionally not performed by this task.
