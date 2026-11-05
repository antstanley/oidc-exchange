# Done Certificate — Task 03: Config lift, validation, and startup warning

**Task:** [03-config_lift_validation_startup_warning.md](03-config_lift_validation_startup_warning.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-08-31

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The two provider keys lift to their `EmailVerification` variants in
  `provider_config_to_oidc` with fail-closed validation, a non-Standard provider logs exactly
  one structured warning at registry build, and an Entra-shaped block resolves through
  `resolve_config_toml`.
- **P2 — Obligations.** Done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD
  order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not change the lift or validation of the existing keys
  (`client_id`, `client_secret`, `scopes`, `endpoint_origins`), must not add validation in
  `Config::resolve` (the keys are registry-build concerns, per the change spec's Decision),
  and a block with neither key must produce `Standard` with no warning.

## Obligations

- **O1 — Lift matrix tested.**
  - *Claim:* `email_verified_claim = "x"` lifts to `Claim("x")`; `trust_email_verified = true`
    lifts to `TrustEmail`; an explicit `trust_email_verified = false` and an absent key both
    lift to `Standard`; the Entra-shaped block (`adapter = "oidc"`, a
    `login.microsoftonline.com/…/v2.0` issuer, `client_id`, `scopes`,
    `email_verified_claim = "xms_edov"`) resolves through `resolve_config_toml`.
  - *Evidence to collect:* run the new cases beside `provider_config_to_oidc_tests`
    (`crates/server/src/bootstrap.rs`) — expect each lifted variant asserted by equality; run
    the Entra resolve test — expect `Ok` and the provider present in the resolved config.
  - *Checks:* confirm the lift matches on key presence first (`config.extra.get(...)`), then
    errors when the present value's `as_str`/`as_bool` fails — not the
    silently-absence-mapping `get_str` pattern used for `client_secret`, which maps a
    set-but-non-string value to `None` and cannot distinguish it from an absent key.
  - *Status:* ☑ SATISFIED — `email_verified_claim_lifts_to_the_claim_variant`,
    `trust_email_verified_true_lifts_to_trust_email`,
    `absent_keys_and_explicit_false_trust_both_lift_to_standard`, and
    `entra_shaped_block_with_mapped_claim_resolves` all PASS (nextest, task workspace); each
    variant asserted by equality, Entra resolve returns `Ok` with the provider present and
    lifting to `Claim("xms_edov")`. Check: the lift reads key presence first
    (`extra.get(...)` at bootstrap.rs:1672-1673, both-set rejected at :1675 before any type
    read), then errors when a present value's `as_str`/`as_bool` fails (:1685, :1701) — the
    `get_str` closure (:1739) is not used for either key.

- **O2 — Negative space and the named, code-point-counted cap.**
  - *Claim:* both keys set, a set-but-non-boolean `trust_email_verified`, and a non-string /
    empty / over-cap `email_verified_claim` each fail with `Error::ConfigError` naming the
    provider; the cap is a named constant; a 64-code-point name is accepted and a
    65-code-point name rejected, counted in code points not bytes.
  - *Evidence to collect:* run the five rejection cases — expect each `ConfigError` detail to
    contain the provider name; read the constant declaration (e.g.
    `MAX_EMAIL_VERIFIED_CLAIM_LEN`) and grep the lift for a literal `64` — expect only the
    named constant; run the boundary pair (64 accepted / 65 rejected) and the multi-byte case
    (a 64-code-point name whose byte length exceeds 64, accepted) — expect all three PASS.
  - *Checks:* trace the length check — confirm it uses `chars().count()` (or an equivalent
    code-point count), not `len()`; confirm the over-cap error message does not embed the
    oversized name itself (the endpoint-origins discipline).
  - *Status:* ☑ SATISFIED — the five rejection shapes PASS
    (`both_email_verification_keys_set_is_rejected_naming_the_provider` over true and false,
    `non_boolean_trust_email_verified_is_rejected_never_coerced` over `"true"` and `1`,
    `invalid_email_verified_claim_values_are_rejected_naming_the_provider` over integer /
    boolean / empty / over-cap), each asserting the `ConfigError` detail contains the
    provider name. `MAX_EMAIL_VERIFIED_CLAIM_LEN: usize = 64` declared at bootstrap.rs:1656;
    the only literal `64` in the lift region is that declaration (tests use the constant).
    Boundary pair `claim_name_exactly_at_the_cap_is_accepted_and_one_over_rejected` and
    multi-byte `claim_name_cap_counts_code_points_not_bytes` (é×64 = 128 bytes, 64 code
    points, accepted) both PASS. Check: the length test is `claim.chars().count() >
    MAX_EMAIL_VERIFIED_CLAIM_LEN` (:1709), not `len()`; the over-cap message (:1713-1716)
    embeds only the provider name and the constant, never the oversized value.

- **O3 — One structured warning per non-Standard provider; none for Standard.**
  - *Claim:* at registry build, a provider whose resolved mode is `TrustEmail` or `Claim(...)`
    emits exactly one structured `tracing::warn!` naming the provider id and the mode (and
    the mapped claim name for `Claim`); a `Standard` provider emits none.
  - *Evidence to collect:* read the warning call site in `build_single_provider`
    (`crates/server/src/bootstrap.rs`, beside the existing structured-warning precedent at
    `:566-590`) and confirm its fields (provider id, mode, claim name), that it sits on the
    once-per-provider path outside any loop, and that it is emitted from a unit-testable
    position — either a pure helper (e.g. `fn email_verification_warning(mode) ->
    Option<…>`) or immediately after the synchronous `provider_config_to_oidc` call, before
    the async `from_config().await`; run the warning test — expect exactly one warning for
    a `Claim` fixture and for a `TrustEmail` fixture, and zero warnings for a `Standard`
    fixture, captured via `tracing::subscriber::with_default` with a capturing writer (or
    `install_span_capture` from the test-utils crate) in the `bootstrap.rs` `#[cfg(test)]`
    module, or via an integration test on the existing wiremock-discovery pattern.
  - *Checks:* confirm the warning fires after a successful lift (a mistyped key errors and
    never warns) and is keyed on the resolved `EmailVerification`, not on raw key presence;
    confirm the test asserts the warning count (exactly one), not mere presence.
  - *Status:* ☑ SATISFIED — call site at bootstrap.rs:1602 in `build_single_provider`,
    immediately after the synchronous `provider_config_to_oidc` call (:1597) and before the
    async `from_config().await` (:1604), outside any loop (once per provider via the
    `build_providers` loop). The helper `warn_nonstandard_email_verification` (:1623) emits
    `provider`, `mode`, and (for `Claim`) `claim` fields; the `Standard` arm emits nothing.
    Tests use `install_span_capture` in the `bootstrap.rs` `#[cfg(test)]` module:
    `claim_mode_logs_exactly_one_warning_naming_provider_and_claim`,
    `trust_email_mode_logs_exactly_one_warning_naming_provider_and_mode` (both
    `assert_eq!(warn_line_count, 1)` plus provider/mode/claim containment), and
    `standard_mode_logs_no_warning` (`assert_eq!(count, 0)` and no telemetry at all) — all
    PASS. Checks: a mistyped key makes `?` at :1597 return before :1602, so an invalid
    config never warns; the helper is keyed on the resolved
    `oidc_config.email_verification`, not raw key presence; counts are exact, not presence.

- **O4 — Meets the repo definition of done.**
  - *Claim:* format, lint, and the full test suite pass; touched functions carry at least two
    meaningful assertions.
  - *Evidence to collect:* run `cargo fmt --check --all`,
    `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect all
    clean (per [development-guidelines.md](../../../development-guidelines.md) §Definition of
    done); read the lift function for its assertions.
  - *Status:* ☑ SATISFIED — in the task workspace: `cargo fmt --check --all` exit 0;
    `cargo clippy --workspace -- -D warnings` exit 0 (also clean with `--all-targets`);
    `cargo nextest run --workspace` → 951 passed, 0 failed, 78 skipped. Both new functions
    are under the 70-line gate (`warn_nonstandard_email_verification` 29 lines,
    `lift_email_verification` 55 lines); rustfmt-clean at 100 columns; every new test
    carries at least two meaningful assertions and each touched production function is
    covered by multi-assertion tests.

- **O5 — Reviewable: the Entra fixture resolves while a both-keys block fails (Reviewable).**
  - *Claim:* a reviewer runs the new bootstrap test cases and observes the Entra-shaped
    fixture resolving and a block setting both keys failing with the provider-naming
    `ConfigError`.
  - *Evidence to collect:* run the two named tests and record their PASS output; quote the
    both-keys error detail showing the provider name.
  - *Status:* ☑ SATISFIED — `bootstrap::email_verification_boot_tests::
    entra_shaped_block_with_mapped_claim_resolves` PASS (the Entra fixture resolves and
    lifts to `Claim("xms_edov")` with scopes intact) and `bootstrap::
    provider_config_to_oidc_tests::both_email_verification_keys_set_is_rejected_naming_the_provider`
    PASS. The both-keys detail (format at bootstrap.rs:1677-1680, interpolating only the
    provider name; containment asserted by the passing test): `provider 'google':
    'email_verified_claim' and 'trust_email_verified' are mutually exclusive; set at most
    one`.

## Regression check

- The pre-existing `provider_config_to_oidc_tests` (endpoint-origins lifting, caps,
  rejections) run unchanged: re-run the module — expect every pre-existing case green with
  unmodified expectations : PRESERVED — all 6 pre-existing cases PASS; the diff leaves
  their bodies untouched (only the module doc header and a new import changed).
- `build_single_provider`'s Apple arm: an `adapter = "apple"` block still builds without
  touching the new lift (the keys are inert on a foreign adapter, per the change spec's
  Decision): re-run the provider-adapter resolution tests — expect green :
  PRESERVED — `provider_adapter_resolution_tests` 4/4 PASS; the Apple arm (:1607-1611)
  calls `AppleProvider::from_config(&config.extra)` and never reaches the lift.
- A provider block with neither key: trace it through the lift — expect `Standard`, no
  warning, and a resolved config otherwise identical to before Task 01 :
  PRESERVED — trace: both `extra.get` calls return `None` → both-set check false → trust
  branch skipped → `let Some(...) else` returns `Standard` (:1697-1700); the warn helper's
  `Standard` arm emits nothing (:1630); the replaced placeholder was
  `EmailVerification::default()` = `Standard`, so the resolved config is bit-identical.
  Pinned by `absent_keys_and_explicit_false_trust_both_lift_to_standard` and
  `standard_mode_logs_no_warning`, both PASS.

## Residue

Notes for the validator: the resolve-level Entra test exercises `resolve_config_toml` only —
it does not build the registry (registry build would perform discovery I/O). The warning's
once-per-boot property therefore rests on the O3 capture test, not on the resolve test. A `role = "admin"` deployment builds no registry, so the lift errors surface
only on token-serving roles — a known, accepted consequence recorded in the change spec.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with run evidence (18 targeted tests PASS, fmt/clippy/nextest
clean at 951 workspace tests) and all three named regression surfaces PRESERVED, so the
rubric derives DONE with nothing left UNVERIFIED.
