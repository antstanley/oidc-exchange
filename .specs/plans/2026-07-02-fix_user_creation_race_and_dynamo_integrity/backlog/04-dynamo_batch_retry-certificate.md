# Done Certificate — Task 04: DynamoDB BatchWriteItem unprocessed-items retry

**Task:** [04-dynamo_batch_retry.md](04-dynamo_batch_retry.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — Negative-space: a never-draining batch exhausts the budget and errors.**
  - *Claim:* items that never drain cause an `Err`, not `Ok`.
  - *Evidence to collect:* run the unit test whose fake client always returns items unprocessed — expect `Err(StoreError)` after `BATCH_WRITE_MAX_ATTEMPTS`, not an infinite loop or `Ok`.
  - *Status:* ☐ unverified

- **O3 — Retry budget and backoff are named constants.**
  - *Claim:* the loop bound and backoff base are named constants with unit suffixes, referenced by name.
  - *Evidence to collect:* grep the retry helper for numeric literals; confirm `BATCH_WRITE_MAX_ATTEMPTS` and a backoff-base constant (e.g. `_MS`) are defined and referenced, no magic numbers.
  - *Status:* ☐ unverified

- **O4 — cleanup counts actually-deleted items.**
  - *Claim:* `cleanup_expired_sessions` returns the count of items drained (deleted), not the count submitted at `dynamo/mod.rs:392`.
  - *Evidence to collect:* read the modified `cleanup_expired_sessions`; confirm the counter increments on drained results. Run its unit/integration test asserting the count matches deletions when some items were initially unprocessed.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; `08-persistence.md` prose updated.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean; confirm the DynamoDB batch-retry / cleanup sentences are rewritten.
  - *Status:* ☐ unverified

- **O6 — Reviewable: retry-loop tests pass and both call sites drain.**
  - *Claim:* a reviewer runs the retry-loop unit tests and reads both call sites, confirming they drain unprocessed items and error on budget exhaustion.
  - *Evidence to collect:* run the tests; read `revoke_all_user_sessions` and `cleanup_expired_sessions`, confirming both route through the retry helper.
  - *Status:* ☐ unverified

## Regression check

- `SessionRepository::revoke_all_user_sessions` caller (`admin_delete_user`, revoke flow) — trace revoke-all with no throttling → expect it still deletes every session and returns `Ok(())` : ☐ (PRESERVED / REGRESSION)
- `cleanup_expired_sessions` caller (external scheduler / admin) — trace a clean sweep → expect the deleted count equals the number of expired items : ☐ (PRESERVED / REGRESSION)

## Residue

- Dynamo Local does not readily inject `unprocessed_items`, so the drain path is unit-tested against a fake client rather than the live store (per the change spec's note 10); a validator should accept the fake-client unit test as the primary evidence.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
