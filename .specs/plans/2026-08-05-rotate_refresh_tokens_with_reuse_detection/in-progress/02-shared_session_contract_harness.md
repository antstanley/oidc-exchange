# Task 02 — Shared session-store conformance harness

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Session-store conformance suite (SR1–SR5).
**Depends on:** 01 · domain_config_port_contract
**Produces:** generic, reusable assertions over `impl SessionRepository`, invoked by `MockRepository` and ready for each persistent adapter.
**Pointers:** `crates/test-utils/src/session_contract.rs` (new); `crates/test-utils/src/lib.rs`; adapter test modules under `crates/adapters`.

## Steps

- [ ] Add a test-utils module exposing fixture builders and generic async assertions without leaking adapter internals.
- [ ] Cover: post-rotation classification, one true under concurrent rotate, false-CAS no mutation, immediately readable retirement, older generation as `Retired`, complete count-returning family revocation, immediate unknown after revoke, and absolute-expiry preservation.
- [ ] Invoke the suite for `MockRepository`; make clock/identifiers deterministic and bound concurrency/timeouts with named constants.
- [ ] Provide an adapter-facing invocation pattern that permits ignored Dynamo integration tests without weakening assertions.

## Definition of done

- [ ] Each SR1–SR5 obligation maps to at least one named shared assertion and a negative case.
- [ ] The concurrency assertion awaits two competing operations and verifies exactly one success, not merely eventual state.
- [ ] The failed-CAS assertion snapshots all observable session/retirement state before and after.
- [ ] `MockRepository` runs the entire suite in normal tests; API is consumable by all five adapter modules.
- [ ] Done certificates remain intentionally absent.
