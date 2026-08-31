# Done Certificate — Task 01: Typed email-verification form in core

**Task:** [01-email_verification_typed_form.md](01-email_verification_typed_form.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-08-31

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
  - *Status:* SATISFIED — enum at `crates/core/src/domain/provider.rs:12-23` is byte-identical to
    the change spec's Typed form block (derives, `#[default] Standard`, `TrustEmail`,
    `Claim(String)`, all variant doc comments; an added enum-level doc comment is a superset);
    field at `provider.rs:54`; re-export at `crates/core/src/domain/mod.rs:18`
    (`pub use provider::{EmailVerification, OidcProviderConfig}`); grep finds exactly one
    `EmailVerification` definition workspace-wide, so the re-export resolves to the `provider.rs`
    enum with no sibling shadow. `email_verification_defaults_to_standard` — PASS.

- **O2 — Debug output carries the mode and still redacts the secret.**
  - *Claim:* the hand-written `Debug` for `OidcProviderConfig` emits an `email_verification`
    field, and the rendered output never contains the secret sentinel.
  - *Evidence to collect:* read the `Debug` impl (`provider.rs`, formerly `:37-59`) and confirm
    the added `.field("email_verification", …)`; run the extended
    `debug_output_redacts_client_secret` (or successor) test — expect PASS with both the
    mode-presence assertion and the preserved `!rendered.contains(SECRET_SENTINEL)` negative
    assertion.
  - *Status:* SATISFIED — Debug impl adds `.field("email_verification", &self.email_verification)`
    at `provider.rs:80`; the original `debug_output_redacts_client_secret` (provider.rs:112-121)
    is preserved unmodified and PASSES; the new `debug_output_names_email_verification_mode`
    (provider.rs:139-152) asserts both `rendered.contains("email_verification: Standard")` and
    `!rendered.contains(SECRET_SENTINEL)` — PASS.

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
  - *Status:* SATISFIED — grep found exactly the seven construction sites, each setting the
    field: `provider.rs:104` (sample_config), `bootstrap.rs:1713`, `adapters/src/oidc/mod.rs:353`
    (make_config), `cross_provider_corpus.rs:206`, `upstream_error_leak_corpus.rs:63` and `:186`,
    `request_leak_oracle.rs:612`; no eighth site and no struct-update (`..`) literal that could
    omit it. Grep of `crates/adapters` and `crates/server` for `email_verification` shows only
    those constructor writes — no logic reads. `bootstrap.rs:1713` sets
    `oidc_exchange_core::domain::EmailVerification::default()` (placeholder, not a lifted
    value). `cargo nextest run --workspace` — 929 passed, 78 skipped, expectations unmodified.

- **O4 — Meets the repo definition of done.**
  - *Claim:* format, lint, and the full test suite pass.
  - *Evidence to collect:* run `cargo fmt --check --all`,
    `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect
    all clean (per [development-guidelines.md](../../../development-guidelines.md)
    §Definition of done).
  - *Status:* SATISFIED — ran in the task workspace: `cargo fmt --check --all` clean;
    `cargo clippy --workspace -- -D warnings` finished with no warnings;
    `cargo nextest run --workspace` — 929 passed, 78 skipped. Each new test carries two
    meaningful assertions.

- **O5 — Reviewable: default-mode and Debug assertions pass; every constructor sets the field (Reviewable).**
  - *Claim:* a reviewer runs the `crates/core` domain provider tests and sees the default-mode
    and Debug assertions pass, then greps the workspace for `OidcProviderConfig {` and finds
    no constructor missing the field.
  - *Evidence to collect:* run the domain provider test module and record the two passing
    assertions; run the grep and record that each hit sets `email_verification`.
  - *Status:* SATISFIED — exercised: `cargo nextest run -p oidc-exchange-core 'domain::provider'`
    → 3 passed (`email_verification_defaults_to_standard`,
    `debug_output_names_email_verification_mode`, `debug_output_redacts_client_secret`);
    `grep -rn "OidcProviderConfig {"` over the workspace returned the seven struct-literal
    sites listed under O3, every one setting `email_verification` — none missing.

## Regression check

- `provider_config_to_oidc` (`crates/server/src/bootstrap.rs`) still returns the same config
  for an unchanged provider block: re-run the existing `provider_config_to_oidc_tests` and the
  existing oidc adapter wiremock suite — expect every pre-existing expectation unchanged :
  PRESERVED — `bootstrap::provider_config_to_oidc_tests` 6/6 passed and the
  `oidc-exchange-adapters` `oidc::` suite 28/28 passed, all with unchanged expectations;
  grep confirms nothing reads `email_verification`, so every pre-existing field flows
  through unchanged.
- The `Debug` redaction path: re-run the secret-sentinel assertion — expect the secret still
  absent from output : PRESERVED — `debug_output_redacts_client_secret` passed with its
  original `!rendered.contains(SECRET_SENTINEL)` assertion intact.

## Residue

Notes for the validator: the constructor list includes three test-side sites the change spec
did not enumerate (cross_provider_corpus, upstream_error_leak_corpus, request_leak_oracle);
finding an eighth site is not a failure — it must set the field like the others.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with collected evidence (enum byte-identical to the spec block, both Debug assertions and the default-mode test pass, all seven constructors set the field with zero logic reads, fmt/clippy/nextest clean at 929 passed / 78 skipped), and both named regression paths are PRESERVED.
