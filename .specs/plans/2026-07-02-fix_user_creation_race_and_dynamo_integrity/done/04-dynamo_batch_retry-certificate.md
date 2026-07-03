# Done Certificate — Task 04: DynamoDB BatchWriteItem unprocessed-items retry

**Task:** [04-dynamo_batch_retry.md](04-dynamo_batch_retry.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 04. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** `revoke_all_user_sessions` and `cleanup_expired_sessions` retry `BatchWriteItem` `unprocessed_items` with capped exponential backoff until the batch drains or a named budget is exhausted (then error), so a success means every targeted session was deleted.
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not change the query/scan pagination or the 25-item chunking; must not turn a genuine SDK error into a silent `Ok`.

## Obligations

- **O1 — The retry helper drains and reports the true deleted count.**
  - *Claim:* a batch returning items as `unprocessed` for N-1 attempts then draining is fully deleted within the budget, and the reported count equals the drained count.
  - *Evidence to collect:* run the retry-helper unit test with a fake client scripted to return unprocessed items then drain — expect the loop drains and returns the true count.
  - *Checks:* resolve the re-submit call to the helper's own `batch_write_item`, and confirm it re-submits only the returned `unprocessed_items`, not the original full batch.
  - *Status:* ☑ SATISFIED — `retry_tests::drains_within_budget_and_reports_true_deleted_count` PASS: fake client returns 1 item unprocessed for attempts 1–3, drains at attempt 4 (≤ `BATCH_WRITE_MAX_ATTEMPTS` = 8); asserts the result equals all 3 submitted items and exactly 4 submit calls (stops on drain). Resolution check: `batch_write_with_retry` (dynamo/mod.rs:90–110) calls `self.client.batch_write_item()` — the `aws_sdk_dynamodb::Client` fluent builder, no shadow — and `drain_unprocessed` reassigns `pending = submit(pending).await?` where the closure returns only `unprocessed_items.remove(&table_name)`, so retries carry only the unprocessed subset.

- **O2 — Negative-space: a never-draining batch exhausts the budget and errors.**
  - *Claim:* items that never drain cause an `Err`, not `Ok`.
  - *Evidence to collect:* run the unit test whose fake client always returns items unprocessed — expect `Err(StoreError)` after `BATCH_WRITE_MAX_ATTEMPTS`, not an infinite loop or `Ok`.
  - *Status:* ☑ SATISFIED — `retry_tests::errors_when_retry_budget_is_exhausted_without_draining` PASS: a submit that always echoes its input unprocessed yields `Err(Error::StoreError { detail })` with `detail` naming the unprocessed remainder; the loop is bounded by `1..=BATCH_WRITE_MAX_ATTEMPTS` (dynamo/mod.rs:49) and the test completed in 0.028s under paused tokio time — no infinite loop.

- **O3 — Retry budget and backoff are named constants.**
  - *Claim:* the loop bound and backoff base are named constants with unit suffixes, referenced by name.
  - *Evidence to collect:* grep the retry helper for numeric literals; confirm `BATCH_WRITE_MAX_ATTEMPTS` and a backoff-base constant (e.g. `_MS`) are defined and referenced, no magic numbers.
  - *Status:* ☑ SATISFIED — `BATCH_WRITE_MAX_ATTEMPTS: u32 = 8` (dynamo/mod.rs:25) and `BATCH_WRITE_BACKOFF_BASE_MS: u64 = 50` (dynamo/mod.rs:30) are module constants with doc comments and unit suffix; the loop bound and backoff computation reference them by name. Remaining literals in the helper (`1`, `2` in `attempt > 1` / `1u64 << (attempt - 2)`) are loop-index arithmetic, not budget/backoff magic numbers.

- **O4 — cleanup counts actually-deleted items.**
  - *Claim:* `cleanup_expired_sessions` returns the count of items drained (deleted), not the count submitted at `dynamo/mod.rs:392`.
  - *Evidence to collect:* read the modified `cleanup_expired_sessions`; confirm the counter increments on drained results. Run its unit/integration test asserting the count matches deletions when some items were initially unprocessed.
  - *Status:* ☑ SATISFIED — the pre-send `deleted += delete_requests.len()` is removed; `cleanup_expired_sessions` now does `deleted += self.batch_write_with_retry(delete_requests).await?` (dynamo/mod.rs:473), i.e. counts the helper's drained return (and propagates `Err` when a batch does not drain, so no over-count). The initially-unprocessed count assertion is carried by the fake-client unit test (`drains_within_budget_and_reports_true_deleted_count`) per the Residue note — Dynamo Local cannot inject `unprocessed_items`.

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; `08-persistence.md` prose updated.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean; confirm the DynamoDB batch-retry / cleanup sentences are rewritten.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` clean; `cargo clippy --workspace -- -D warnings` clean; `cargo nextest run --workspace` 267 passed, 0 failed (12 skipped: environment-gated integration tests). `08-persistence.md` §DynamoDB rewritten: the revoke-all table row notes "(unprocessed items retried)" and the `cleanup_expired_sessions` no-op-cost sentence is replaced with the retry-until-drained / bounded-budget / "successful return means every expired session found is gone" prose.

- **O6 — Reviewable: retry-loop tests pass and both call sites drain.**
  - *Claim:* a reviewer runs the retry-loop unit tests and reads both call sites, confirming they drain unprocessed items and error on budget exhaustion.
  - *Evidence to collect:* run the tests; read `revoke_all_user_sessions` and `cleanup_expired_sessions`, confirming both route through the retry helper.
  - *Status:* ☑ SATISFIED — ran `cargo nextest run -p oidc-exchange-adapters -E 'test(retry_tests)'`: 3/3 pass (drain-within-budget, budget-exhaustion error, empty-batch no-op). Read both call sites: `cleanup_expired_sessions` (dynamo/mod.rs:473) and `revoke_all_user_sessions` (dynamo/mod.rs:540) both call `self.batch_write_with_retry(...)`, which routes through `drain_unprocessed` — draining unprocessed items and erroring on budget exhaustion.

## Regression check

- `SessionRepository::revoke_all_user_sessions` caller (`admin_delete_user`, revoke flow) — trace revoke-all with no throttling → expect it still deletes every session and returns `Ok(())` : ☑ PRESERVED — GSI1 query pages → `chunks(25)` (unchanged) → `batch_write_with_retry` → first submit returns an empty `unprocessed_items` map → `drain_unprocessed` returns `Ok(total)` on attempt 1 with no backoff sleep → `Ok(())`. A genuine SDK error still propagates via `.map_err(Self::store_err)?` inside the submit closure — not silenced into `Ok` (invariant P3 holds; scan/query pagination untouched by the diff).
- `cleanup_expired_sessions` caller (external scheduler / admin) — trace a clean sweep → expect the deleted count equals the number of expired items : ☑ PRESERVED — each 25-item chunk drains on attempt 1, `deleted` accumulates the helper's drained count per chunk across all scan pages, totalling exactly the expired items found.

## Residue

- Dynamo Local does not readily inject `unprocessed_items`, so the drain path is unit-tested against a fake client rather than the live store (per the change spec's note 10); a validator should accept the fake-client unit test as the primary evidence.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with run evidence (3/3 retry-helper unit tests pass, fmt/clippy/nextest workspace clean at 267 passed, both call sites resolved through `batch_write_with_retry`/`drain_unprocessed` with named constants and drained-count accounting, `08-persistence.md` prose rewritten), and both named regression callers trace PRESERVED with the 25-item chunking, pagination, and SDK-error propagation invariants intact.
