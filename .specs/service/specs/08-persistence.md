# Persistence

**Status:** Implemented · **Date:** 2026-08-22 · **Owner:** Ant Stanley · **Scope:** crates/adapters storage, schemas/

How the domain entities ([01-domain-model.md](01-domain-model.md)) are stored by each
repository adapter. The adapter-agnostic logical model is `schemas/datamodel.schema.json`; the
DynamoDB physical layout is `schemas/dynamodb/table-design.json`.

## DynamoDB (`adapters/dynamo`)

Single-table design with one global secondary index (`GSI1`). Keys `pk`/`sk`, GSI keys
`GSI1pk`/`GSI1sk` (projection `ALL`).

| Item | pk | sk | GSI1pk | GSI1sk |
|---|---|---|---|---|
| User | `USER#<id>` | `PROFILE` | — | — |
| User uniqueness guard | `EXT#<provider>#<external_id>` | `UNIQUE` | — | — |
| Session | `SESSION#<refresh_token_hash>` | `SESSION` | `USER#<user_id>` | `FAM#<family_id>#SESSION#<created_at>` |
| Retired refresh token | `RETIRED#<refresh_token_hash>` | `RETIRED` | `USER#<user_id>` | `FAM#<family_id>#RETIRED#<retired_at>` |

GSI1 is retired for the User item; it remains for the admin listing paths, where a stale
read is a cosmetic defect rather than a surviving credential — the revocation paths read
the per-user roster on the user item instead (below), and a user is never looked up through
GSI1.

Access patterns:

| Operation | DynamoDB call |
|---|---|
| `get_user_by_id` | `GetItem` on `USER#<id>` / `PROFILE` |
| `get_user_by_external_id` | two strongly consistent `GetItem`s: the guard (`EXT#<provider>#<external_id>` / `UNIQUE`) to resolve `user_id`, then `USER#<user_id>` / `PROFILE` |
| `get_session_by_refresh_token` | `GetItem` on `SESSION#<hash>` |
| `resolve_refresh_token` | strongly consistent `GetItem`s on both `SESSION#<hash>` and `RETIRED#<hash>` |
| `rotate_refresh_token` | one `TransactWriteItems`: conditioned delete of the live session, put of the retirement item, conditioned put of the replacement |
| `revoke_family` / `revoke_all_user_sessions` | strongly consistent `GetItem` of the user item's roster, then `BatchWrite` deletes (unprocessed items retried) |
| `count_by_status` / `count_active_sessions` | table scans / counts |

`resolve_refresh_token` issues strongly consistent `GetItem`s — `consistent_read(true)` on
both the `SESSION#` and the `RETIRED#` lookup, matching what `get_user_by_external_id`
already does on the identity path. Both answers carry a security decision: an eventually
consistent `SESSION#` read lets a revoked token mint an access token for the width of the
replication window, and an eventually consistent `RETIRED#` read reports reuse as an unknown
token, which is refused but raises no alarm.

`rotate_refresh_token` is one `TransactWriteItems`: a `Delete` of the live session item
conditioned on `attribute_exists(pk)`, a `Put` of the retirement item, and a `Put` of the
replacement session conditioned on `attribute_not_exists(pk)`. A `TransactionCanceledException`
whose reasons include `ConditionalCheckFailed` means the live generation moved and maps to a
`false` return, not an error; any other transaction failure maps to `Error::StoreError`.
Retirement items carry the same numeric `ttl` attribute as sessions, so DynamoDB reaps them
natively. `revoke_family` deletes by family id from the same roster (below).

