# Done Certificate — Task 02: Adapter derivation with explicit-claim precedence

**Task:** [02-adapter_email_verified_derivation.md](02-adapter_email_verified_derivation.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-31 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `validate_id_token` derives `email_verified` per the provider's configured
  mode under the two-step precedence rule, pinned by a ten-case wiremock matrix, with
  `Standard` byte-identical to 0.4.0.
- **P2 — Obligations.** Done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD
  order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not change any other field of `IdentityClaims` (subject, email,
  name, is_private_email, signing_alg, raw_claims), must not touch
  `crates/providers/src/apple.rs`, and must not alter `registration_policy_reason` or its
  call sites in `crates/core/src/service/exchange.rs`.

## Obligations

- **O1 — Precedence holds in both directions.**
  - *Claim:* an explicit `email_verified` claim always passes through: with mode
    `Claim("xms_edov")`, a token carrying `email_verified: false` beside `xms_edov: true`
    yields `Some(false)`; an explicit `true` is returned from step 1 without consulting the
    mode.
  - *Evidence to collect:* run the wiremock case for `email_verified: false` beside
    `xms_edov: true` — expect the returned `IdentityClaims.email_verified == Some(false)`.
    Trace the derivation helper in `crates/adapters/src/oidc/mod.rs`: confirm the
    `coerce_bool(&claims["email_verified"])` result short-circuits before any mode match.
  - *Checks:* resolve `coerce_bool` at both call sites — confirm it is
    `crate::shared::claims::coerce_bool` (`crates/adapters/src/shared/claims.rs`), not a
    local re-implementation; confirm the mode consulted is the one copied from
    `OidcProviderConfig.email_verification` in `from_config`, not a default constructed in
    `validate_id_token`.
  - *Status:* ☐ unverified

- **O2 — All ten wiremock cases pass.**
  - *Claim:* the matrix passes — `Claim("xms_edov")`: JSON `true` → `Some(true)`, string
    `"true"` → `Some(true)`, absent → `None`, `xms_edov: 42` (a JSON number, neither boolean
    nor `"true"`/`"false"`) → `None`, explicit-false-beside-true → `Some(false)`;
    `TrustEmail`: email present → `Some(true)`, email absent → `None`, empty-string email →
    `None`, explicit `email_verified: false` → `Some(false)`; `Standard`: absent → `None`.
  - *Evidence to collect:* run the oidc adapter test module and list the ten named cases with
    their PASS results; confirm the coercion negative-space case (`xms_edov: 42` under
    `Claim("xms_edov")`) asserts `email_verified == None` through `coerce_bool`.
  - *Checks:* for the `TrustEmail` empty-string case, trace the guard — confirm it requires a
    non-empty *string* (`as_str` + emptiness), so a numeric or null `email` also yields `None`.
  - *Status:* ☐ unverified

- **O3 — Standard is byte-identical to 0.4.0; Apple untouched.**
  - *Claim:* the named `Standard` pin passes, no pre-existing adapter test changed its
    expectation, and `crates/providers/src/apple.rs` has no diff.
  - *Evidence to collect:* run the `Standard`-absent pin — expect `None`; diff the task's
    change set — expect `crates/providers/src/apple.rs` absent from it and no modified
    expectation in pre-existing oidc tests (fixture-plumbing edits to `make_config` callers
    are acceptable; changed assertions are not).
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* format, lint, and the full test suite pass, and the touched functions carry at
    least two meaningful assertions.
  - *Evidence to collect:* run `cargo fmt --check --all`,
    `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect all
    clean; read the derivation helper and count its assertions (e.g. explicit-passthrough and
    Standard-never-fabricates) per
    [development-guidelines.md](../../../development-guidelines.md) §Definition of done.
  - *Status:* ☐ unverified

- **O5 — Reviewable: the false-beside-`xms_edov` case proves overrides never overturn (Reviewable).**
  - *Claim:* a reviewer runs the oidc adapter wiremock suite and reads the
    false-beside-`xms_edov: true` case yielding `Some(false)` as the demonstration that an
    explicit provider signal cannot be overturned by configuration.
  - *Evidence to collect:* run the suite, record the case's PASS, and quote its assertion
    line.
  - *Status:* ☐ unverified

## Regression check

- `crates/core/src/service/exchange.rs` (`registration_policy_reason` and its three call
  sites) reads `IdentityClaims.email_verified` unchanged: run the core registration-policy
  tests in `crates/core/tests/exchange.rs` — expect them green with no modification :
  ☐ (PRESERVED / REGRESSION)
- Existing oidc adapter tests (signature/iss/aud/nbf validation, subject extraction,
  signing_alg): re-run the pre-existing module — expect every case green with unchanged
  expectations : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: the derivation reads `claims[name]` for an operator-configured name;
the name's length/content validation is Task 03's obligation, not this one's — at this stage
the mode can only be constructed programmatically in tests.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
