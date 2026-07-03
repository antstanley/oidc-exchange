# Change: Fix user-creation races and DynamoDB write integrity

**Status:** Merged · **Date:** 2026-07-01 · **Merged:** 2026-07-03 · **Owner:** Ant Stanley · **Target:** crates/core (exchange), crates/adapters (dynamo, postgres, sqlite)

Enforce the `(provider, external_id)` uniqueness invariant on DynamoDB with a
transactional uniqueness-guard item that also becomes the lookup path
(`get_user_by_external_id` moves to strongly consistent `GetItem`s, retiring the GSI1 user
entry); give the repository port a distinguishable `Conflict` error so the exchange flow's
lookup-then-create race resolves to a re-lookup instead of a 500; retry DynamoDB
`BatchWriteItem` unprocessed items so revoke-all and cleanup actually delete every session;
make `update_user` concurrency-safe with an explicit integer `version` instead of an
unguarded read-modify-write; and free a deleted user's external id for re-registration on
every backend.

---

## Motivation

[01-domain-model.md](../service/specs/01-domain-model.md) assumes "a user is uniquely keyed
by `(provider, external_id)`". Postgres and SQLite enforce it with a unique index
(`crates/adapters/src/postgres/mod.rs:28`, `sqlite/mod.rs:28`), but the DynamoDB adapter's
`create_user` (`dynamo/mod.rs:84-113`) conditions only on `attribute_not_exists(pk)` for a
freshly minted `usr_{ulid}` — a condition that never fires. Two concurrent first logins
create two Dynamo users for one subject, and `get_user_by_external_id` (`limit(1)`,
`dynamo/mod.rs:72`) thereafter returns an arbitrary one. On the SQL backends the index
holds, but the exchange flow's lookup-then-create (`crates/core/src/service/exchange.rs:85-137`)
has no conflict handling: the losing racer's insert violates the index and a legitimate
first login gets a `StoreError` 500.

Two further write-integrity gaps: `BatchWriteItem` responses' `unprocessed_items` are
silently dropped in `cleanup_expired_sessions` and `revoke_all_user_sessions`
(`dynamo/mod.rs:392-399`, `:466-471`) — under throttling, revoke-all returns `Ok(())` with
live sessions remaining, and it backs user delete and (proposed) suspend/compromise paths.
And `update_user` is an unguarded read-modify-write in all three adapters
(`dynamo/mod.rs:116-153`, `postgres/mod.rs:224-270`, `sqlite/mod.rs:254-307`): an admin
suspend racing a claims patch can be silently reverted to `active`.

---

## Affected spec pages