**The GSI is an index, not the roster.** GSI1 is eventually consistent, so a session written
moments before a revocation can be absent from the index at query time and survive the sweep
permanently — the token stays live with nothing left to find it. Enumeration for the
revocation paths therefore reads an authoritative list instead: the user item
(`pk = USER#<user_id>`, `sk = USER`) carries a `sessions` attribute, a string set of the live
`refresh_token_hash` values, and a `families` attribute mapping each `family_id` to its
member hashes. Every write that creates or removes a session updates that list **inside the
same `TransactWriteItems`** as the session item itself — creation on exchange, the
delete-plus-put of a rotation, retirement, and revocation — so the list and the items cannot
disagree, and a strongly consistent `GetItem` on the user item is a complete roster.
`revoke_family` and `revoke_all_user_sessions` read that roster with `consistent_read(true)`,
delete exactly the hashes it names, and return the count. The cost is stated plainly: every
session write becomes a transaction touching the user item, so a single user's concurrent
logins now contend on one item, and that item grows with live session count. That is the
price of a revocation that means what it says; SR5 in the conformance suite
([02-ports-and-adapters.md](02-ports-and-adapters.md)) is what proves it.

`metadata` and `claims` are stored as DynamoDB maps; `created_at`/`updated_at`/`expires_at`
are ISO-8601 strings. Sessions carry a numeric `ttl` (epoch seconds) so DynamoDB expires them
natively — `cleanup_expired_sessions` batch-deletes whatever TTL has not yet reaped, retrying
any `BatchWriteItem` `unprocessed_items` with capped exponential backoff until the batch drains
or a bounded retry budget is exhausted (then error), so a successful return means every expired
session found is gone. `revoke_all_user_sessions` retries the same way, so a successful return
means every targeted session item was deleted. The provider prefix that keeps the same external
id from two providers from colliding now lives on the guard item's `pk` (`EXT#<provider>#<external_id>`),
since the User item no longer carries a GSI1 key.

`create_user` is a `TransactWriteItems` of two `Put`s — the user item and a uniqueness-guard
item (`EXT#<provider>#<external_id>` / `UNIQUE`, attribute `user_id`) — each conditioned on
`attribute_not_exists(pk)`, making `(provider, external_id)` unique at write time. A
`TransactionCanceledException` whose cancellation reasons include `ConditionalCheckFailed`
means the guard already existed (a racing duplicate `create_user`) and is mapped to
`Error::Conflict`; any other transaction failure (e.g. a missing table, throttling) maps to
`Error::StoreError`. Guard items for users written before this invariant existed are backfilled
by `DynamoRepository::backfill_uniqueness_guards`, a one-off, idempotent migration step (each
write conditioned on `attribute_not_exists(pk)`, so re-running after a partial failure is safe).
Deployments must run that backfill to completion — leaving no user unguarded — before deploying
this guard-based `get_user_by_external_id`; a guard-less pre-existing user is invisible to it
even though its profile item still exists.

`update_user` writes conditionally on the integer `version` read at the start of the
read-modify-write (`ConditionExpression: version = :read_version OR attribute_not_exists(version)`,
a missing attribute counting as the migration default `1`), incrementing `version` on every
write; a `ConditionalCheckFailedException` means a concurrent writer already advanced the
item's `version`, and the read-modify-write is retried against the fresh value up to
`UPDATE_MAX_ATTEMPTS` before erroring — a lost update cannot silently revert a concurrent
status change.

When a patch transitions `status` to `Deleted`, `update_user` writes the same
version-conditioned user item through a `TransactWriteItems` call instead of a plain
`PutItem`, adding a `Delete` of the uniqueness-guard item
(`EXT#<provider>#<external_id>` / `UNIQUE`) as the transaction's second item. Both writes
succeed or neither does, so a reader can never observe a `Deleted` user whose guard is
still standing (which would keep blocking re-registration) or a removed guard whose
status write did not land. Any other patch (no status change, or a change to `Active`/
`Suspended`) keeps the cheaper plain versioned `PutItem` — the guard is only ever touched
on the transition into `Deleted`. Removing the guard is what frees `(provider,
external_id)`: with no guard item, `get_user_by_external_id` returns `None` for that
identity and a subsequent `create_user` for the same pair writes a fresh guard rather than
losing to the guard's `attribute_not_exists(pk)` condition. The deleted user's profile item
itself is never removed — only its guard.

## PostgreSQL (`adapters/postgres`)

