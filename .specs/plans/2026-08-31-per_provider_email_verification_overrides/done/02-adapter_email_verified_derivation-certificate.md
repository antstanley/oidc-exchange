# Done Certificate — Task 02: Adapter derivation with explicit-claim precedence

**Task:** [02-adapter_email_verified_derivation.md](02-adapter_email_verified_derivation.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-08-31

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
  - *Status:* SATISFIED — `claim_mode_explicit_false_beats_true_override` PASS
    (`IdentityClaims.email_verified == Some(false)`). Trace: `derive_email_verified`
    (mod.rs:140–172) returns from `coerce_bool(&claims["email_verified"])` at line 143–145
    before the `match mode` at 149. Checks: `coerce_bool` at both call sites resolves via
    `use crate::shared::claims::coerce_bool` (mod.rs:9 → shared/claims.rs:14), no local
    re-implementation; line 238 consults `&self.email_verification`, set only at
    from_config mod.rs:125 from `config.email_verification.clone()` — no default in
    `validate_id_token`.

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
  - *Status:* SATISFIED — all ten PASS (`cargo nextest run -p oidc-exchange-adapters -E
    'test(claim_mode_) or test(trust_email_mode_) or test(standard_mode_absent_claim_stays_none)'`
    → 10 passed): `claim_mode_derives_true_from_json_bool_override`,
    `claim_mode_coerces_string_true_override`, `claim_mode_absent_override_stays_none`,
    `claim_mode_non_coercible_override_stays_none` (asserts `email_verified == None` for
    `xms_edov: 42` while `raw_claims` still carries the refused value),
    `claim_mode_explicit_false_beats_true_override`, `trust_email_mode_present_email_derives_true`,
    `trust_email_mode_absent_email_stays_none`, `trust_email_mode_empty_email_stays_none`,
    `trust_email_mode_explicit_false_is_never_overturned`, `standard_mode_absent_claim_stays_none`.
    Check: the TrustEmail guard (mod.rs:152–155) is `claims["email"].as_str()` +
    `!email.is_empty()`, so numeric/null `email` hits the `_` arm → `None`.

- **O3 — Standard is byte-identical to 0.4.0; Apple untouched.**
  - *Claim:* the named `Standard` pin passes, no pre-existing adapter test changed its
    expectation, and `crates/providers/src/apple.rs` has no diff.
  - *Evidence to collect:* run the `Standard`-absent pin — expect `None`; diff the task's
    change set — expect `crates/providers/src/apple.rs` absent from it and no modified
    expectation in pre-existing oidc tests (fixture-plumbing edits to `make_config` callers
    are acceptable; changed assertions are not).
  - *Status:* SATISFIED — `standard_mode_absent_claim_stays_none` PASS (`email_verified ==
    None` with `email` and an unconfigured `xms_edov: true` present). `jj diff --stat` shows
    the change set is exactly one file, `crates/adapters/src/oidc/mod.rs` (+287/−3) —
    `crates/providers/src/apple.rs` absent. The only deletions are the import line, the old
    derivation line in `validate_id_token`, and `make_config`'s hardcoded
    `EmailVerification::default()` (fixture plumbing: body now delegates to
    `make_config_with_mode` with the same default); no pre-existing assertion changed.
    Under `Standard`, step 1 is the identical `coerce_bool(&claims["email_verified"])` call
    and step 2 maps its `None` to `None` — extensionally identical to 0.4.0 for all inputs.

- **O4 — Meets the repo definition of done.**
  - *Claim:* format, lint, and the full test suite pass, and the touched functions carry at
    least two meaningful assertions.
  - *Evidence to collect:* run `cargo fmt --check --all`,
    `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect all
    clean; read the derivation helper and count its assertions (e.g. explicit-passthrough and
    Standard-never-fabricates) per
    [development-guidelines.md](../../../development-guidelines.md) §Definition of done.
  - *Status:* SATISFIED — `cargo fmt --check --all` clean; `cargo clippy --workspace -- -D
    warnings` clean; `cargo nextest run --workspace` → 939 passed, 0 failed (no LEAK on
    this run). `derive_email_verified` carries two meaningful `debug_assert!`s
    (mod.rs:160–163 explicit-passthrough / step-2-only-on-absence; mod.rs:166–169
    Standard-never-fabricates); every one of the ten new tests carries two assertions;
    `derive_email_verified` is 33 lines and `validate_with_mode` 49 lines (limit 70);
    rustfmt (default max_width 100) clean covers the column limit.

- **O5 — Reviewable: the false-beside-`xms_edov` case proves overrides never overturn (Reviewable).**
  - *Claim:* a reviewer runs the oidc adapter wiremock suite and reads the
    false-beside-`xms_edov: true` case yielding `Some(false)` as the demonstration that an
    explicit provider signal cannot be overturned by configuration.
  - *Evidence to collect:* run the suite, record the case's PASS, and quote its assertion
    line.
  - *Status:* SATISFIED — ran the oidc adapter wiremock suite (`-E 'test(oidc::tests::)'`,
    38 passed); `claim_mode_explicit_false_beats_true_override` PASS. Its assertion
    (mod.rs:1805–1809):
    `assert_eq!(identity.email_verified, Some(false), "an explicit provider signal must
    never be overturned by configuration");` — read alongside `raw_claims["xms_edov"] ==
    json!(true)` proving the losing override really was in the token.

## Regression check

- `crates/core/src/service/exchange.rs` (`registration_policy_reason` and its three call
  sites) reads `IdentityClaims.email_verified` unchanged: run the core registration-policy
  tests in `crates/core/tests/exchange.rs` — expect them green with no modification :
  PRESERVED — `cargo nextest run -p oidc-exchange-core --test exchange` → 26 passed, file
  untouched by the diff; `registration_policy_reason` (exchange.rs:96) and its call sites
  (exchange.rs:331, 349, 425) read `claims.email_verified` exactly as before, and every
  existing config carries the default `Standard` mode, whose derivation is extensionally
  identical to 0.4.0.
- Existing oidc adapter tests (signature/iss/aud/nbf validation, subject extraction,
  signing_alg): re-run the pre-existing module — expect every case green with unchanged
  expectations : PRESERVED — full `oidc::tests` module → 38 passed (28 pre-existing + 10
  new), no pre-existing expectation modified in the diff; workspace total 939 passed
  (929 baseline + 10 new).

## Residue

Notes for the validator: the derivation reads `claims[name]` for an operator-configured name;
the name's length/content validation is Task 03's obligation, not this one's — at this stage
the mode can only be constructed programmatically in tests.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with run evidence (ten matrix cases green, precedence proof
observed, fmt/clippy/939-test workspace suite clean, apple.rs absent from the single-file
diff) and both named regression surfaces PRESERVED (core exchange 26/26, oidc module 38/38
with unchanged pre-existing expectations).