| Canonical page                                                                               | Nature of change                                                                                                    |
| -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md)             | Assumptions bullet reworded: at most one **non-deleted** user per `(provider, external_id)`; the `User` struct listing gains the `version` field; Deleted lifecycle prose gains the freed-identity claim |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md)               | DynamoDB section: uniqueness-guard item, transactional `create_user`, guard-based `get_user_by_external_id` (GSI1 user entry retired — the provider-prefix sentence is rewritten for the guard item), guard delete on user deletion, batch-delete retry (replacing the `cleanup_expired_sessions` no-op-cost sentence), version-conditional `update_user`. PostgreSQL / SQLite section: partial unique index excluding deleted rows, deleted rows excluded from lookup, `version` column |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | `UserRepository` contract: `create_user` conflict semantics; version-based `update_user` atomicity; deletion frees the external id                                              |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md)           | Exchange step 3: conflict on JIT create → re-lookup and continue; `UserCreated` audited only on the winning create                                                              |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md)                     | Error mapping table: `Conflict` → 409 `conflict`                                                                    |
| [`.specs/service/specs/canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | `$defs.User` gains required `version`                                                                          |
| [`.specs/canonical-types.schema.json`](../canonical-types.schema.json)                       | `$defs.OAuthErrorEnvelope` `error` enum gains `conflict`                                                            |
| [`schemas/dynamodb/table-design.json`](../../schemas/dynamodb/table-design.json)             | Guard item added; the User item's GSI1 entry retired; `get_user_by_external_id` access pattern rewritten            |
| [`schemas/datamodel.schema.json`](../../schemas/datamodel.schema.json)                       | `definitions.User` gains required `version` — the adapter-agnostic logical model that 08-persistence.md declares canonical-types mirrors |

---

## Proposed changes

### `.specs/service/specs/01-domain-model.md` → Entities → User (Modify)

The struct listing gains `version`, and the paragraph below it gains the store-managed
sentence:

> ```rust
> struct User {
>     id: String,                       // "usr_…"
>     external_id: String,              // provider sub / DID
>     provider: String,                 // "google", "apple"
>     email: Option<String>,
>     display_name: Option<String>,
>     metadata: HashMap<String, Value>, // extensible sync data
>     claims: HashMap<String, Value>,   // per-user private claims merged into the access JWT
>     status: UserStatus,
>     version: u64,                     // optimistic-concurrency counter; 1 on create
>     created_at: DateTime<Utc>,
>     updated_at: DateTime<Utc>,
> }
> ```
>
> `version` is store-managed, never caller-supplied: `create_user` writes `1`, every
> `update_user` increments it, and the write is conditioned on the version that was read
> (see [08-persistence.md](08-persistence.md)). It appears in neither `NewUser` nor
> `UserPatch`.

### `.specs/service/specs/01-domain-model.md` → Lifecycles → User status (Modify)

The `Deleted` bullet keeps its "row is kept" claim and gains the counter-balancing
freed-identity claim:

> - **Deleted** — soft delete via `UserPatch { status: Deleted }`; the service revokes all
>   the user's sessions on delete. The row is kept, but the identity is freed:
>   `get_user_by_external_id` no longer returns the deleted user, and a later first login
>   for the same `(provider, external_id)` re-registers as a brand-new user with no claims
>   or sessions carried over.

### `.specs/service/specs/01-domain-model.md` → Assumptions (Modify)

> - At most one **non-deleted** user exists per `(provider, external_id)`; provider
>   namespacing prevents cross-provider subject collisions. Deleting a user frees the
>   identity for re-registration — the deleted record is retained but no longer occupies
>   the key.

### `.specs/service/specs/08-persistence.md` → DynamoDB (Modify)

> | Item                  | pk                             | sk        | GSI1pk           | GSI1sk                 |
> | --------------------- | ------------------------------ | --------- | ---------------- | ---------------------- |
> | User                  | `USER#<id>`                    | `PROFILE` | —                | —                      |
> | User uniqueness guard | `EXT#<provider>#<external_id>` | `UNIQUE`  | —                | —                      |
> | Session               | `SESSION#<refresh_token_hash>` | `SESSION` | `USER#<user_id>` | `SESSION#<created_at>` |
>
> Access patterns:
>
> | Operation | DynamoDB call |
> |---|---|
> | `get_user_by_id` | `GetItem` on `USER#<id>` / `PROFILE` |
> | `get_user_by_external_id` | two strongly consistent `GetItem`s: `EXT#<provider>#<external_id>` / `UNIQUE`, then `USER#<user_id>` / `PROFILE` |
> | `get_session_by_refresh_token` | `GetItem` on `SESSION#<hash>` |
> | `revoke_all_user_sessions` | `Query` GSI1 `USER#<user_id>` then `BatchWrite` deletes (unprocessed items retried) |
> | `count_by_status` / `count_active_sessions` | table scans / counts |
>
> `create_user` is a `TransactWriteItems` of two puts: the user item and a uniqueness-guard
> item keyed on the external identity, each conditioned on `attribute_not_exists(pk)`. The
> guard item carries the owning `user_id` and makes `(provider, external_id)` unique at
> write time — a cancelled transaction (guard already present) surfaces as `Conflict`.
>
> `get_user_by_external_id` is two strongly consistent `GetItem`s: the guard item resolves
> the external identity to a `user_id`, then the profile is read at `USER#<id>` / `PROFILE`.
> The GSI1 user entry (`EXT#…` / `USER`) is retired; GSI1 serves only session lookups.
>
> `delete_user` (the `Deleted` transition) deletes the guard item in the same
> `TransactWriteItems` as the status write, freeing the external id: a later first login for
> the same subject re-registers as a brand-new user rather than finding the deleted row.
> (The SQL backends match; see the PostgreSQL / SQLite block below.)
>
> `revoke_all_user_sessions` and `cleanup_expired_sessions` retry any `unprocessed_items`
> returned by `BatchWriteItem` with exponential backoff until the batch drains or a bounded
> retry budget is exhausted (then error); a successful return means every targeted session
> item was deleted.
>
> `update_user` writes conditionally on the integer `version` read at the start of the
> read-modify-write (`ConditionExpression: version = :read_version`, a missing attribute
> counting as the migration default `1`), incrementing `version` on every write and retrying
> the read-modify-write on condition failure; a lost update cannot silently revert a
> concurrent status change.

Two sentences in the section's closing paragraph become false once the above merges and
are rewritten. "The GSI1 user key includes the provider prefix so the same external id
from two providers does not collide" (the GSI1 user entry is retired) and
"`cleanup_expired_sessions` is a no-op cost there" (it now drains `BatchWriteItem`
retries) become:

> `metadata` and `claims` are stored as DynamoDB maps; `created_at`/`updated_at`/`expires_at`
> are ISO-8601 strings. Sessions carry a numeric `ttl` (epoch seconds) so DynamoDB expires
> them natively — `cleanup_expired_sessions` batch-deletes whatever TTL has not yet reaped,
> retrying unprocessed items as above, so a successful return means every expired session
> found is gone. The guard item's `pk` (`EXT#<provider>#<external_id>`) includes the
> provider prefix so the same external id from two providers does not collide.

### `.specs/service/specs/08-persistence.md` → PostgreSQL / SQLite (Modify)

In the PostgreSQL section, the sentence "indexes cover `(external_id, provider)` and
`sessions.user_id`" becomes:

> `users` carries a `version BIGINT NOT NULL DEFAULT 1` column backing version-conditional
> `update_user`; indexes cover `sessions.user_id` and — as a **partial unique index**
> excluding deleted rows
> (`CREATE UNIQUE INDEX … ON users (external_id, provider) WHERE status != 'deleted'`) —
> `(external_id, provider)`, so at most one non-deleted user exists per identity while a
> deleted identity is free to re-register. A unique violation on insert maps to
> `Error::Conflict`. `get_user_by_external_id` filters `status != 'deleted'`, so a deleted
> row is never returned to the exchange flow — matching DynamoDB, where the guard item is
> gone.

The SQLite section gains:

> The same `version` column, partial unique index, and deleted-row exclusion as PostgreSQL
> apply; SQLite's unique-violation code maps to `Error::Conflict`.

### `schemas/dynamodb/table-design.json` (Modify)

The physical-layout sidecar referenced by 08-persistence.md changes in three places:

> - `item_schemas.User` loses its `GSI1pk` / `GSI1sk` keys (the `EXT#…` / `USER` entry is
>   retired) and gains `"version": "N"` in `attributes`.
> - A new `item_schemas.UserUniquenessGuard` entry: `pk` = `EXT#<provider>#<external_id>`,
>   `sk` = `UNIQUE`, attributes `{ "user_id": "S" }`, no GSI keys.
> - `access_patterns.get_user_by_external_id` becomes two strongly consistent `GetItem`s
>   (`EXT#<provider>#<external_id>` / `UNIQUE` → `USER#<user_id>` / `PROFILE`) instead of
>   the GSI1 `Query`; GSI1 serves only session lookups.

Note for the merger: the sidecar's *current* User `GSI1pk` is `EXT#<external_id>` — it
lacks the provider prefix that 08-persistence.md documents. This pre-existing divergence
disappears with the retirement (the guard item's `pk` carries the provider prefix), but do
not expect the deleted keys to match the spec page's wording.

### `.specs/service/specs/02-ports-and-adapters.md` → Port traits → UserRepository (Modify)

> `create_user` fails with `Error::Conflict` when a live user with the same
> `(provider, external_id)` already exists — every adapter maps its native uniqueness
> violation (SQL unique index, DynamoDB transaction cancellation) to this variant so callers
> can distinguish "already registered" from an infrastructure failure. `update_user` applies
> a patch atomically with respect to concurrent updates using the user's integer `version`:
> the write is conditioned on the version that was read and increments it, so two racing
> patches serialize and neither silently overwrites the other's fields. `delete_user` frees
> the `(provider, external_id)` key: after a delete, `get_user_by_external_id` returns
> nothing for that identity and `create_user` succeeds as a new user.

### `.specs/service/specs/03-service-flows.md` → Token exchange (Modify)

> 3. … Otherwise (`mode == "open"`) → `create_user(NewUser{…})` (audited `UserCreated`);
>    if creation returns `Conflict` (a concurrent first login won the race), re-run
>    `get_user_by_external_id` and continue with the existing user, re-applying the
>    suspended-status check. The losing racer emits no `UserCreated` event — the winning
>    create already audited it — and the flow otherwise proceeds as for a found user.

The "(audited `UserCreated`)" annotation is retained deliberately:
[2026-07-01-wire_audit_event_emission.md](2026-07-01-wire_audit_event_emission.md) also
touches this bullet, and the two deltas compose only if this block keeps the annotation
in place.

### `.specs/service/specs/04-http-api.md` → Error mapping (Modify)

> | `Conflict` | 409 | `conflict` |

---

## Type changes

`User` gains a monotonically increasing `version` counter for optimistic concurrency, in
the service `canonical-types.schema.json` (`$defs.User`) and the `01-domain-model.md`
struct listing:

```json
{
  "$comment": "Fragment for 2026-07-01-fix_user_creation_race_and_dynamo_integrity. Folds into .specs/service/specs/canonical-types.schema.json $defs.User on merge.",
  "version": {
    "type": "integer",
    "minimum": 1,
    "description": "Optimistic-concurrency counter; starts at 1 on create, incremented by every update_user."
  }
}
```

`version` joins `User`'s `required` list. It is store-managed, never caller-supplied, so
`NewUser` and `UserPatch` are unchanged. Migration default is `1`: SQL backends add
`ALTER TABLE users ADD COLUMN version BIGINT NOT NULL DEFAULT 1`; Dynamo treats a missing
attribute as `1` at read time.

The same property (identical shape) also folds into `definitions.User` in
[`schemas/datamodel.schema.json`](../../schemas/datamodel.schema.json), joining its
`required` list — 08-persistence.md declares that logical model the cross-adapter source of
truth which canonical-types mirrors, and the mirror claim only survives the merge if both
files gain `version`.

The new 409 wire code `conflict` (04-http-api.md error-mapping row) must also join the
closed `error` enum of the repo-wide `OAuthErrorEnvelope`
([`.specs/canonical-types.schema.json`](../canonical-types.schema.json)) — without this
the envelope's schema rejects the new body:

```json
{
  "$comment": "Fragment for 2026-07-01-fix_user_creation_race_and_dynamo_integrity. Folds into the repo-wide .specs/canonical-types.schema.json $defs.OAuthErrorEnvelope on merge.",
  "properties": {
    "error": {
      "type": "string",
      "enum": [
        "invalid_grant",
        "invalid_token",
        "invalid_request",
        "access_denied",
        "unauthorized",
        "unsupported_grant_type",
        "conflict",
        "server_error"
      ]
    }
  }
}
```

`Error::Conflict` is a new error-enum variant in `crates/core/src/error.rs`, not a
canonical type.

---

## Implementation notes

1. Add `Error::Conflict { detail }` to `crates/core/src/error.rs`; map it in
   `crates/server/src/error.rs:51-108` (409, wire code `conflict` — see Type changes for
   the envelope-enum addition) and any FFI error tables.
2. `crates/adapters/src/postgres/mod.rs` / `sqlite/mod.rs` `create_user` — inspect the
   `sqlx` database error for a unique-violation code (Postgres `23505`; SQLite `2067`) and
   return `Conflict` instead of `StoreError`.
3. `crates/adapters/src/dynamo/mod.rs:84-113` — replace `put_item` with
   `transact_write_items` (user + `EXT#<provider>#<external_id>` / `UNIQUE` guard carrying
   `user_id`, both `attribute_not_exists(pk)`); map `TransactionCanceledException` with
   `ConditionalCheckFailed` reasons to `Conflict`. Backfill guard items for existing users
   (one-off script or lazy write) before relying on the invariant.
4. `crates/adapters/src/dynamo/mod.rs:56-81` — replace the GSI1 `Query` in
   `get_user_by_external_id` with a `consistent_read(true)` `GetItem` on
   `EXT#<provider>#<external_id>` / `UNIQUE` followed by a `GetItem` on `USER#<id>` /
   `PROFILE`; drop the user item's GSI1 attributes from `user_to_item`
   (`crates/adapters/src/dynamo/schema.rs:22-27`). The guard backfill in note 3 must
   complete before this lookup ships — a guard-less existing user would otherwise become
   invisible.
5. `crates/core/src/service/exchange.rs:131-138` — wrap the `create_user` call: on
   `Conflict`, re-run the lookup at `exchange.rs:85-89` and take the found-user branch.
6. `dynamo/mod.rs:392-399` and `:466-471` — loop while the response's `unprocessed_items`
   is non-empty, re-submitting with capped exponential backoff (e.g. 8 attempts) and
   erroring when the budget is exhausted; in `cleanup_expired_sessions` count deletions from
   drained batches, not submitted requests (`dynamo/mod.rs:392`).
7. `update_user`: Dynamo (`:116-153`) — store `version` on the item and write with
   `condition_expression("version = :read_version OR attribute_not_exists(version)")`
   setting `version = read + 1`, retrying the read-modify-write on condition failure;
   Postgres/SQLite (`postgres/mod.rs:224-270`, `sqlite/mod.rs:254-307`) —
   `UPDATE … SET version = version + 1 … WHERE id = $1 AND version = $2`, retrying on zero
   rows.
8. Deletion frees the external id: on Dynamo, any write that sets `status = Deleted` — the
   `update_user` patch path, which also backs `delete_user` (`dynamo/mod.rs:156-169`) —
   becomes a `TransactWriteItems` of the versioned status write plus a `Delete` of the
   guard item.
   Postgres/SQLite — replace the full unique index (`postgres/mod.rs:28`, `sqlite/mod.rs:28`)
   with a partial unique index
   (`CREATE UNIQUE INDEX … ON users (external_id, provider) WHERE status != 'deleted'`;
   both engines support partial indexes) and add `AND status != 'deleted'` to
   `get_user_by_external_id` (`postgres/mod.rs:178`, `sqlite/mod.rs:195`) so a deleted row
   is never returned to the exchange flow — matching Dynamo, where the guard is gone.
9. SQL migrations: `ALTER TABLE users ADD COLUMN version BIGINT NOT NULL DEFAULT 1` and the
   index swap (`DROP INDEX idx_users_external_id_provider` + partial recreate) need explicit
   steps for existing tables — the inline `CREATE … IF NOT EXISTS` DDL
   (`postgres/mod.rs:20-40`, `sqlite/mod.rs:20-40`) only covers fresh databases.
10. Tests: concurrent `create_user` (Dynamo Local + SQL) yields one user and one `Conflict`;
    exchange under the race returns a token, not 500; unprocessed-item retry drains
    (mockable via Dynamo Local is limited — unit-test the retry loop); racing suspend +
    claims patch ends suspended (version conflict retried); delete then re-login creates a
    fresh user on all three backends; `get_user_by_external_id` never returns a deleted
    user.

---

## Merge plan

1. Apply the 01, 08, 02, 03, and 04 blocks to their canonical pages; bump each page's
   `**Date:**`.
2. Apply the `schemas/dynamodb/table-design.json` block: add the `UserUniquenessGuard`
   item schema, retire the User item's GSI1 entry, and rewrite the
   `get_user_by_external_id` access pattern.
3. Fold the first Type-changes fragment into `$defs.User` in
   `.specs/service/specs/canonical-types.schema.json` (add `version` to `properties` and
   `required`), and the same property into `definitions.User` in
   `schemas/datamodel.schema.json` (`properties` + `required`).
4. Fold the second Type-changes fragment into
   `$defs.OAuthErrorEnvelope.properties.error.enum` in the repo-wide
   `.specs/canonical-types.schema.json` (add `conflict`).
5. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
6. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- The DynamoDB table's write capacity tolerates `create_user` becoming a 2-item transaction
  (double the write units of the current put).
- Existing production tables contain no duplicate `(provider, external_id)` users; if scans
  find any, dedup is a manual migration before the guard backfill.

### Decisions

- _Guard item over GSI condition._ **Uniqueness is enforced by a base-table guard item in a
  transaction.** DynamoDB conditions cannot span a GSI; a canonical item keyed on the
  external identity is the standard single-table uniqueness pattern.
- _Conflict is a first-class port error._ **Adapters map native uniqueness violations to
  `Error::Conflict`.** String-matching `StoreError` details in core would couple the service
  to driver messages. Its 409 wire code `conflict` is added to the closed `error` enum of
  the repo-wide `OAuthErrorEnvelope` (see Type changes) — RFC 6749 permits extension codes,
  but the canonical enum must name every code the API can emit.
- _The losing racer does not audit `UserCreated`._ **When JIT creation returns `Conflict`,
  the flow re-looks-up and continues without emitting `UserCreated`.** Exactly one create
  happened, so exactly one event is emitted — by the winner. Composes with
  [2026-07-01-wire_audit_event_emission.md](2026-07-01-wire_audit_event_emission.md), which
  wires the annotation's emission.
- _External-id lookup via the guard._ **`get_user_by_external_id` on Dynamo is two strongly
  consistent `GetItem`s (guard → profile), retiring the GSI1 user entry.** The guard item
  must exist for uniqueness anyway; reading it removes GSI eventual-consistency lag from the
  exchange hot path, so this is folded into the change rather than left as a follow-up.
- _Explicit version counter._ **`User` carries a monotonically increasing integer `version`;
  `update_user` conditions on the version it read and increments it.** `updated_at` has
  second precision and can collide under concurrency; an integer is unambiguous at the cost
  of a small schema migration (default `1` for existing rows/items).
- _Deletion frees the external id._ **`delete_user` removes the Dynamo guard item, and the
  SQL unique index becomes partial (`WHERE status != 'deleted'`) with deleted rows excluded
  from lookup.** A deleted subject can re-register as a brand-new user with no claims or
  sessions carried over, consistent with `Deleted` being terminal per
  [2026-07-01-enforce_user_lifecycle_transitions.md](2026-07-01-enforce_user_lifecycle_transitions.md).

### Open questions

- (None at this stage.)
