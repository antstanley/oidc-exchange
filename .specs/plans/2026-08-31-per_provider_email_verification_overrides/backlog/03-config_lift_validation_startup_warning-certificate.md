# Done Certificate — Task 03: Config lift, validation, and startup warning

**Task:** [03-config_lift_validation_startup_warning.md](03-config_lift_validation_startup_warning.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-31 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Status:* ☐ unverified

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
  - *Status:* ☐ unverified

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
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* format, lint, and the full test suite pass; touched functions carry at least two
    meaningful assertions.
  - *Evidence to collect:* run `cargo fmt --check --all`,
    `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect all
    clean (per [development-guidelines.md](../../../development-guidelines.md) §Definition of
    done); read the lift function for its assertions.
  - *Status:* ☐ unverified

- **O5 — Reviewable: the Entra fixture resolves while a both-keys block fails (Reviewable).**
  - *Claim:* a reviewer runs the new bootstrap test cases and observes the Entra-shaped
    fixture resolving and a block setting both keys failing with the provider-naming
    `ConfigError`.
  - *Evidence to collect:* run the two named tests and record their PASS output; quote the
    both-keys error detail showing the provider name.
  - *Status:* ☐ unverified

## Regression check

- The pre-existing `provider_config_to_oidc_tests` (endpoint-origins lifting, caps,
  rejections) run unchanged: re-run the module — expect every pre-existing case green with
  unmodified expectations : ☐ (PRESERVED / REGRESSION)
- `build_single_provider`'s Apple arm: an `adapter = "apple"` block still builds without
  touching the new lift (the keys are inert on a foreign adapter, per the change spec's
  Decision): re-run the provider-adapter resolution tests — expect green :
  ☐ (PRESERVED / REGRESSION)
- A provider block with neither key: trace it through the lift — expect `Standard`, no
  warning, and a resolved config otherwise identical to before Task 01 :
  ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: the resolve-level Entra test exercises `resolve_config_toml` only —
it does not build the registry (registry build would perform discovery I/O). The warning's
once-per-boot property therefore rests on the O3 capture test, not on the resolve test. A `role = "admin"` deployment builds no registry, so the lift errors surface
only on token-serving roles — a known, accepted consequence recorded in the change spec.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
