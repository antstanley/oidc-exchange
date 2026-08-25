# 02 · Exchange-only default and config validation

**Status:** Done  
**Implements:** [source spec](../../../changes/merged/2026-08-05-harden_admin_plane.md) §"Implementation notes" step 3; [04-http-api](../../../service/specs/04-http-api.md) Service roles target; [06-configuration](../../../service/specs/06-configuration.md) target  
**Depends on:** —  
**Produces:** An omitted `server.role` defaults to `exchange`, with migration documentation and tests that make implicit-admin exposure impossible.

**Pointers:** `crates/core/src/config.rs:74-90,152-163`; `config/default.toml`; configuration tests; deployment/release documentation location selected under existing project conventions.

## Work

- Change only the default role from `all` to `exchange`; keep explicit role validation and role-specific behaviour intact for task 04 to consume.
- Add configuration tests for absent role, explicit `all`, and explicit `admin` so the compatibility impact is intentional and visible.
- Add a release/deployment migration note instructing installations that depended on implicit `all` to set `server.role` explicitly; use an established documentation location, not a new speculative changelog.

## Definition of done

- [x] Deserializing/defaulting configuration without `server.role` yields `exchange`; explicit `all` and `admin` preserve their configured values.
- [x] Tests establish that the default cannot serve internal routes once task 04’s router split is present, without asserting an implementation not yet built here.
- [x] The migration impact is documented with rationale: admin exposure must be a deliberate deployment decision.
- [x] `cargo fmt --check --all`, `cargo clippy --workspace -- -D warnings`, and affected Rust tests pass; unrelated failures are recorded but not fixed.
- [x] Reviewable: the default is one named configuration value with explicit-deployment migration guidance.
