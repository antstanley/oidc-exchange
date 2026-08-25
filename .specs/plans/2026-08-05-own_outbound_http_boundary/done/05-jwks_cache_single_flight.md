# Task 05 — Redesign JWKS cache single-flight

**Plan:** [plan.md](../plan.md) · **Status:** Done · **Depends on:** [02](02-provider_transport.md), [04](04-verification_key_set.md)

**Implements:** source `JwksCache` design; implementation notes 1 and 7; cache concurrency tests.

**Scope:** Convert cached values to `Arc<VerificationKeySet>`, remove cache and forced-refresh write guards from every network await, and introduce a bounded single-flight refill mechanism. Preserve current forced-refetch timestamp semantics and fail-closed unknown-`kid` behavior.

## Steps

- [x] Replace deep-cloned cached JSON with `Arc<VerificationKeySet>` and cheap clones at every cache return path; benchmark the large-key-set scenario before/after and record measured result.
- [x] Use a `tokio::sync::Semaphore` (or explicitly approved equivalent) to elect one refill; handle cold cache correctly with `try_acquire`/wait behavior.
- [x] Serve a stale but parseable key set to non-elected callers during refill when one exists; ensure a missing `kid` still forces the rate-limited refetch path before rejection.
- [x] Write forced-refresh timestamp before fetching, release its guard before the await, and guarantee at most one network attempt per minimum interval even after failures.
- [x] Add delayed-origin tests with three or more concurrent expired-TTL callers: exactly one fetch and no caller waits beyond one fetch; test stale, cold, failed, and rate-limited cases.
- [x] Verify the new clippy configuration reports no await-held invalid guard in `jwks.rs`.

## Definition of done

- [x] No `RwLock` guard protecting cached keys or forced-refresh time crosses a network await.
- [x] Expired-cache concurrency has one outbound request, stale availability follows the specified rule, and unknown kids fail closed after a bounded refetch.
- [x] Non-2xx/oversized/malformed JWKS values are never cached, including during concurrent refill.
- [x] Benchmark result meets the source's sub-millisecond target or is documented as a release blocker/exception; no done certificate is produced.

## Notes (completion record)

**Election shape.** `get_keys` races expired-TTL callers for a one-permit
`tokio::sync::Semaphore` (`MAX_CONCURRENT_REFILLS = 1`, a named constant).
`try_acquire` first keeps the cold-cache case correct — the first arrival is
elected without yielding. A caller that loses the election serves the
stale-but-parseable set when one exists (`any_cached`) and returns
immediately: stale is old, not untrusted, and a `kid` absent from it still
falls through to the rate-limited forced refetch, so staleness fails closed.
Only a cold cache queues on `acquire()`. The winner re-checks freshness under
the write guard (a refill may have landed while it raced or waited), fetches
with **no data-lock guard alive across the network await**, stores the fresh
entry, and only then lets the permit drop — so everyone queued behind a
successful wave finds the entry and spends no request of their own.

**Forced path.** `refresh()` semantics are exactly wave A's: the
`MIN_REFRESH_INTERVAL` timestamp is written *before* the network call, the
guard releases before the await, racing callers are declined by the timestamp
alone, and at most one attempt per interval happens even against an unhealthy
upstream. It is serialized by *time*, deliberately not by the permit, so the
TTL-refill and kid-miss triggers stay independently bounded.

**Failed refills.** With a stale set present, a failed elected attempt costs
exactly one request total: every racer arriving during the window gets the
stale set, the fault reaches only the elected caller, and the expired entry is
never wiped nor overwritten with anything unparseable. On a cold cache there
is nothing to serve, so each queued waiter takes one serialized turn of its
own (the permit makes parallel attempts impossible); this bounded retry chain
is recorded as a deviation rather than inventing shared failure state.

**Benchmark.** Cache values were already `Arc<VerificationKeySet>` behind cheap
clones at every return path from task 04's migration; this task adds the
measured evidence. A 96-key set (one real RSA JWK replicated under distinct
kids, sized to stay under the shared 64 KiB body ceiling) fills once, then
2 000 hits measure a mean of **0.8–0.9 µs per hit** (Apple M-series, debug
build) including the RwLock read, TTL check, Arc clone, and kid resolution —
roughly three orders of magnitude under the sub-millisecond target. Not a
release blocker; no exception needed.

**Tests added (6).** All races are barrier-aligned with `RACE_SIZE = 4`
callers against a 400 ms delayed origin, so wiremock `.expect(N)` panics on
drop if single-flight ever degrades into a herd: expired-TTL racers collapse
into one refill with ≥1 racer stale-served and nobody waiting beyond one
fetch (wall-clock bound); cold-cache racers all share the winner's set by
`Arc::ptr_eq` with exactly one request; a failed refill with a stale set
yields exactly one error plus RACE_SIZE−1 stale-served Ok results and leaves
the cache untouched; a failed refill on a cold cache yields one serialized
attempt per waiter (`.expect(RACE_SIZE)`), all failing closed, cache
unpopulated; concurrent unknown-kid lookups force exactly one rate-limited
refetch and all fail closed; the benchmark test asserts the sub-millisecond
mean and prints the measurement. Existing suites (rate-limit-on-failure,
ineligible-kid miss, non-2xx leaving the cache unpopulated, kid-miss rotation)
pass unchanged. Clippy's committed `await-holding-invalid-types` gate runs
clean over the redesigned file.

**Deviations recorded.** (1) Waiters on a *failed* cold-cache wave each make
one serialized retry rather than propagating the leader's error verbatim;
propagating would require shared failure state (an epoch/last-error channel)
that the spec does not describe, and the serialized chain is bounded by the
caller count and the transport timeouts. (2) The forced-refetch path stays
outside the permit (timestamp-serialized) as described above; folding it in
would couple two independently bounded triggers for no stated property.

No done certificate produced.
