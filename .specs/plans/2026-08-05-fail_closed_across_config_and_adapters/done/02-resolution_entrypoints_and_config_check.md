# Task 02 — Resolution entrypoints and config check

**Plan:** [plan.md](../plan.md)  
**Status:** Done  
**Implements:** [source spec](../../../changes/merged/2026-08-05-fail_closed_across_config_and_adapters.md) → Configuration Loading order and Implementation notes 3/5; [configuration canonical page](../../../service/specs/06-configuration.md) → Loading order and Validation at load  
**Depends on:** 01  
**Produces:** `load_config`, `parse_config`, server/Lambda startup, and FFI construction converge on `Config::resolve`; `oidc-exchange config check <path>` resolves without adapter/network side effects and renders the deployable config safely.  
**Pointers:** `crates/server/src/bootstrap.rs`; `crates/server/src/main.rs`; `crates/ffi/src/lib.rs`; `crates/ffi/tests/integration.rs`; `crates/server/Cargo.toml`; `config/default.toml`.

## Steps

- [x] Repoint disk configuration assembly in `bootstrap::load_config` to parse raw TOML,
  preserve documented overlay/environment/placeholder ordering, and call the single resolver.
- [x] Repoint `bootstrap::parse_config` and FFI `OidcExchange::new`/`from_file` construction to
  the same resolver with equivalent environment behavior; no FFI-only unvalidated parse remains.
- [x] Add the `config check <path>` CLI subcommand while preserving `--version`; make it read the
  supplied file through the same resolve path, avoid service/adapters/network initialization, and
  print a redacted resolved view or a field-named failure.
- [x] Decide and test the path/overlay semantics for `config check` from the source spec and
  current loader conventions; reject ambiguous or unreadable inputs rather than falling back to
  the working directory silently.
- [x] Add integration tests proving disk load, raw TOML/FFI construction, and config-check each
  accept the same good fixture and reject the same invalid closed-domain fixture.
- [x] Add tests proving config check does not initialize KMS, repositories, providers, routers,
  telemetry, or a listener, and does not print redacted secrets.

## Definition of done

- [x] Exactly one production resolve implementation establishes the configuration invariants for
  server, Lambda, FFI, and config-check paths.
- [x] Overlay/env/placeholder behavior stays in the documented order; tests cover a positive
  merged case and failure propagation from resolution.
- [x] `oidc-exchange config check <path>` exits successfully only for resolvable configuration,
  returns a non-zero error naming the faulty field otherwise, and has no adapter side effects.
- [x] `--version` remains supported and no unrelated CLI/argument supply-chain work is absorbed.
- [x] Focused server/FFI/CLI tests, `cargo fmt`, and relevant clippy checks are reported.

## Execution evidence — 2026-08-16

- Completed in PR25; implementation and focused verification are covered by the final workspace suite: `cargo nextest run --workspace --no-fail-fast` — **389 passed, 27 skipped**.

## Sibling boundaries

- The placeholder-resolution sibling depends on this shared resolver. Do not take ownership of
  its placeholder-gap tests or canonical rewording beyond preserving the shared seam.
- Runtime parity beyond the named construction paths belongs to the runtime-parity sibling.
