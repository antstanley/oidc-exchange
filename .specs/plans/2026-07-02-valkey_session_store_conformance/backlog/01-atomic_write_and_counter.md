# Task 01 — Atomic session write with counter increment and TTL rejection

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-atomic_write_and_counter-certificate.md](01-atomic_write_and_counter-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §"Session-only stores" (Valkey atomic writes, maintained counter, TTL'd index sets, TTL rejection) · change spec Implementation note 1
**Depends on:** —
**Produces:** `store_refresh_token` applies the session hash, its TTL, the user-set membership, an `EXPIRE {prefix}user_sessions:{user_id} ttl GT` bump, and an `INCR {prefix}active_sessions` through a single `fred` pipeline; a session whose `expires_at` is not in the future is rejected with `Error::StoreError` and writes no key; the `#[ignore]`-gated Valkey integration-test module and its live-server harness exist for later tasks to reuse.
**Pointers:** `crates/adapters/src/valkey/mod.rs:37-81` (`store_refresh_token`), `:25-31` (`session_key`/`user_sessions_key` helpers — add an `active_sessions_key` beside them), `fred` pipeline via `client.pipeline()` and `ExpireOptions::GT` (`fred` 10), `crates/core/src/error.rs:36` (`StoreError`), test pattern at `crates/adapters/src/dynamo/mod.rs:489` (`#[cfg(test)] mod tests`, `#[ignore]` live-backend tests).

## Steps

- [ ] Add an `active_sessions_key(&self) -> String` helper returning `{prefix}active_sessions`, beside the existing key helpers.
- [ ] Declare a named TTL floor constant with units (e.g. `SESSION_TTL_SECONDS_MIN: i64 = 1`); compute `ttl_seconds` from `expires_at - Utc::now()` first and return `Error::StoreError` when it is below the floor (non-future `expires_at`), before issuing any write.
- [ ] Replace the three separate `hset`/`expire`/`sadd` calls with one `client.pipeline()` batching: `HSET` the fields, `EXPIRE {key} ttl_seconds`, `SADD {prefix}user_sessions:{user_id} {hash}`, `EXPIRE {prefix}user_sessions:{user_id} ttl_seconds GT` (only-extend), and `INCR {prefix}active_sessions`; execute the pipeline once and map any error to `StoreError`.
- [ ] Add ≥2 meaningful assertions (e.g. assert `ttl_seconds >= SESSION_TTL_SECONDS_MIN` after the guard as a postcondition; assert the pipeline was constructed with the expected non-empty key inputs).
- [ ] Add a `#[cfg(test)] mod tests` with a live-server harness: a `create_test_repo` that builds a `ValkeySessionRepository` against a local Valkey using a unique `key_prefix` per test (so runs isolate and self-clean); mark the tests `#[ignore]` with a run comment, mirroring the DynamoDB module.
- [ ] Write integration tests: a stored session yields a TTL'd hash and a user-set member, the set TTL is bumped, and the counter reads 1 after one store; a second store for the same user with a **shorter** TTL does not shorten the set TTL (GT only-extends); a session with `expires_at` at or before now is rejected with `StoreError` and creates neither the hash nor the counter.

## Definition of done

- [ ] `store_refresh_token` issues the hash, EXPIRE, SADD, set-TTL `GT` bump, and counter `INCR` as one `fred` pipeline, and rejects a non-future `expires_at` with `Error::StoreError` before writing anything.
- [ ] Negative-space test: a zero/negative-TTL session is rejected and leaves no `{prefix}session:*` key and no counter increment; the `GT` bump does not shorten an existing longer set TTL.
- [ ] The TTL floor is a named constant with units; the `active_sessions_key` helper exists; the touched function carries ≥2 meaningful assertions.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace`; `#[ignore]` integration tests pass against a local Valkey — see plan.md baseline).
- [ ] Reviewable: with a local Valkey running, `cargo nextest run -p oidc-exchange-adapters -- --ignored valkey` shows the store test writing a TTL'd hash + user-set member + counter at 1, the GT-only-extend test green, and the negative-TTL test asserting no key was created.

## Open questions

- Whether the harness reads the Valkey URL from an env var (e.g. `VALKEY_TEST_URL`) with a `redis://localhost:6379` default, matching how the DynamoDB tests hardcode the Local endpoint. Resolve at build time; does not block the slice.
