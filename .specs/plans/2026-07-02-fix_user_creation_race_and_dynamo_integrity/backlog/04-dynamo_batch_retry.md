# Task 04 — DynamoDB BatchWriteItem unprocessed-items retry

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-dynamo_batch_retry-certificate.md](04-dynamo_batch_retry-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §DynamoDB (batch-delete retry of `unprocessed_items`; the `cleanup_expired_sessions` no-op-cost sentence rewrite)
**Depends on:** —
**Produces:** `revoke_all_user_sessions` and `cleanup_expired_sessions` retry any `BatchWriteItem` `unprocessed_items` with capped exponential backoff until the batch drains or a named retry budget is exhausted (then error), so a successful return means every targeted session item was deleted.
**Pointers:** `crates/adapters/src/dynamo/mod.rs:466-471` (revoke-all batch send) · `crates/adapters/src/dynamo/mod.rs:392-399` (cleanup batch send; also `:392` deletion count) · new named constants in the module

## Steps

- [ ] Add module constants for the retry budget and backoff base (e.g. `BATCH_WRITE_MAX_ATTEMPTS: u32` and `BATCH_WRITE_BACKOFF_BASE_MS: u64`), named with units per the guidelines.
- [ ] Extract a helper that submits one `BatchWriteItem` and, while the response's `unprocessed_items` is non-empty, re-submits only the unprocessed requests with capped exponential backoff, up to the budget; return an `Error::StoreError` when the budget is exhausted with items still unprocessed.
- [ ] Call the helper from both `revoke_all_user_sessions` (`:466-471`) and `cleanup_expired_sessions` (`:392-399`).
- [ ] In `cleanup_expired_sessions`, count deletions from drained (actually-deleted) requests, not from the submitted request count at `:392`.
- [ ] Rewrite the `08-persistence.md` §DynamoDB batch-retry / `cleanup_expired_sessions` sentences per the change spec (retry unprocessed items; a successful return means every expired session found is gone).

## Definition of done

- [ ] A unit test of the retry helper feeds a fake client that returns items as `unprocessed` for the first N-1 attempts then drains, and asserts the loop drains within the budget and reports the true deleted count.
- [ ] Negative-space test: a helper input whose items never drain exhausts the budget and returns an error (not `Ok`).
- [ ] The retry budget and backoff base are named constants with unit suffixes, referenced by name — no magic numbers in the loop.
- [ ] `cleanup_expired_sessions` returns the count of items actually deleted across drained batches, not the count submitted.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the retry-loop unit tests and reads the two call sites, confirming both drain unprocessed items and a budget-exhaustion path errors.
