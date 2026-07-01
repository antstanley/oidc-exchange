# Done Certificate — Task 03: Overlay and env overrides

**Task:** [03-overlay_and_env_overrides.md](03-overlay_and_env_overrides.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** `load_config` deep-merges the env-specific TOML over the default (tables recurse, scalars/arrays replace) and applies `OIDC_EXCHANGE__…` env overrides down to nested and map-valued keys.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the `Reviewable:` item.
- **P3 — Invariants.** The existing fallbacks in `load_config` (`crates/server/src/bootstrap.rs:26-47`) — missing default file, missing/empty env file, no files at all → `AppConfig::default()` — still hold.

## Obligations

- **O1 — Overlay merges, does not wholesale-replace.**
  - *Claim:* a value present only in `config/default.toml` survives when an env overlay sets a *different* key; a key set in both takes the env value.
  - *Evidence to collect:* read the rewritten `load_config`; confirm the env TOML is layered *over* the default (not the `if env_config.is_empty() { default } else { env }` replacement it has today). Run the overlay test — expect the default-only key present and the shared key at the env value.
  - *Checks:* confirm the merge recurses into tables rather than replacing a whole section when only one key changes.
  - *Status:* ☐ unverified

- **O2 — Env overrides reach nested and map-valued paths.**
  - *Claim:* `OIDC_EXCHANGE__PROVIDERS__GOOGLE__CLIENT_ID` sets `providers.google.client_id` and `OIDC_EXCHANGE__SERVER__PORT` sets `server.port`.
  - *Evidence to collect:* read the `Environment` source setup (prefix `OIDC_EXCHANGE`, separator `__`); run the override tests with those vars exported and assert the resulting `AppConfig` fields.
  - *Checks:* confirm segments are lowercased and `__` is the separator so a nested provider key resolves.
  - *Status:* ☐ unverified

- **O3 — Missing-files fallback and single-underscore addressability.**
  - *Claim:* missing files fall back to defaults without error; a single-underscore segment (`my_idp`) is addressed as one segment, not split.
  - *Evidence to collect:* run the missing-files test (expect `AppConfig::default()`, no error) and the `my_idp` test (an `OIDC_EXCHANGE__PROVIDERS__MY_IDP__…` override lands on `providers.my_idp`).
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: merged config carries both default and overlaid/overridden values.**
  - *Claim:* with a default TOML, an env overlay, and an `OIDC_EXCHANGE__…` var exported, the loaded `AppConfig` shows the un-overridden default value plus the overlaid and overridden values.
  - *Evidence to collect:* run the integration-style test (or a manual `load_config` invocation under a temp config dir) and assert all three value sources are reflected.
  - *Status:* ☐ unverified

## Regression check

- Callers of `load_config` (the server `main`/bootstrap entry) still receive a valid `AppConfig` when no `OIDC_EXCHANGE_ENV` and no override vars are set → expect the committed default config : ☐ (PRESERVED / REGRESSION)

## Residue

- Whether the `config` crate `0.15` deep-merges tables as required or needs a `toml::Value` recursive merge is an Open question in the task; the choice is a note for the validator, not a separate obligation.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
