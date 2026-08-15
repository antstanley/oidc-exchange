# Task 01 — Session-bound access-token contract

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/changes/2026-08-05-validate_revoke_token_claims.md](../../../changes/2026-08-05-validate_revoke_token_claims.md) §Type changes and implementation notes 1–3 and 7; the code portion of its `Build access token` and `Custom claims` deltas.
**Depends on:** —
**Produces:** every newly minted access token has a required `sid` naming its current session, has a pinned `typ: "at+jwt"` header, and cannot have `sid` overridden by custom claims.
**Pointers:** `crates/core/src/domain/token.rs:35-46`; `crates/core/src/service/mod.rs:63-100`; `crates/core/src/service/exchange.rs:291-316`; `crates/core/src/service/refresh.rs:22-128`; `crates/core/src/service/claims.rs:7-45`; `crates/core/tests/exchange.rs:263-372,1479-1487`; `crates/core/tests/refresh.rs:94-137`; `crates/core/tests/claims.rs:89-127`.

## Steps

- [ ] Add the required `pub sid: String` field to `AccessTokenClaims` before flattened `custom`.
  Retain serde’s required-field behavior: old payloads that omit `sid` must fail to deserialize.
- [ ] Change `AppService::build_access_token` to accept `sid: &str`; assert both the user id and
  session identifier are non-empty before building the payload, copy the value into claims, and
  mint `{ alg: keys.algorithm(), typ: "at+jwt", kid: keys.key_id() }`.
- [ ] At the exchange call site, pass `&session.refresh_token_hash` only after the session has been
  successfully stored. At the refresh call site, pass the hash from the session already resolved
  by the presented refresh token. Update every internal caller; do not add compatibility overloads.
- [ ] Add `nbf` and `sid` to `RESERVED_CLAIMS` and update its documentation. Extend
  `crates/core/tests/claims.rs` so both config and per-user reserved-claim tests prove `sid` and
  `nbf` are dropped while unrelated custom claims remain.
- [ ] Extend exchange tests to decode the access JWT header and claims: `typ == "at+jwt"`, and
  `sid == sha256_hex(returned refresh token) == stored session.refresh_token_hash`. Extend refresh
  tests to prove the refreshed access JWT has the same `sid` as the original session hash.
- [ ] Keep touched functions within the review limits: at least two meaningful assertions per
  touched/new function, no magic identifier length, no new dependency, and comments explain why
  the hash is bound to the session rather than describing serialization.

## Definition of done

- [ ] `AccessTokenClaims` cannot deserialize without `sid`, and all workspace constructors/callers
  compile after the type change.
- [ ] Exchange and refresh mint signed `at+jwt` tokens whose `sid` identifies precisely the current
  stored session; refresh leaves the session identifier stable.
- [ ] A config template or user claim attempting to supply `sid` or `nbf` is silently excluded;
  normal custom claims still serialize.
- [ ] Negative-space coverage proves the `sid` binding cannot be supplied by callers/custom claims
  and old payload shape is rejected on deserialization.
- [ ] Relevant core claims/exchange/refresh tests pass, then Rust format, clippy, and the workspace
  test command are run and reported. Do not repair the known unrelated three config-test failures
  caused by missing `providers.*.adapter`.
- [ ] Do not create a done certificate or any `*-certificate.md` file; leave this task package as
  the sole task artifact while it moves through kanban states.
