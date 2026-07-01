# Done Certificate — Task 03: Refetch JWKS on an unknown kid in both providers

**Task:** [03-refetch_on_unknown_kid.md](03-refetch_on_unknown_kid.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** An unknown `kid` triggers exactly one rate-limited JWKS refetch in both `OidcProvider` and `AppleProvider` before the token is rejected, so a rotated signing key validates on the next login without a TTL wait.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the rest of `validate_id_token` in either provider — header decode, JWK→decoding-key build, algorithm-from-JWK selection, issuer/audience validation, and `sub` extraction (`oidc/mod.rs:82-167`, `apple.rs:210-290`) all still run once the `kid` resolves.

## Obligations

- **O1 — Unknown `kid` triggers one refetch then validates, in `OidcProvider`.**
  - *Claim:* at `oidc/mod.rs:103-108` the terminal `InvalidGrant` is replaced by a forced refetch (task 02 API) and a re-search; a token whose `kid` was absent then present validates.
  - *Evidence to collect:* read `oidc/mod.rs` around the `kid` lookup; confirm the miss path calls the refetch API and re-searches before rejecting. Run the new `wiremock` rotation test for `OidcProvider` — expect PASS (first key set omits the `kid`, second contains it, validation succeeds without a TTL sleep).
  - *Checks:* resolve the refetch call — confirm it is task 02's rate-limited API on `JwksCache`, not a fresh unbounded fetch.
  - *Status:* ☐ unverified

- **O2 — Unknown `kid` triggers one refetch then validates, in `AppleProvider`.**
  - *Claim:* the same change is applied at `apple.rs:231-236`.
  - *Evidence to collect:* read `apple.rs` around the `kid` lookup; run the `AppleProvider` rotation `wiremock` test — expect PASS.
  - *Checks:* resolve the refetch call at `apple.rs` to the same `JwksCache` rate-limited API.
  - *Status:* ☐ unverified

- **O3 — Refetch is rate-limited; a still-missing `kid` is rejected without a loop.**
  - *Claim:* repeated unknown `kid`s do not each cause a network fetch (task 02's `MIN_REFRESH_INTERVAL` guard holds), and a `kid` still absent after the refetch yields `InvalidGrant` with no infinite loop.
  - *Evidence to collect:* run the negative-space test — a `kid` absent from both the cached and refetched set returns `InvalidGrant`; assert the JWKS endpoint received at most the rate-limit-permitted number of requests (`wiremock` `expect`).
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, ≥2 assertions per touched function.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-adapters -p oidc-exchange-providers` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: a rotated key validates on the next call in both providers without waiting out the TTL.**
  - *Claim:* a reviewer can run the two rotation tests and see a rotated `kid` validate without a TTL sleep.
  - *Evidence to collect:* run the `OidcProvider` and `AppleProvider` `kid`-rotation tests — expect both green and neither sleeping for the TTL.
  - *Status:* ☐ unverified

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `oidc/mod.rs::validate_id_token` for a token whose `kid` is already in the cached set → expect it still validates in the first pass with no extra refetch : ☐ (PRESERVED / REGRESSION)
- `apple.rs::validate_id_token` for an already-present `kid` → expect unchanged single-fetch validation : ☐ (PRESERVED / REGRESSION)

## Residue

- The fail-closed status check and the refetch API itself belong to Task 02; this task only consumes them at the two call sites.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
