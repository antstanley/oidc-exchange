# Task 04 — Canonical spec and schema sync

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/changes/2026-08-05-validate_revoke_token_claims.md](../../../changes/2026-08-05-validate_revoke_token_claims.md) §Affected spec pages, Proposed changes, Type changes, and merge plan steps 1–4.
**Depends on:** 01, 02, 03 (review — canonical artifacts must describe the implemented session-bound minting, strict validation, and narrowed revoke behavior)
**Produces:** all five affected canonical artifacts accurately describe the merged code and `AccessTokenClaims.sid` schema, while change-spec status/move/README merge bookkeeping remains with the orchestrator.
**Pointers:** `.specs/service/specs/01-domain-model.md:54-84`; `.specs/service/specs/02-ports-and-adapters.md:52-71`; `.specs/service/specs/03-service-flows.md:55-88,141-153`; `.specs/service/specs/04-http-api.md:10-18,146-157`; `.specs/service/specs/canonical-types.schema.json:73-85`; source spec merge plan lines 295-313.

## Steps

- [x] Apply the source spec’s `03-service-flows.md` changes against the code that tasks 01–03
  shipped: replace the access-token revoke-all wording with validator → `sid` → one-session
  revocation; add `Validate access token` after minting; document required `sid` and `at+jwt` in
  minting; reserve `nbf`/`sid`; and add the five decisions. Bump the page date.
- [x] Apply the `01-domain-model.md` updates: state that `refresh_token_hash` is presently the
  session identifier and that `AccessTokenClaims` includes required `sid`. Bump the page date.
- [x] Apply the `02-ports-and-adapters.md` `KeyManager::verify` correction: it authenticates the
  signature as the first validation step, not the whole revoke authorization. Bump the page date.
- [x] Apply the `04-http-api.md` public `/revoke` table and decision changes: the public credential
  reaches only the session it names, invalid/unknown token state remains 200, and backend failure
  remains 503. Bump the page date.
- [x] Update `canonical-types.schema.json` `$defs.AccessTokenClaims`: add `sid` to `properties`
  with the exact 64-lowercase-hex pattern and session-identifier description from the change spec,
  and add it to `required`. Validate JSON syntax and ensure schema/prose/code use the same name and
  semantics.
- [x] Reconcile external proposed specs in prose only: audit/throttling is a prerequisite for the
  source wording that names `AuthenticationFailed`/security durability, and refresh rotation later
  supersedes hash-valued `sid` with `family_id`. Do not implement either proposal or claim it has
  merged; retain their ordering/supersession caveats accurately.
- [x] Do **not** flip the change spec to Merged, add a `Merged:` date, move it to `changes/merged/`,
  or modify `.specs/README.md` as part of implementation completion. Those source-spec merge-plan
  actions remain for the orchestrator after code and canonical review. The planning README entry
  was created with this plan and is not merge bookkeeping.

## Definition of done

- [x] The five canonical artifacts match the code: a typed required `sid`, `at+jwt` minting, header
  and claim validation before claim use, one-session access-token revocation, and unchanged
  200-token-state/503-backend HTTP contract.
- [x] Schema validation/parsing succeeds; `sid` is required and constrained to the current
  SHA-256-hex session identifier without introducing a persistence schema migration.
- [x] No stale prose says access-token `/revoke` calls `revoke_all_user_sessions`, signature-only
  verification is sufficient, or a custom claim can set `sid`/`nbf`.
- [x] External sibling ordering is represented as an external dependency/open question, not silently
  absorbed into this unstacked PR.
- [x] No done certificate or any `*-certificate.md` file is created. Change-spec merge/move and
  README change-spec registry updates are intentionally not performed by this task.

## Completion notes (2026-08-22)

