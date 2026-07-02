# Done Certificate — Task 07: DynamoDB get_user_by_external_id via the guard item

**Task:** [07-dynamo_guard_lookup.md](07-dynamo_guard_lookup.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* SATISFIED — `dynamo::tests::get_user_by_external_id_resolves_through_guard_then_profile` run against DynamoDB Local (docker, port 8000): PASS — the user created via task 06's transactional `create_user` is returned with matching id/external_id/provider/email/display_name. Method (`crates/adapters/src/dynamo/mod.rs:223`) issues `get_item` on `guard_pk(provider, external_id)` / `GUARD_SK` then `get_item` on `USER#<user_id>` / `PROFILE`; `guard_pk` resolves to `schema.rs:103` (`EXT#<provider>#<external_id>`), `GUARD_SK` to `schema.rs:100` (`UNIQUE`) — both imported from `schema`, no shadowing. No `query()`/`index_name(GSI1)` remains in the method (remaining GSI1 uses are session revoke at mod.rs:628-638 and test table creation at mod.rs:854-892).

- **O2 — Negative-space: no guard yields None.**
  - *Claim:* an identity with no guard item returns `None`, not an error or an arbitrary user.
  - *Evidence to collect:* run the test looking up an unregistered `(provider, external_id)` — expect `Ok(None)`.
  - *Status:* SATISFIED — `dynamo::tests::get_user_by_external_id_with_no_guard_item_returns_none` run against DynamoDB Local: PASS — with an unrelated user present in the table, looking up `("google|no_such_identity", "google")` returns `Ok(None)` (no error, no arbitrary user). Code path: `guard.item` is `None` → early `return Ok(None)` (mod.rs, `let Some(guard_item) = guard.item else`).

- **O3 — Consistent read and no User GSI1 attributes.**
  - *Claim:* the guard `GetItem` sets `consistent_read(true)` and `user_to_item` no longer writes `GSI1pk`/`GSI1sk` for the User item.
  - *Evidence to collect:* read the method for `consistent_read(true)`; read `user_to_item` in `schema.rs` — confirm the User GSI writes are gone; run the updated `user_item_has_correct_keys` test — expect it asserts no GSI keys.
  - *Status:* SATISFIED — the guard `get_item` sets `.consistent_read(true)` (the profile `get_item` does too); `user_to_item` in `schema.rs` no longer inserts `GSI1pk`/`GSI1sk` for the User item (replaced by a comment pointing at the guard); `dynamo::schema::tests::user_item_has_correct_keys` now asserts `!item.contains_key("GSI1pk")` and `!item.contains_key("GSI1sk")` — run: PASS. Session items keep their GSI keys (`session_item_has_correct_keys`: PASS).

- **O4 — Sidecar and prose describe the two-GetItem lookup.**
  - *Claim:* `table-design.json` drops the User item's GSI keys and rewrites `access_patterns.get_user_by_external_id`; `08-persistence.md` matches.
  - *Evidence to collect:* read `item_schemas.User` (no `GSI1pk`/`GSI1sk`) and `access_patterns.get_user_by_external_id` (two `GetItem`s) in the sidecar; read the `08-persistence.md` §DynamoDB access-pattern row + GSI1-retired + provider-prefix sentences.
  - *Status:* SATISFIED — `schemas/dynamodb/table-design.json`: `item_schemas.User` has only `pk`/`sk` (GSI keys removed); `access_patterns.get_user_by_external_id` is `"GetItem + GetItem"` describing the strongly-consistent guard read (`EXT#<provider>#<external_id>` / `UNIQUE`) then `USER#<user_id>` / `PROFILE`, with None on an absent guard. `08-persistence.md`: User item row shows `—`/`—` for GSI1, a new "GSI1 is retired for the User item…" paragraph, the access-pattern row rewritten as the two-`GetItem` guard path, and the closing provider-prefix sentence rewritten to place the prefix on the guard item's `pk`.

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean.
  - *Status:* SATISFIED — `cargo fmt --all --check`: clean; `cargo clippy --workspace -- -D warnings`: clean; `cargo nextest run --workspace`: 270 passed, 0 failed (21 skipped = the `#[ignore]` Dynamo Local tests, run separately under O1/O2/O6). Key strings (`UNIQUE`, guard pk format) live behind the named `GUARD_SK` constant and `guard_pk` helper.

- **O6 — Reviewable: guard lookup returns the user; missing guard returns None.**
  - *Claim:* a reviewer runs the guard-lookup Dynamo Local test and observes the user resolved via the guard and a missing guard yielding `None`.
  - *Evidence to collect:* run the two tests (present-guard hit, absent-guard `None`).
  - *Status:* SATISFIED — exercised as the reviewer: started DynamoDB Local (docker `amazon/dynamodb-local`, port 8000) and ran `cargo nextest run -p oidc-exchange-adapters --run-ignored=all` on the two tests — `get_user_by_external_id_resolves_through_guard_then_profile`: PASS (user resolved via guard → profile) and `get_user_by_external_id_with_no_guard_item_returns_none`: PASS (missing guard yields `None`).

## Regression check

- `exchange()` and `refresh` call `get_user_by_external_id` / `get_user_by_id` — trace an active-user exchange after the lookup change → expect the user still resolved and a token issued : PRESERVED — the port signature (`crates/core/src/ports/repository.rs:11`) and returned `User` shape are untouched (no core files in the diff); `exchange.rs:87,147` call through the same trait; the Dynamo `dynamo_repository_crud` integration test exercises create → `get_user_by_external_id` → found (PASS), so an active-user exchange still resolves the user.
- `revoke_all_user_sessions` still uses GSI1 for sessions — trace revoke-all → expect it still finds and deletes sessions (session GSI untouched) : PRESERVED — `revoke_all_user_sessions` (mod.rs:628) still queries `GSI1` with `GSI1pk = USER#<user_id>`; session items keep their GSI keys (`session_item_has_correct_keys`: PASS); `dynamo_repository_crud` exercises revoke-all against DynamoDB Local (mod.rs:1083): PASS. Full ignored Dynamo Local suite: 22/22 PASS.

Residue check: the backfill mechanism exists — `DynamoRepository::backfill_uniqueness_guards` with the ordering precondition documented on both the backfill and the lookup, and `backfill_writes_guards_for_legacy_users_and_is_idempotent`: PASS. Running it in production before deploy remains an operational step outside this repo.

## Residue

- The guard backfill from task 06 must have run before this ships; a validator should confirm the backfill mechanism exists, since a guard-less legacy user would become invisible to the new lookup.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with direct evidence — both guard-lookup Dynamo Local tests run and pass, `consistent_read(true)` and the removed User GSI keys verified in code and by the updated schema test, sidecar and prose rewritten to the two-`GetItem` guard path, fmt/clippy/nextest clean (270 passed) — and both named regression surfaces (exchange lookup path, session GSI1 revoke-all) are PRESERVED with the full 22-test Dynamo Local suite green.
