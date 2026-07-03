# Done Certificate — Task 01: base_path config field

**Task:** [01-base_path_config.md](01-base_path_config.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* SATISFIED — field `pub base_path: Option<String>` present on `ServerConfig`
    (`crates/core/src/config.rs:149`; struct actually at 132-150, line-drifted from the
    certificate's cited 23-30). Positive test `server_base_path_deserializes_present_and_absent`
    (`config.rs:662-682`) asserts `base_path.as_deref() == Some("/prod")` (line 670) and, for the
    omitted key, `is_none()` (line 681). `cargo nextest run -p oidc-exchange-core config` → both
    PASS. Check: struct retains `#[serde(default)]` at `config.rs:133` (above the struct), so an
    omitted key resolves to the `Default` value — confirmed by the passing absent-key assertion.

- **O2 — Default impl returns `None`.**
  - *Claim:* `ServerConfig::default().base_path` is `None`, and the default-TOML test asserts it.
  - *Evidence to collect:* read the `Default` impl at `crates/core/src/config.rs:32-41` and confirm
    `base_path: None`; confirm `deserialize_default_toml` (`config.rs:257-299`) asserts
    `config.server.base_path.is_none()`; run that test — expect PASS.
  - *Status:* SATISFIED — `Default for ServerConfig` (`crates/core/src/config.rs:152-163`, drifted
    from cited 32-41) sets `base_path: None` at line 160. `deserialize_default_toml`
    (`config.rs:422-467`, drifted from cited 257-299) asserts `config.server.base_path.is_none()`
    at line 437. Test PASS.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and clippy clean, no new magic-number bound introduced.
  - *Evidence to collect:* run the repo's Rust gates from
    [.specs/development-guidelines.md](../../../development-guidelines.md) §Definition of done —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* SATISFIED — `cargo fmt --all --check` clean (exit 0);
    `cargo clippy --workspace --all-targets -- -D warnings` clean (CLIPPY_CLEAN);
    `cargo nextest run --workspace` → 369 passed, 27 skipped, 0 failed. No new magic-number bound
    introduced (the change adds a plain `Option<String>` field with no numeric limit).

- **O4 — Reviewable: present and absent config cases pass (Reviewable).**
  - *Claim:* a reviewer runs the config tests and sees both the `base_path = "/prod"` present case
    and the absent-key `None` case pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core config` and observe the
    present-case and absent-case tests both reported as passed.
  - *Status:* SATISFIED — ran `cargo nextest run -p oidc-exchange-core config`:
    `config::tests::server_base_path_deserializes_present_and_absent` PASS (covers both the
    `base_path = "/prod"` present case and the absent-key `None` case in one test) and
    `config::tests::deserialize_default_toml` PASS (absent-key default). 23 tests run, 23 passed.

## Regression check

- The full-config round-trip test `deserialize_full_config` (`crates/core/src/config.rs:301-429`)
  parses a `[server]` block without `base_path`: expect it still deserializes and its existing
  `server.*` assertions hold : PRESERVED — `deserialize_full_config` (now at
  `crates/core/src/config.rs:469`, drifted from cited 301-429) parses a `[server]` block with no
  `base_path` key and passed in the full-workspace run; its existing `server.*` assertions hold and
  `base_path` resolves to `None` via `#[serde(default)]`. No existing `host`/`port`/`issuer`/`role`
  deserialization path was altered.

## Residue

Notes for the validator: the field is a plain `Option<String>` with no format validation (a
leading-slash or trailing-slash normalization is deferred to Task 02's strip layer, where the
boundary rule lives). Not an obligation of Task 01.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O4 all SATISFIED with evidence — `base_path: Option<String>` on `ServerConfig` is
`#[serde(default)]`, defaults to `None`, and deserializes `Some("/prod")` when present; fmt/clippy
clean and 369 workspace tests pass; the `deserialize_full_config` regression caller is PRESERVED.
Note: certificate/task line pointers drifted (struct/Default now at config.rs:132-163, tests at
422-467 and 662-682), but the content matches — a documentation nit, not a defect.