- All five canonical artifacts updated and verified against the shipped code (commits `17cfb106`,
  `540a1ef3`, `3f1f7b50`): `03-service-flows.md` — Revocation rewritten around
  `validate_access_token(token)` → `sid` → one-session `revoke_session(sid)` via the shared
  lookup-revoke-audit helper; new `Validate access token` section immediately after
  `Build access token` stating the shipped seven-step order including boundary semantics (one
  captured `Utc::now()`, `CLOCK_SKEW_SECS = 60`, saturating comparisons, expiry exactly at the
  skew edge still valid, `nbf` parsed separately only after typed required claims);
  `Build access token` steps 2–3 now carry `sid` and header `typ: "at+jwt"`; reserved-claim
  sentence extended to `nbf`/`sid`; exchange/refresh call-site lines updated to
  `build_access_token(user, &session.refresh_token_hash)`; all five Decisions added; page date
  bumped. `01-domain-model.md` — Session paragraph records that `refresh_token_hash` doubles as
  the session's identifier and is carried as access-token `sid`; `AccessTokenClaims` bullet lists
  required `sid` and the six-required-fields discipline; date bumped. `02-ports-and-adapters.md`
  — `KeyManager::verify` note corrected to "first step of validation, not the whole of it" with a
  link to `03-service-flows.md`; date bumped. `04-http-api.md` — public-routes `/revoke` row now
  names the session-scoped authority model; added Decision *Revocation reaches one session*;
  date bumped. `canonical-types.schema.json` — `sid` added to `$defs.AccessTokenClaims`
  `properties` (`type: string`, `pattern: ^[0-9a-f]{64}$`, description verbatim from the change
  spec) and joined `required`.
- Programmatic verification passed: schema parses as JSON; `required == ["sub","iss","aud","iat",
  "exp","sid"]`; pattern accepts 64-lowercase-hex and rejects uppercase/63-char/non-hex values;
  code/prose agreement asserted (`pub sid: String` in `domain/token.rs`,
  `ACCESS_TOKEN_TYP = "at+jwt"` in `service/mod.rs`, `"nbf", "sid"` in `RESERVED_CLAIMS`);
  stale-term scan over all four pages finds no `verify_and_extract_sub`, no `typ: "JWT"`, no
  `revoke_all_user_sessions(sub)`; every relative markdown link resolves to an existing file;
  all four page dates are `2026-08-22`.
- Deviation from source-spec wording (justified): the Revocation block and *Failed revocation is
  recorded* Decision name an `AuthenticationFailed` event "(rendered `ValidationFailed`)" on a
  mandatory channel under `audit.durability = "enforce"`. That API belongs to the
  audit/throttling sibling, which has NOT merged here (task 03 already resolved this per plan:
  shipped code emits `ValidationFailed` at Info severity — identical to the success-path
  `TokenRevocation` emission — under the current `blocking_threshold` durability model). Task 04
  therefore folds the outcome-symmetry requirement in shipped terms and records the sibling as an
  external dependency in `03`'s Assumptions and inline in that Decision; nothing claims it merged,
  and no sibling API was invented.
- Sibling specs are referenced by backticked name only, without markdown links:
  neither `2026-08-05-audit_and_throttle_authentication_failures.md` nor
  `2026-08-05-rotate_refresh_tokens_with_reuse_detection.md` exists under `.specs/changes/` in
  this workspace, so any relative link would dangle (the link-resolution check would fail).
  Ordering/supersession caveats are retained accurately instead.
- `schemas/datamodel.schema.json` considered and intentionally untouched: it contains no
  `AccessTokenClaims` definition (User/Session shapes only), so the authoritative five-artifact
  list in Pointers is complete; no persistence-schema migration introduced anywhere.
- No Rust tests added or changed (docs/schema-only diff); workspace gates re-run unchanged:
  fmt clean, clippy `-D warnings` clean, `cargo nextest run --workspace --no-fail-fast` →
  401 passed / 27 skipped, matching the post-task-03 state.
- Merge bookkeeping deliberately NOT performed (per task steps): change spec remains
  **Proposed** at `.specs/changes/2026-08-05-validate_revoke_token_claims.md` with no
  `Merged:` date; not moved to `changes/merged/`; `.specs/README.md` untouched. No
  certificate file of any kind exists under the plan folder.
