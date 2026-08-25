# Task 01 — Session-bound access-token contract

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/changes/merged/2026-08-05-validate_revoke_token_claims.md](../../../changes/merged/2026-08-05-validate_revoke_token_claims.md) §Type changes and implementation notes 1–3 and 7; the code portion of its `Build access token` and `Custom claims` deltas.
**Depends on:** —
**Produces:** every newly minted access token has a required `sid` naming its current session, has a pinned `typ: "at+jwt"` header, and cannot have `sid` overridden by custom claims.
**Pointers:** `crates/core/src/domain/token.rs:35-46`; `crates/core/src/service/mod.rs:63-100`; `crates/core/src/service/exchange.rs:291-316`; `crates/core/src/service/refresh.rs:22-128`; `crates/core/src/service/claims.rs:7-45`; `crates/core/tests/exchange.rs:263-372,1479-1487`; `crates/core/tests/refresh.rs:94-137`; `crates/core/tests/claims.rs:89-127`.

## Steps

- [x] Add the required `pub sid: String` field to `AccessTokenClaims` before flattened `custom`.
  Retain serde’s required-field behavior: old payloads that omit `sid` must fail to deserialize.
- [x] Change `AppService::build_access_token` to accept `sid: &str`; assert both the user id and
  session identifier are non-empty before building the payload, copy the value into claims, and
  mint `{ alg: keys.algorithm(), typ: "at+jwt", kid: keys.key_id() }`.
- [x] At the exchange call site, pass `&session.refresh_token_hash` only after the session has been
  successfully stored. At the refresh call site, pass the hash from the session already resolved
  by the presented refresh token. Update every internal caller; do not add compatibility overloads.
- [x] Add `nbf` and `sid` to `RESERVED_CLAIMS` and update its documentation. Extend
  `crates/core/tests/claims.rs` so both config and per-user reserved-claim tests prove `sid` and
  `nbf` are dropped while unrelated custom claims remain.
- [x] Extend exchange tests to decode the access JWT header and claims: `typ == "at+jwt"`, and
  `sid == sha256_hex(returned refresh token) == stored session.refresh_token_hash`. Extend refresh
  tests to prove the refreshed access JWT has the same `sid` as the original session hash.
- [x] Keep touched functions within the review limits: at least two meaningful assertions per
  touched/new function, no magic identifier length, no new dependency, and comments explain why
  the hash is bound to the session rather than describing serialization.

## Definition of done

- [x] `AccessTokenClaims` cannot deserialize without `sid`, and all workspace constructors/callers
  compile after the type change.
- [x] Exchange and refresh mint signed `at+jwt` tokens whose `sid` identifies precisely the current
  stored session; refresh leaves the session identifier stable.
- [x] A config template or user claim attempting to supply `sid` or `nbf` is silently excluded;
  normal custom claims still serialize.
- [x] Negative-space coverage proves the `sid` binding cannot be supplied by callers/custom claims
  and old payload shape is rejected on deserialization.
- [x] Relevant core claims/exchange/refresh tests pass, then Rust format, clippy, and the workspace
  test command are run and reported. Do not repair the known unrelated three config-test failures
  caused by missing `providers.*.adapter`.
- [x] Do not create a done certificate or any `*-certificate.md` file; leave this task package as
  the sole task artifact while it moves through kanban states.

## Completion notes (2026-08-22)

- Implemented as specified. The typ media type lives in one named constant,
  `service::ACCESS_TOKEN_TYP` (`"at+jwt"`), shared by minting here and by task 02's validator so
  the two boundaries cannot drift.
- Workspace gates after this task: `cargo fmt --all --check` clean;
  `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo nextest run --workspace`
  → 388 passed / 27 skipped (baseline was 387 passed / 27 skipped; +1 new deserialization test).
- **Reconciliation note for PR #22 merge (Ant):** the overlap point between this branch and
  #22's vendored seam is `AccessTokenClaims` in `crates/core/src/domain/token.rs` — both branches
  add a required `pub sid: String` field immediately before the flattened `custom` map. This
  branch populates it with the SESSION's `refresh_token_hash` (`sha256_hex(refresh token)`) and
  pins header `typ: "at+jwt"` per this plan; #22's vendored copy populates `sid` with the FAMILY
  id (`fam_...`) and keeps `typ: "JWT"`. Secondary overlap points: the
  `build_access_token(&self, user, sid)` signature and its two call sites in `exchange.rs` /
  `refresh.rs`, plus the `RESERVED_CLAIMS` extension in `claims.rs` (both branches add entries).
  When merging, take THIS branch's semantics for the field position/requiredness and the
  reserved-claim list; #22's rotation work re-points the *value* later per both specs' stated
  supersession order. No family-id code exists on this branch by design.
