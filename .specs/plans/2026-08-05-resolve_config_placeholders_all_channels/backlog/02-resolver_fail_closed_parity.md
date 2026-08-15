# Task 02: Resolver fail-closed hardening and entry-point parity

**Plan:** [plan.md](../plan.md)
**Implements:** [source spec](../../../changes/2026-08-05-resolve_config_placeholders_all_channels.md) → Placeholder resolution / Implementation notes 4–6 and 8; [06-configuration.md](../../../service/specs/06-configuration.md) → future Placeholder resolution
**Depends on:** 01
**Produces:** a total, path-aware shared resolver that rejects empty, malformed, and residual unescaped placeholders and a parity table exercising all current configuration entry points.
**Pointers:** `crates/server/src/bootstrap.rs:141-237`, especially `resolve_placeholders`, `resolve_placeholders_in_str`, `scan_placeholder_name`; existing tests at `bootstrap.rs:1031-1200`.

## Steps

- [ ] Thread a stable config path through the value-tree walk, including table keys and array indices, so resolution failures name both the environment variable (where applicable) and the configuration location without exposing a resolved value.
- [ ] Reject `Ok("")` from `std::env::var` with wording distinguishable from an unset environment variable.
- [ ] Treat any unescaped `${` with no closing `}` within `PLACEHOLDER_NAME_LEN_MAX` as `ConfigError`; reject `${}` explicitly. Keep `$${` as the only literal escape and preserve its no-lookup guarantee.
- [ ] Add a post-resolution tree pass that rejects residual unescaped `${` while permitting the explicit escape result according to the documented representation; make its traversal bounded/iterative as required by the project guidelines.
- [ ] Add one parity-table test body run through `load_config_from_dir` and `parse_config`, covering set, unset, empty, escaped, unterminated, and empty-name cases. Assert equivalent success/failure semantics, relevant variable/path diagnostics, and absence of secret values.
- [ ] Cover a nested/map-valued path and at least one array path if the `config::Value` representation permits it, so path propagation is not table-only.

## Definition of done

- [ ] No unescaped `${` can reach a runtime `AppConfig`: valid names resolve to non-empty environment values, unset/empty/malformed/empty-name/residual forms return `ConfigError`, and `$${` yields literal `${` without lookup.
- [ ] Every resolver error names the config path and appropriate variable/token category but never the resolved secret; redacted `Debug` remains the only output route for secret-bearing fields.
- [ ] The same parity cases produce the same outcomes for file-backed and FFI TOML inputs; adding a future entry point has an obvious table hook.
- [ ] Existing valid file-backed resolution remains covered; malformed and empty conditions have paired negative-space tests.
- [ ] Targeted server/FFI tests, `cargo fmt --all --check`, and `cargo clippy --workspace -- -D warnings` pass. Workspace test baseline remains explicitly excluded: do not repair the three missing `providers.*.adapter` tests.