Two tables via `sqlx`: `users` and `sessions`. `metadata` and `claims` are `JSONB`; `users`
carries a `version BIGINT NOT NULL DEFAULT 1` column — a store-managed optimistic-concurrency
counter that `create_user` writes as `1` and every read carries through unchanged. The
migration DDL adds the column with an idempotent `ALTER TABLE … ADD COLUMN IF NOT EXISTS`
step alongside the inline `CREATE TABLE`, so a `users` table that predates the column gets
it and an existing row reads back as `1`. Indexes cover `(external_id, provider)` and
`sessions.user_id`. A unique violation on insert (SQLSTATE `23505`) maps to
`Error::Conflict` rather than `Error::StoreError`, so a racing duplicate `create_user` is
distinguishable from an infrastructure failure. `create_pool(url, max_connections,
run_migrations)` builds the connection pool and, unless `run_migrations` is `false`, executes
the adapter's idempotent migrations (`CREATE TABLE IF NOT EXISTS …`, run via sqlx's raw
simple-query path since the migration DDL is multi-statement) before returning — like
SQLite, a fresh database is ready to serve after startup with no external migration step.
With `run_migrations = false`, `create_pool` only connects, leaving DDL to an out-of-band
process — for locked-down deployments where the app role has no DDL rights. When the migration
is instead denied by Postgres itself — the connected role lacks DDL rights and the DDL fails
with SQLSTATE `42501` (`insufficient_privilege`) — `create_pool` degrades only after verifying
the invariants the migration would have established. It logs a structured warning and probes
for the `users` and `sessions` tables; the `idx_users_external_id_provider` index, which must
exist, be **unique**, and be **partial** (`indisunique` and a non-null `indpred` in `pg_index`);
and the `users.version` column. The pool is returned only when every probe passes. If any is
missing or the probe itself fails, `create_pool` returns the **original** migration error and
startup fails — an inconclusive probe must not mask denied DDL. Table presence alone is not
sufficient: the partial unique index is the only enforcer of one live user per
`(provider, external_id)`, and the registration path depends on the database raising `23505`.
Every other migration failure still fails fast. Implements both repository traits.

The `(external_id, provider)` index is a *partial* unique index, `WHERE status !=
'deleted'`: uniqueness is enforced only among live users, so a soft-deleted row frees its
identity for re-registration rather than permanently occupying the slot. Because
`CREATE UNIQUE INDEX IF NOT EXISTS` cannot turn a pre-existing full index into a partial
one, the inline DDL also runs an explicit `DROP INDEX IF EXISTS idx_users_external_id_provider`
immediately before recreating it with the `WHERE` predicate — idempotent on both a fresh
database (the `DROP` is a no-op) and one that predates this migration (the full index is
replaced). `get_user_by_external_id` adds `AND status != 'deleted'` so a deleted row is
never returned to the exchange flow, matching the index's own predicate.

`sessions` carries `family_id TEXT`, `generation INTEGER NOT NULL DEFAULT 0` and
`rotated_at` alongside the existing columns, added by the same idempotent `ALTER TABLE` step
that added `users.version` (`ADD COLUMN IF NOT EXISTS` on Postgres, a bare `ADD COLUMN` on
SQLite). `family_id` is nullable: a session row written before rotation shipped needs no
backfill, and its first redemption mints a family for the replacement and deletes the legacy
row without writing a retirement record — there is no prior generation to detect reuse
against. A second table holds the retirement records:

```sql
CREATE TABLE IF NOT EXISTS retired_refresh_tokens (
    refresh_token_hash  TEXT PRIMARY KEY,
    family_id           TEXT NOT NULL,
    user_id             TEXT NOT NULL,
    successor_hash      TEXT NOT NULL,
    retired_at          TIMESTAMPTZ NOT NULL,
    expires_at          TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_retired_family ON retired_refresh_tokens (family_id);
CREATE INDEX IF NOT EXISTS idx_retired_expires_at ON retired_refresh_tokens (expires_at);
```

(On SQLite, `retired_at` and `expires_at` are `TEXT`, matching its `sessions` table.)
`rotate_refresh_token` runs its delete, retirement insert and replacement insert inside one
`BEGIN … COMMIT`. The compare-and-swap condition is the delete's affected-row count: zero
rows means the live generation moved, the transaction rolls back and the method returns
`false`. `cleanup_expired_sessions` sweeps both tables and its count covers both.

