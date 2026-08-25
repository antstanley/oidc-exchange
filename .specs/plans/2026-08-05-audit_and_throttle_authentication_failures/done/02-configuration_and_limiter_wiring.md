# Task 02 — Configuration and limiter wiring

**Plan:** [plan.md](../plan.md)

**Implements:** [.specs/service/specs/06-configuration.md](../../../service/specs/06-configuration.md) §Validation at load, §Committed default, §Sections, and §Defaults summary; [.specs/service/specs/02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §Adapter inventory; [source spec](../../../changes/merged/2026-08-05-audit_and_throttle_authentication_failures.md) §Proposed changes and §Implementation notes
**Depends on:** 01
**Produces:** Validated `audit.durability`, rate-limit, and trusted-proxy settings that construct a bounded in-process limiter and retain explicit no-op behavior when disabled.
**Pointers:** `crates/core/src/config.rs:134`; `config/default.toml`; `crates/server/src/bootstrap.rs:244`; `crates/adapters/src/noop/mod.rs`; `crates/server/Cargo.toml`

## Steps

- [x] Add typed audit durability, rate-limit windows/budgets/concurrency, and trusted-proxy CIDR/hop configuration with safe defaults from the source spec.
- [x] Validate duration, nonzero budgets, maximum entries, proxy CIDRs/hops, and adapter/durability combinations at config-load time with typed `ConfigError`s.
- [x] Change the committed audit adapter default to stdout and wire enabled/disabled rate-limiter selection into `build_service` without adding a shared hot-path dependency.
- [x] Implement the in-process fixed-window state with named bounds, expiry eviction, and bounded cardinality; ensure limiter operational errors are observable by callers and can fail open by policy.
- [x] Add configuration and limiter unit tests at below/at/above bounds, malformed values, disabled/noop behavior, fixed-window expiry, and eviction limits.

## Definition of done

- [x] Default config produces stdout audit, `observe` durability, and the source-spec rate-limit/trusted-proxy defaults.
- [x] Invalid rates, durations, proxy settings, and impossible bounds fail at configuration load before router construction.
- [x] The in-process limiter has explicit window, key-count, and expiry-eviction bounds and never stores a raw subject identifier.
- [x] Tests cover disabled and malformed configuration plus each validity boundary for limiter capacity/window settings.
- [x] Meets the repo definition of done (targeted tests, Rust format/clippy/nextest, assertions, and named-constant limits — see plan.md baseline).
- [x] Reviewable: load valid and invalid configurations and demonstrate a bounded limiter construction with the disabled path selecting noop.
