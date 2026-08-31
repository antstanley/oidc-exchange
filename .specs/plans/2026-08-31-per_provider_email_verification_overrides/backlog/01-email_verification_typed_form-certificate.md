# Done Certificate — Task 01: Typed email-verification form in core

**Task:** [01-email_verification_typed_form.md](01-email_verification_typed_form.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-31 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `EmailVerification` (default `Standard`) and `OidcProviderConfig.email_verification`
  exist, re-exported, set in every struct-literal constructor and emitted by the hand-written
  Debug, with the workspace green and behaviour byte-identical to before.
- **P2 — Obligations.** Done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD
  order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not change any runtime behaviour (no adapter, bootstrap, or core
  logic reads the new field in this task), and must not weaken the secret redaction of
  `OidcProviderConfig`'s hand-written `Debug`.

## Obligations

- **O1 — Enum and field compile everywhere; default mode pinned by test.**
  - *Claim:* `EmailVerification` exists in `crates/core/src/domain/provider.rs` with derives
    `Debug, Clone, Default, PartialEq, Eq`, variants `Standard` (marked `#[default]`),
    `TrustEmail`, `Claim(String)`; `OidcProviderConfig` carries
    `pub email_verification: EmailVerification`; a unit test asserts
    `EmailVerification::default() == EmailVerification::Standard`.
  - *Evidence to collect:* read the enum and field in `crates/core/src/domain/provider.rs` and
    diff them against the change spec's Typed form block; confirm the re-export in
    `crates/core/src/domain/mod.rs` (beside `OidcProviderConfig`); run the new default-mode
    unit test — expect PASS.
  - *Checks:* resolve the re-exported name — confirm `oidc_exchange_core::domain::EmailVerification`
    resolves to the `provider.rs` enum, not a new sibling module.
  - *Status:* ☐ unverified

- **O2 — Debug output carries the mode and still redacts the secret.**
  - *Claim:* the hand-written `Debug` for `OidcProviderConfig` emits an `email_verification`
    field, and the rendered output never contains the secret sentinel.
  - *Evidence to collect:* read the `Debug` impl (`provider.rs`, formerly `:37-59`) and confirm
    the added `.field("email_verification", …)`; run the extended
    `debug_output_redacts_client_secret` (or successor) test — expect PASS with both the
    mode-presence assertion and the preserved `!rendered.contains(SECRET_SENTINEL)` negative
    assertion.
  - *Status:* ☐ unverified

- **O3 — No behavioural change; every constructor updated.**
  - *Claim:* nothing reads the field yet, and every `OidcProviderConfig { … }` struct literal
    in the workspace sets it (to `EmailVerification::default()` or `Standard`).
  - *Evidence to collect:* grep the workspace for `OidcProviderConfig {` and confirm all seven
    construction sites set the field: `provider.rs` `sample_config`,
    `crates/server/src/bootstrap.rs` (`provider_config_to_oidc`),
    `crates/adapters/src/oidc/mod.rs` (`make_config`),
    `crates/providers/tests/cross_provider_corpus.rs`,
    `crates/providers/tests/upstream_error_leak_corpus.rs` (two sites),
    `crates/server/tests/request_leak_oracle.rs`. Grep `crates/adapters` and `crates/server`
    for reads of `email_verification` — expect only constructor writes, no logic reads. Run
    `cargo nextest run --workspace` — expect the pre-existing suite green with unmodified
    expectations.
  - *Checks:* confirm the `bootstrap.rs` site sets `EmailVerification::default()` (the task-03
    placeholder), not a lifted value — the lift belongs to Task 03.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* format, lint, and the full test suite pass.
  - *Evidence to collect:* run `cargo fmt --check --all`,
    `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect
    all clean (per [development-guidelines.md](../../../development-guidelines.md)
    §Definition of done).
  - *Status:* ☐ unverified

- **O5 — Reviewable: default-mode and Debug assertions pass; every constructor sets the field (Reviewable).**
  - *Claim:* a reviewer runs the `crates/core` domain provider tests and sees the default-mode
    and Debug assertions pass, then greps the workspace for `OidcProviderConfig {` and finds
    no constructor missing the field.
  - *Evidence to collect:* run the domain provider test module and record the two passing
    assertions; run the grep and record that each hit sets `email_verification`.
  - *Status:* ☐ unverified

## Regression check

- `provider_config_to_oidc` (`crates/server/src/bootstrap.rs`) still returns the same config
  for an unchanged provider block: re-run the existing `provider_config_to_oidc_tests` and the
  existing oidc adapter wiremock suite — expect every pre-existing expectation unchanged :
  ☐ (PRESERVED / REGRESSION)
- The `Debug` redaction path: re-run the secret-sentinel assertion — expect the secret still
  absent from output : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: the constructor list includes three test-side sites the change spec
did not enumerate (cross_provider_corpus, upstream_error_leak_corpus, request_leak_oracle);
finding an eighth site is not a failure — it must set the field like the others.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
