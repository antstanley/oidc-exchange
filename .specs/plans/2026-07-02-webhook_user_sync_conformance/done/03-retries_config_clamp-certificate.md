# Done Certificate — Task 03: Clamp `[user_sync.webhook].retries` at config load

**Task:** [03-retries_config_clamp.md](03-retries_config_clamp.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** A configured `[user_sync.webhook].retries` above a named maximum of 10 is clamped to 10 at config load, logging a warning; a valid value passes through unchanged.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Must not break `WebhookConfig` deserialization (`config.rs:179-185`), its redacted-`secret` `Debug` (`config.rs:187-196`), the default of 2 when `retries` is unset (`bootstrap.rs:436`), or the `build_user_sync` webhook wiring (`bootstrap.rs:436-445`).

## Obligations

- **O1 — `retries = 20` yields effective 10; in-range and default pass through.**
  - *Claim:* the effective retries is `min(configured, MAX_WEBHOOK_RETRIES)`; `retries = 20` → 10, `retries = 5` → 5, unset → 2.
  - *Evidence to collect:* read the new clamp method on `WebhookConfig` in `crates/core/src/config.rs`. Run the new `config.rs` unit test — expect PASS asserting `20 → 10`, `5 → 5`, and the unset default `→ 2`.
  - *Checks:* resolve the clamp call site at `crates/server/src/bootstrap.rs:436` — confirm the value handed to `WebhookUserSync::new` comes through the clamp method, not the raw `wh_cfg.retries.unwrap_or(2)`.
  - *Status:* ☑ SATISFIED — `effective_retries()` (`config.rs:310-323`) returns `configured` when `configured <= MAX_WEBHOOK_RETRIES` else the max, with `configured = self.retries.unwrap_or(DEFAULT_WEBHOOK_RETRIES=2)`, i.e. `20 → 10`, `5 → 5`, unset `→ 2`. Test `effective_retries_clamps_over_max_passes_through_in_range_and_default` PASSED asserting all three. Call site now reads `let retries = wh_cfg.effective_retries();` (`bootstrap.rs:621`), not the raw `unwrap_or(2)` — resolves to the inherent method on `WebhookConfig` (no trait shadow); the clamped value is what is passed to `WebhookUserSync::new` (`bootstrap.rs:628`). (Note: certificate `:436` line numbers have drifted to `:621`; same call site.)

- **O2 — The maximum is a named constant and reducing the value logs a warning.**
  - *Claim:* the maximum is a named constant (e.g. `MAX_WEBHOOK_RETRIES = 10`), not a literal, and clamping emits a `tracing::warn!` naming the configured and clamped values.
  - *Evidence to collect:* read `config.rs` — confirm the named constant and a `tracing::warn!` in the clamp path that includes the configured and clamped values. Confirm the negative-space test exercises the out-of-range (`20`) input.
  - *Checks:* confirm the warning fires only when the configured value exceeds the maximum, not for in-range values.
  - *Status:* ☑ SATISFIED — `MAX_WEBHOOK_RETRIES: u32 = 10` is a named `pub const` (`config.rs:292`), not a literal; the clamp compares against it. The `tracing::warn!` (`config.rs:313-318`) carries `configured_retries = configured` and `clamped_retries = MAX_WEBHOOK_RETRIES` plus a message naming the max. It sits inside the `if configured > MAX_WEBHOOK_RETRIES` branch only — the `else` returns `configured` with no warning, so in-range values do not warn. Negative-space coverage: `effective_retries_clamps_...` exercises the out-of-range `20` input; `effective_retries_at_max_is_not_clamped` asserts the boundary `10` is not clamped. Both PASSED.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, the bound is a named constant, touched functions keep ≥2 meaningful assertions.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-core -p oidc-exchange-server` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` exit 0; `cargo clippy --workspace -- -D warnings` exit 0; `cargo nextest run --workspace` → 341 passed, 0 failed (superset of the two named crates). The bound is the named `pub const MAX_WEBHOOK_RETRIES`. The new `effective_retries` is covered by tests with multiple meaningful assertions (`20→10`, `5→5`, unset`→2`, boundary `10`).

- **O4 — Reviewable: `retries = 20` is reduced to 10 with a warning at load.**
  - *Claim:* a reviewer can confirm a config with `retries = 20` hands the webhook adapter an effective 10 and logs a warning.
  - *Evidence to collect:* run the config clamp test and read the `bootstrap.rs:436` call site; observe the clamp is applied before `WebhookUserSync::new` and that a warning is logged for the out-of-range value.
  - *Status:* ☑ SATISFIED — the config clamp test PASSED (`retries = 20 → 10`). Read the call site (`bootstrap.rs:621`, drifted from `:436`): `let retries = wh_cfg.effective_retries();` runs before `WebhookUserSync::new(...)` (`bootstrap.rs:624-628`), so the adapter can only receive a value `<= MAX_WEBHOOK_RETRIES`. The clamp path emits the `tracing::warn!` for the out-of-range input. A reviewer can reproduce with `cargo nextest run -p oidc-exchange-core effective_retries`.

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `build_user_sync` still constructs a `WebhookUserSync` with the configured URL/secret/timeout and now a clamped `retries` → expect the existing config deserialization tests (`config.rs` webhook tests, `bootstrap` wiring) still pass : ☑ PRESERVED — `WebhookConfig` fields and `#[derive(Deserialize)]` are unchanged (`config.rs:297-303`); the redacted-`secret` `Debug` impl still emits `"<redacted>"` (`config.rs:326-334`); the unset default remains `2` via `DEFAULT_WEBHOOK_RETRIES`. `build_user_sync` passes url/secret/timeout as before and only swaps the raw `unwrap_or(2)` for `effective_retries()`. Full `cargo nextest run --workspace` = 341 passed, 0 failed.

## Residue

- The per-attempt adapter cap (task 02) is a separate, complementary bound; this task only clamps the configured count. Not an obligation here.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O4 all SATISFIED with evidence — `effective_retries()` clamps `20 → 10`, passes `5` and the unset default `2` through, warns (naming configured/clamped) only above the named `MAX_WEBHOOK_RETRIES = 10`, and is the value wired into `WebhookUserSync::new` at `bootstrap.rs:621`; fmt/clippy clean and the full 341-test suite passes with the `build_user_sync`/deserialization/redacted-`Debug` regression surface PRESERVED.
