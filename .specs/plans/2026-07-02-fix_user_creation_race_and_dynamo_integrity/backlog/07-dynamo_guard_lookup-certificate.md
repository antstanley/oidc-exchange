# Done Certificate — Task 07: DynamoDB get_user_by_external_id via the guard item

**Task:** [07-dynamo_guard_lookup.md](07-dynamo_guard_lookup.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 07. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 07) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** `get_user_by_external_id` is two strongly-consistent `GetItem`s (guard → profile), retiring the User item's GSI1 entry so GSI1 serves only session lookups.
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not change session GSI1 usage (`revoke_all_user_sessions`, session GSI keys); must not change the returned `User` shape.

## Obligations

- **O1 — Lookup resolves via guard then profile.**
  - *Claim:* `get_user_by_external_id` for a created user returns that user by reading the guard then the profile.
  - *Evidence to collect:* run the Dynamo Local test creating a user (task 06 path) then looking it up by external id — expect the same user returned.
  - *Checks:* resolve the two `GetItem` calls — first on `EXT#…`/`UNIQUE`, second on `USER#<id>`/`PROFILE`; confirm no `Query`/`index_name(GSI1)` remains in this method.
  - *Status:* ☐ unverified

- **O2 — Negative-space: no guard yields None.**
  - *Claim:* an identity with no guard item returns `None`, not an error or an arbitrary user.
  - *Evidence to collect:* run the test looking up an unregistered `(provider, external_id)` — expect `Ok(None)`.
  - *Status:* ☐ unverified

- **O3 — Consistent read and no User GSI1 attributes.**
  - *Claim:* the guard `GetItem` sets `consistent_read(true)` and `user_to_item` no longer writes `GSI1pk`/`GSI1sk` for the User item.
  - *Evidence to collect:* read the method for `consistent_read(true)`; read `user_to_item` in `schema.rs` — confirm the User GSI writes are gone; run the updated `user_item_has_correct_keys` test — expect it asserts no GSI keys.
  - *Status:* ☐ unverified

- **O4 — Sidecar and prose describe the two-GetItem lookup.**
  - *Claim:* `table-design.json` drops the User item's GSI keys and rewrites `access_patterns.get_user_by_external_id`; `08-persistence.md` matches.
  - *Evidence to collect:* read `item_schemas.User` (no `GSI1pk`/`GSI1sk`) and `access_patterns.get_user_by_external_id` (two `GetItem`s) in the sidecar; read the `08-persistence.md` §DynamoDB access-pattern row + GSI1-retired + provider-prefix sentences.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean.
  - *Status:* ☐ unverified

- **O6 — Reviewable: guard lookup returns the user; missing guard returns None.**
  - *Claim:* a reviewer runs the guard-lookup Dynamo Local test and observes the user resolved via the guard and a missing guard yielding `None`.
  - *Evidence to collect:* run the two tests (present-guard hit, absent-guard `None`).
  - *Status:* ☐ unverified

## Regression check

- `exchange()` and `refresh` call `get_user_by_external_id` / `get_user_by_id` — trace an active-user exchange after the lookup change → expect the user still resolved and a token issued : ☐ (PRESERVED / REGRESSION)
- `revoke_all_user_sessions` still uses GSI1 for sessions — trace revoke-all → expect it still finds and deletes sessions (session GSI untouched) : ☐ (PRESERVED / REGRESSION)

## Residue

- The guard backfill from task 06 must have run before this ships; a validator should confirm the backfill mechanism exists, since a guard-less legacy user would become invisible to the new lookup.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
