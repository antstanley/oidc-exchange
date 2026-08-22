# Task 01 — Establish prerequisite contracts and guard lint

**Plan:** [plan.md](../plan.md) · **Status:** Backlog · **Depends on:** —

**Implements:** source implementation notes 2, 4–5; external dependencies named in source §Affected spec pages and §Assumptions.

**Scope:** Establish the exact contracts for sibling-owned `HttpsUrl`, `http::read_bounded`/`MAX_UPSTREAM_BODY_BYTES`, and `upstream::error_detail`, which are absent in this unstacked checkout. Add root `clippy.toml` with only the source-specified Tokio guard types for `await-holding-invalid-types`; make the current JWKS violations visible without suppressing them. Capture the pre-consolidation OIDC/Apple key-selection corpus result as review evidence.

## Steps

- [ ] Confirm whether each sibling artifact is available through a separately merged dependency, must be coordinated into this PR, or blocks work; record its module path, signature, error type, and test expectations in the change/review context.
- [ ] Do not invent replacement helper semantics while ownership is unresolved; if this PR must carry them, obtain an explicit scope decision and add a separate dependency task before 02/03/05.
- [ ] Add `clippy.toml` at workspace root listing the three Tokio guard types specified by the source; run clippy to prove it reports the two current JWKS lock-across-await sites.
- [ ] Add a temporary or committed, deterministic cross-provider baseline corpus that records acceptance/rejection for every source-listed JWK case before selector consolidation, including `use: sig` non-regression and duplicate-`kid` array-order cases.

## Definition of done

- [ ] Each absent sibling requirement has an explicit disposition; dependent tasks identify the actual API rather than an assumed one.
- [ ] `clippy.toml` contains no unrelated lint policy and detects both stated guard-across-await violations.
- [ ] The C12 baseline corpus covers OIDC and Apple paths with identical fixtures and reports their current behavior for review.
- [ ] No done certificate is produced.
