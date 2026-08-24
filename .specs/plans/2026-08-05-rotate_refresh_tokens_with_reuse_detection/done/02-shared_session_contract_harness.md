# Task 02 — Shared session-store conformance harness

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Session-store conformance suite (SR1–SR5).
**Depends on:** 01 · domain_config_port_contract
**Produces:** generic, reusable assertions over `impl SessionRepository`, invoked by `MockRepository` and ready for each persistent adapter.
**Pointers:** `crates/test-utils/src/session_contract.rs` (new); `crates/test-utils/src/lib.rs`; adapter test modules under `crates/adapters`.

## Steps

- [x] Add a test-utils module exposing fixture builders and generic async assertions without leaking adapter internals.
- [x] Cover: post-rotation classification, one true under concurrent rotate, false-CAS no mutation, immediately readable retirement, older generation as `Retired`, complete count-returning family revocation, immediate unknown after revoke, and absolute-expiry preservation.
- [x] Invoke the suite for `MockRepository`; make clock/identifiers deterministic and bound concurrency/timeouts with named constants.
- [x] Provide an adapter-facing invocation pattern that permits ignored Dynamo integration tests without weakening assertions.

## Definition of done

- [x] Each SR1–SR5 obligation maps to at least one named shared assertion and a negative case.
- [x] The concurrency assertion awaits two competing operations and verifies exactly one success, not merely eventual state.
- [x] The failed-CAS assertion snapshots all observable session/retirement state before and after.
- [x] `MockRepository` runs the entire suite in normal tests; API is consumable by all five adapter modules.
- [x] Done certificates remain intentionally absent.

## Completion notes

- `crates/test-utils/src/session_contract.rs` is a public module of generic assertions over any `SessionRepository + ?Sized` — no adapter type appears anywhere in it. Fixture builders: `fixture_family_id` (`fam_` + 26 lowercase Crockford characters derived from SHA-256 of a caller-supplied tag), `fixture_hash` (SHA-256 hex), `family_chain` (a gen0/gen1/alt_gen1/gen2 chain whose successors inherit `expires_at`/`created_at` per contract), `generation_session`, and `capture_base_instant`.
- Determinism: identifiers are fully deterministic from the tag (disjoint suites can share one physical backend; re-runs are reproducible); the clock is deterministic *in relation* — each assertion captures one base instant and derives every timestamp by explicit offsets. Named constants bound the race and fixtures: `CONCURRENT_ROTATIONS = 2`, `RACE_JOIN_TIMEOUT = 10s`, `FIXTURE_FAMILY_TTL_SECS = 24h`.
- Obligation map (each assertion carries its negative space inline): `assert_resolution_classifies_all_four_shapes` (SR1 classification: Live → Superseded → Retired → Unknown, fallen generation must not keep claiming grace); `assert_rotation_installs_successor_and_demotes_presented` (SR2 effects + `/revoke` liveness lookup); `assert_failed_cas_leaves_store_byte_identical` (SR2 negative: port-level snapshot — resolution + full live-session lookup for every hash in play plus active count — compared before/after losing against both a moved hash and an unknown hash; loser's proposal must not exist in any form); `assert_concurrent_rotation_yields_exactly_one_winner` (SR3: two same-family proposals raced via `tokio::join!` under `RACE_JOIN_TIMEOUT`, XOR-checked outcomes, store verified to hold only the winner); `assert_retirement_readable_immediately_after_rotation` (SR4: first post-rotation observation is already `Superseded`, never `Unknown`); `assert_older_generation_resolves_as_retired` (retained history); `assert_family_revocation_removes_everything_and_returns_count` (SR5: count covers live row + records, scoped to one family, honest zero on re-revoke and on unknown well-formed id; active-count check is store-relative so it also holds on backends carrying unrelated families); `assert_resolution_unknown_immediately_after_revoke` (SR1 negative: revoked hash reads Unknown immediately while its predecessor's retained record reclassifies to Retired); `assert_rotation_preserves_absolute_expiry` (expiry/creation inheritance).
- Invocation: `MockRepository` calls every granular assertion as its own normal `#[tokio::test]` plus the `assert_full_conformance` orchestrator once end-to-end (`tests::conformance_*` in `crates/test-utils/src/lib.rs`). The old inline mock tests were re-homed into the suite rather than deleted; a mock-only companion (`failed_cas_leaves_internal_maps_byte_identical`) keeps an internal-map snapshot beyond the port surface.
- Adapter-facing pattern (documented on the module): completed adapters call `session_contract::assert_full_conformance(&store, "<adapter-tag>")` from a normal `#[tokio::test]`; DynamoDB wraps the identical call in `#[tokio::test] #[ignore = "requires DynamoDB Local"]` behind its existing environment gating — the assertions are byte-for-byte the same, only the gating differs. Interim adapters (tasks 03–06) invoke individual granular assertions until each obligation is implemented, so coverage grows without weakening anything.
- Workspace gates after extraction: fmt clean, clippy 0 findings, nextest 416 passed / 27 skipped (up from 408: ten conformance tests replace three inline ones). No done certificate exists.