## SQLite (`adapters/sqlite`)

Single-file `sqlx` store with WAL mode and foreign-key enforcement; `metadata`/`claims` stored
as JSON `TEXT`. `users` carries the same store-managed `version INTEGER NOT NULL DEFAULT 1`
column as PostgreSQL; `create_pool(path)` runs the inline DDL and then an idempotent
`ALTER TABLE` step (SQLite's `ADD COLUMN` has no `IF NOT EXISTS` form) that adds the column
to a `users` table that predates it, defaulting existing rows to `1`. The same
partial `(external_id, provider)` unique index (`WHERE status != 'deleted'`, with the same
`DROP INDEX IF EXISTS` + recreate upgrade step for a database that predates it) and
`get_user_by_external_id` deleted-exclusion apply as Postgres, and SQLite's unique-violation
extended result code (`2067`, `SQLITE_CONSTRAINT_UNIQUE`) maps to `Error::Conflict` the same
way as Postgres's `23505`.

As in Postgres, `sessions` carries `family_id TEXT`, `generation INTEGER NOT NULL DEFAULT 0`
and `rotated_at` (added by the same idempotent `ALTER TABLE` upgrade step; nullable
`family_id`, legacy rows migrated on first redemption), a `retired_refresh_tokens` table
holds the retirement records — same DDL, with `retired_at`/`expires_at` as `TEXT` matching
its `sessions` table — rotation is one `BEGIN … COMMIT` whose compare-and-swap condition is
the delete's affected-row count, and `cleanup_expired_sessions` sweeps both tables with its
count covering both. Implements both repository traits — the zero-dependency single-host
option.

## Session-only stores

- **LMDB (`adapters/lmdb`)** — embedded `heed` store with five named databases: `sessions`
  (hash → session), `user_sessions` (user → set of hashes for revoke-all), `retired_tokens`
  (hash → retirement record), `family_index` (`{family_id}\0{hash}` → kind, for
  `revoke_family`), and `single_use` (digest key → RFC 3339 expiry). `rotate_refresh_token` performs all of its reads and writes inside a
  single `heed` write transaction, which is where its compare-and-swap condition is
  evaluated. Constructed with a path and a max map size in MB.
  `cleanup_expired_sessions` commits its deletes in fixed-size batches (256 keys per
  write transaction) rather than one all-or-nothing transaction. LMDB is copy-on-write,
  so a delete must allocate dirty pages before it frees old ones: the shipped
  single-transaction sweep itself fails `MDB_MAP_FULL` on a map filled past roughly 95%,
  which means a reaper wired up after the store has wedged cannot rescue it — recovery
  from a full map is raising `max_size_mb` and restarting, then reaping. Batching keeps
  the reaper effective up to the boundary; the scheduled reaper is what keeps a healthy
  deployment from ever reaching it.
- **Valkey/Redis (`adapters/valkey`)** — `fred` client; keys `{prefix}session:{hash}`,
  `{prefix}retired:{hash}` hashes, a `{prefix}family:{family_id}` set, and a
  `{prefix}user_sessions:{user_id}` set, all TTL'd like the session keys they accompany,
  plus a `{prefix}active_sessions` counter and `{prefix}single_use:{digest}` claims
  (`SET NX EX` / `GETDEL`, natively expiring). A session write applies the hash, its TTL, the
  user-set membership, an `INCR` of the counter, and a bump of the user set's own TTL —
  atomically (single pipeline). The rotation swap runs as one `EVAL`'d Lua script rather
  than a pipeline: it is conditional on the live hash still existing, and a pipeline gives
  batching without atomicity or a condition (the unconditional writes on the
  `store_refresh_token` path keep their pipeline). The set-TTL bump uses `EXPIRE … GT`
  (only-extend), so a concurrent shorter-lived write can never shorten the set's life, and
  idle users' index sets expire on their own. A session whose `expires_at` is not in the
  future is rejected, so no TTL-less key is ever created. `count_active_sessions` reads the
  counter, which is maintained by `INCR` on store and decrement on explicit revoke; natural
  TTL expiry cannot decrement it, so it drifts upward between cleanups. The counter is
  reconciled state, not an invariant the adapter establishes, so a decrement that would go
  negative **clamps the key to zero and emits one structured warning** (`counter_clamped =
  true`, with the observed value) instead of asserting — no counter comparison may unwind;
  the same rule binds `revoke_family` and the rotation script. `cleanup_expired_sessions`
  prunes `user_sessions` set members whose session key no longer exists, reconciles the
  counter by recomputing it from a SCAN of live `{prefix}session:*` keys, and returns the
  number of members pruned; session bodies themselves need no sweep.

Both implement `SessionRepository` only and are selected via `[session_repository]`. Every
session adapter stores retirement records alongside sessions and passes the session-store
conformance suite ([02-ports-and-adapters.md](02-ports-and-adapters.md)), which is what
makes rotation and reuse detection a property of the port rather than of whichever backend
a deployment happens to configure.

## Single-use records

Every store holds the [`SingleUseRecord`](01-domain-model.md) entities behind the
`SessionRepository` single-use pair, using each adapter's natural atomic primitive, so
`put_single_use` and `take_single_use` are one round trip everywhere:

| Adapter | Layout | `put_single_use` | `take_single_use` |
|---|---|---|---|
| DynamoDB | `pk = SINGLEUSE#<key>`, `sk = SINGLEUSE`, numeric `ttl` | `PutItem` conditioned on `attribute_not_exists(pk) OR expires_at < :now` | `DeleteItem` with `ReturnValues=ALL_OLD`, `expires_at` checked on the returned item |
| Postgres / SQLite | `single_use(key PRIMARY KEY, expires_at)` | `INSERT … ON CONFLICT (key) DO UPDATE … WHERE single_use.expires_at < now()`, rows affected reports the result | `DELETE … WHERE key = $1 AND expires_at > now() RETURNING 1` |
| Valkey | `{prefix}single_use:{key}` | `SET … NX EX <ttl>` | `GETDEL` |
| LMDB | `single_use` named DB | one write txn: read, treat an expired value as absent, write | one write txn: read, delete, report whether the value was live |

DynamoDB and Valkey expire records natively; Postgres, SQLite and LMDB rely on the
`cleanup_expired_sessions` sweep for space reclamation only — both operations already
evaluate `expires_at`, so an unswept record is never mistaken for a live one. Storage keeps
only the namespaced digest key and the expiry: no raw nonce or raw assertion material is
ever written.

## Logical schema (`schemas/datamodel.schema.json`)

The adapter-agnostic contract every store satisfies, defining `User`, `Session`,
`RetiredRefreshToken`, `AuditEvent`, and `SingleUseRecord` with their required fields and
the `status` / `severity` / `outcome` enums. It is the cross-adapter source of truth; the
service's typed entities and [canonical-types.schema.json](canonical-types.schema.json)
mirror it.

## Assumptions and open questions

### Assumptions

- DynamoDB TTL is enabled on the `ttl` attribute in deployments that rely on automatic
  expiry; long-lived runtimes schedule their own sweeps regardless (the session reaper,
  [04-http-api.md](04-http-api.md) → Bootstrap), and Lambda deployments drive
  `POST /internal/sessions/cleanup` from an external scheduler.
- The DynamoDB table and GSI1 exist before the service starts (created by the deployment's IaC,
  not by the adapter).

### Decisions

- *Single-table DynamoDB.* **Users and sessions share one table with GSI1.** Direct `GetItem`
  on the token hash for refresh, one query for revoke-all, and TTL-driven cleanup with no extra
  infrastructure.
- *JSON columns for extensible maps.* **`metadata`/`claims` are JSONB (Postgres) / TEXT JSON
  (SQLite) / maps (DynamoDB).** Schema-less extension fields without migrations per claim.
- *Embedded/in-memory session stores.* **LMDB and Valkey cover session-only topologies.** A
  single-host SQLite+LMDB or a fleet with SQL+Valkey can each be expressed in config.

### Open questions

- `count_by_status` / `count_active_sessions` on DynamoDB rely on scans; at large table sizes a
  maintained counter item or a stream-fed aggregate may be needed. Not yet addressed.
