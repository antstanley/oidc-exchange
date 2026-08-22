# Task 05 — Redesign JWKS cache single-flight

**Plan:** [plan.md](../plan.md) · **Status:** Backlog · **Depends on:** [02](02-provider_transport.md), [04](04-verification_key_set.md)

**Implements:** source `JwksCache` design; implementation notes 1 and 7; cache concurrency tests.

**Scope:** Convert cached values to `Arc<VerificationKeySet>`, remove cache and forced-refresh write guards from every network await, and introduce a bounded single-flight refill mechanism. Preserve current forced-refetch timestamp semantics and fail-closed unknown-`kid` behavior.

## Steps

- [ ] Replace deep-cloned cached JSON with `Arc<VerificationKeySet>` and cheap clones at every cache return path; benchmark the large-key-set scenario before/after and record measured result.
- [ ] Use a `tokio::sync::Semaphore` (or explicitly approved equivalent) to elect one refill; handle cold cache correctly with `try_acquire`/wait behavior.
- [ ] Serve a stale but parseable key set to non-elected callers during refill when one exists; ensure a missing `kid` still forces the rate-limited refetch path before rejection.
- [ ] Write forced-refresh timestamp before fetching, release its guard before the await, and guarantee at most one network attempt per minimum interval even after failures.
- [ ] Add delayed-origin tests with three or more concurrent expired-TTL callers: exactly one fetch and no caller waits beyond one fetch; test stale, cold, failed, and rate-limited cases.
- [ ] Verify the new clippy configuration reports no await-held invalid guard in `jwks.rs`.

## Definition of done

- [ ] No `RwLock` guard protecting cached keys or forced-refresh time crosses a network await.
- [ ] Expired-cache concurrency has one outbound request, stale availability follows the specified rule, and unknown kids fail closed after a bounded refetch.
- [ ] Non-2xx/oversized/malformed JWKS values are never cached, including during concurrent refill.
- [ ] Benchmark result meets the source's sub-millisecond target or is documented as a release blocker/exception; no done certificate is produced.
