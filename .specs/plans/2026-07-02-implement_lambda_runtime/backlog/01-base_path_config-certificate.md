# Done Certificate — Task 01: base_path config field

**Task:** [01-base_path_config.md](01-base_path_config.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location or a test result) — not by assertion.

## Premises

- **P1 — Goal.** The task makes the `server.base_path` TOML key deserialize into `ServerConfig`
  as `Option<String>`, defaulting to `None` when absent — the data Task 02's strip layer consumes.
- **P2 — Obligations.** The task is done iff O1…O4 all hold. One Oi per definition-of-done item,
  in DoD order; O4 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the existing `ServerConfig` deserialization
  (`host`/`port`/`issuer`/`role`) or the two config round-trip tests at
  `crates/core/src/config.rs:257-429`; the struct stays `#[serde(default)]`.

## Obligations

- **O1 — Present and absent cases deserialize correctly.**
  - *Claim:* `server.base_path = "/prod"` deserializes to `Some("/prod")`, and a config omitting
    the key yields `None`.
  - *Evidence to collect:* read `crates/core/src/config.rs:23-30` and confirm the field
    `pub base_path: Option<String>` on `ServerConfig`; read the new positive test and confirm it
    asserts `config.server.base_path.as_deref() == Some("/prod")`; run
    `cargo nextest run -p oidc-exchange-core config` — expect both the present-key and absent-key
    assertions PASS.
  - *Checks:* confirm the struct retains `#[serde(default)]` at `config.rs:24` so an omitted key
    resolves to the `Default` value, not a deserialization error.
  - *Status:* ☐ unverified

- **O2 — Default impl returns `None`.**
  - *Claim:* `ServerConfig::default().base_path` is `None`, and the default-TOML test asserts it.
  - *Evidence to collect:* read the `Default` impl at `crates/core/src/config.rs:32-41` and confirm
    `base_path: None`; confirm `deserialize_default_toml` (`config.rs:257-299`) asserts
    `config.server.base_path.is_none()`; run that test — expect PASS.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and clippy clean, no new magic-number bound introduced.
  - *Evidence to collect:* run the repo's Rust gates from
    [.specs/development-guidelines.md](../../../development-guidelines.md) §Definition of done —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O4 — Reviewable: present and absent config cases pass (Reviewable).**
  - *Claim:* a reviewer runs the config tests and sees both the `base_path = "/prod"` present case
    and the absent-key `None` case pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core config` and observe the
    present-case and absent-case tests both reported as passed.
  - *Status:* ☐ unverified

## Regression check

- The full-config round-trip test `deserialize_full_config` (`crates/core/src/config.rs:301-429`)
  parses a `[server]` block without `base_path`: expect it still deserializes and its existing
  `server.*` assertions hold : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: the field is a plain `Option<String>` with no format validation (a
leading-slash or trailing-slash normalization is deferred to Task 02's strip layer, where the
boundary rule lives). Not an obligation of Task 01.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
