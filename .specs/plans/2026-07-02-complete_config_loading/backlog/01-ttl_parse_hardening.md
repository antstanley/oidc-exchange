# Task 01 — TTL parse hardening

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-ttl_parse_hardening-certificate.md](01-ttl_parse_hardening-certificate.md)

**Implements:** [06-configuration.md](../../../service/specs/06-configuration.md) → Validation at load (the `token.access_token_ttl` / `refresh_token_ttl` rule: parse as `<integer><s|m|h|d>` without overflow)
**Depends on:** —
**Produces:** `parse_duration_secs` that cannot panic on any input and cannot overflow silently — malformed or overflowing durations return `ConfigError`, well-formed ones return the exact second count.
**Pointers:** `crates/core/src/service/mod.rs:168-190` (`parse_duration_secs`; the `split_at(s.len() - 1)` panic on a multi-byte final char at :176 and the unchecked multiplications at :183-185)

## Steps

- [ ] Replace the byte-index `split_at(s.len() - 1)` with a char-boundary split (e.g. split the last `char` off, or partition on the first non-digit) so a multi-byte final character cannot panic.
- [ ] Replace the `value * 60` / `* 3600` / `* 86400` multiplications with `checked_mul`, returning `ConfigError` on overflow instead of wrapping/panicking.
- [ ] Introduce named constants for the seconds-per-unit multipliers (minute/hour/day) in the module, per the limits discipline.
- [ ] Keep the existing empty-string and unknown-suffix `ConfigError` branches; ensure the error `detail` names the offending input.
- [ ] Add unit tests covering: valid `"15m"`/`"30d"`/`"1h"`/`"45s"`; a multi-byte final char (e.g. `"15€"`); an overflowing value (e.g. a `u64::MAX`-scale day count); empty string; unknown suffix.

## Definition of done

- [ ] `parse_duration_secs` returns the correct second count for each valid unit and a `ConfigError` (never a panic) for multi-byte, empty, unknown-suffix, and overflowing input.
- [ ] Negative-space tests exist for the multi-byte-final-char and overflow paths that previously panicked/wrapped.
- [ ] The unit multipliers are named constants; the touched function carries at least two meaningful assertions (or its tests do, per repo convention).
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [ ] Reviewable: run `cargo nextest run -p oidc-exchange-core parse_duration` and confirm the multi-byte and overflow cases return `Err`, not a panic.
