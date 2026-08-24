# Task 01 — Establish prerequisite contracts and guard lint

**Plan:** [plan.md](../plan.md) · **Status:** Done · **Depends on:** —

**Implements:** source implementation notes 2, 4–5; external dependencies named in source §Affected spec pages and §Assumptions.

**Scope:** Establish the exact contracts for sibling-owned `HttpsUrl`, `http::read_bounded`/`MAX_UPSTREAM_BODY_BYTES`, and `upstream::error_detail`, which are absent in this unstacked checkout. Add root `clippy.toml` with only the source-specified Tokio guard types for `await-holding-invalid-types`; make the current JWKS violations visible without suppressing them. Capture the pre-consolidation OIDC/Apple key-selection corpus result as review evidence.

## Steps

- [x] Confirm whether each sibling artifact is available through a separately merged dependency, must be coordinated into this PR, or blocks work; record its module path, signature, error type, and test expectations in the change/review context.
- [x] Do not invent replacement helper semantics while ownership is unresolved; if this PR must carry them, obtain an explicit scope decision and add a separate dependency task before 02/03/05.
- [x] Add `clippy.toml` at workspace root listing the three Tokio guard types specified by the source; run clippy to prove it reports the two current JWKS lock-across-await sites.
- [x] Add a temporary or committed, deterministic cross-provider baseline corpus that records acceptance/rejection for every source-listed JWK case before selector consolidation, including `use: sig` non-regression and duplicate-`kid` array-order cases.

## Definition of done

- [x] Each absent sibling requirement has an explicit disposition; dependent tasks identify the actual API rather than an assumed one.
- [x] `clippy.toml` contains no unrelated lint policy and detects both stated guard-across-await violations.
- [x] The C12 baseline corpus covers OIDC and Apple paths with identical fixtures and reports their current behavior for review.
- [x] No done certificate is produced.

## Notes (completion record)

**Sibling artifact dispositions.** None of the three sibling changes
(`fail_closed_across_config_and_adapters`, `eliminate_secret_leakage_in_logs_and_spans`,
`bind_id_token_grant_replay_protection`) is merged into this branch, so no artifact was
available through any dependency. Per the plan's execution order ("resolve or explicitly
vendor/coordinate"), each needed artifact is vendored locally in the crate that owns the
outbound HTTP layer, marked in code comments as a vendored prerequisite so the owning PR
reconciles ownership:

| Artifact | Vendored location | Signature | Error type |
|---|---|---|---|
| `HttpsUrl` (sibling: fail_closed) | `crates/adapters/src/shared/http.rs` | `parse(&str) -> Result<HttpsUrl, HttpsUrlError>`; `as_str()`, `as_url()` | `HttpsUrlError::{NotAnAbsoluteUrl, SchemeNotHttps{actual_scheme}}` |
| `MAX_UPSTREAM_BODY_BYTES`, `read_bounded_bytes` (sibling: eliminate_secret_leakage) | `crates/adapters/src/shared/http.rs` | `const MAX_UPSTREAM_BODY_BYTES: u64 = 64 * 1024`; `read_bounded_bytes(Response) -> Result<Vec<u8>, BoundedBodyError>` | `BoundedBodyError::{OverLimit{limit_bytes}, Network(reqwest::Error)}` — fails AT the limit |
| `upstream::error_detail` (sibling: eliminate_secret_leakage) | `crates/adapters/src/shared/upstream.rs` | `error_detail(StatusCode, &[u8]) -> String` — surfaces bounded OAuth `error`/`error_description`, never echoes bodies | returns `String` |

Scope guard held: only what tasks 02/04 strictly need was vendored. `HttpsUrl` is
contract-pinned with tests but NOT yet consumed by the transport (see task 02 notes);
origin pinning stays in task 03; the replay-protection sibling needs nothing vendored here.

**Guard lint evidence.** With `clippy.toml` committed (three tokio guard types, nothing
else), clippy reported exactly the two predicted sites before any fix:
`shared/jwks.rs:66` (`get_keys`' cache write guard across `fetch_keys`) and
`shared/jwks.rs:138` (`refresh`'s `last_forced_refetch` write guard across `fetch_keys`).
Both are now fixed by releasing the guard before the fetch and re-acquiring only to store
— no `#[allow]`, no suppression; the lint stays active for future violations. The interim
ordering releases the implicit fetch serialization, which is safe only because the byte
ceiling landed first (task 02 landed before this commit deliberately); the full
single-flight redesign remains task 05, which now starts from guards-free-of-network code.

**C12 baseline corpus.** Committed at `crates/providers/tests/cross_provider_corpus.rs`
with deterministic fixtures in `crates/test-utils/src/corpus.rs` (embedded RSA-2048 pair +
P-256 pair from seed `[42u8; 32]`; twelve source-listed cases). Baseline result: the two
validators disagree on **6 of 12 cases** today (absent-alg ×2, RSA-OAEP, alg-less enc EC,
duplicate-kid-enc-first, `alg:"none"`) — contradiction C12 recorded as an executable,
drift-detecting table rather than prose. Both `use: sig` non-regression cases verify on
both paths. Task 04 flips this table to uniform dispositions as its C12-closure evidence.

No done certificate produced.
