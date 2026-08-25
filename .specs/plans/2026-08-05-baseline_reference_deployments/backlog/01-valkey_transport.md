# Task 01 — Valkey transport

**Plan:** [plan.md](../plan.md) · **Certificate:** omitted by requester

**Implements:** [change spec §Implementation notes — A.3 Valkey](../../../changes/2026-08-05-baseline_reference_deployments.md#implementation-notes), [service persistence §Session-only stores](../../../service/specs/08-persistence.md#session-only-stores)
**Depends on:** —
**Produces:** a TLS-capable Valkey client with a regression proving a `rediss://` URL selects TLS before any template advertises encrypted Valkey transport.
**Pointers:** `crates/adapters/Cargo.toml:33`; `crates/adapters/src/valkey/mod.rs:118-135`; `crates/adapters/src/valkey/mod.rs:489-...`

## Steps

- [ ] Enable `fred`’s Rustls TLS feature alongside the existing JSON feature, consistent with the workspace TLS stack.
- [ ] Expose or test the parsed client configuration so `rediss://` has a TLS transport and `redis://` remains the explicit non-TLS local path.
- [ ] Add deterministic positive and negative regression coverage without requiring an external Valkey service.
- [ ] Record the local-compose exception for `examples/linux-postgres/config/postgres-valkey.toml` in the template documentation when Task 02 consumes the new contract.

## Definition of done

- [ ] A test proves `rediss://` configures TLS and fails if the required `fred` TLS feature is removed.
- [ ] A paired test proves `redis://` does not accidentally claim TLS.
- [ ] Touched functions validate inputs and preserve typed error propagation under the repository guidelines.
- [ ] Meets the repo definition of done (Rust format, clippy, nextest; negative-space tests; named-constant limits where introduced — see plan.md baseline).
- [ ] Reviewable: inspect the adapter test and feature manifest, then observe TLS selected for `rediss://`.
