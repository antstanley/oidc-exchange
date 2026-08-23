# Persistence

**Status:** Implemented · **Date:** 2026-08-23 · **Owner:** Ant Stanley · **Scope:** crates/adapters storage, schemas/

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
| Session | `SESSION#<refresh_token_hash>` | `SESSION` | `USER#<user_id>` | `SESSION#<created_at>` |

GSI1 is retired for the User item and serves only session lookups (`list_user_sessions`,
`revoke_all_user_sessions`); a user is never looked up through GSI1.

Access patterns:

| Operation | DynamoDB call |
|---|---|
| `get_user_by_id` | `GetItem` on `USER#<id>` / `PROFILE` |
| `get_user_by_external_id` | two strongly consistent `GetItem`s: the guard (`EXT#<provider>#<external_id>` / `UNIQUE`) to resolve `user_id`, then `USER#<user_id>` / `PROFILE` |
| `get_session_by_refresh_token` | `GetItem` on `SESSION#<hash>` |
| `revoke_all_user_sessions` | `Query` GSI1 `USER#<user_id>` then `BatchWrite` deletes (unprocessed items retried) |
| `count_by_status` / `count_active_sessions` | table scans / counts |

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
with SQLSTATE `42501` (`insufficient_privilege`) — `create_pool` degrades rather than failing
outright: it logs a structured warning and probes `to_regclass('users')` /
`to_regclass('sessions')`, returning the pool when both already exist (a schema pre-provisioned
by an out-of-band process) and failing startup with the original migration error when either is
missing. Every other migration failure still fails fast. Implements both repository traits.

The `(external_id, provider)` index is a *partial* unique index, `WHERE status !=
'deleted'`: uniqueness is enforced only among live users, so a soft-deleted row frees its
identity for re-registration rather than permanently occupying the slot. Because
`CREATE UNIQUE INDEX IF NOT EXISTS` cannot turn a pre-existing full index into a partial
one, the inline DDL also runs an explicit `DROP INDEX IF EXISTS idx_users_external_id_provider`
immediately before recreating it with the `WHERE` predicate — idempotent on both a fresh
database (the `DROP` is a no-op) and one that predates this migration (the full index is
replaced). `get_user_by_external_id` adds `AND status != 'deleted'` so a deleted row is
never returned to the exchange flow, matching the index's own predicate.

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
way as Postgres's `23505`. Implements both repository traits — the zero-dependency
single-host option.

## Session-only stores

- **LMDB (`adapters/lmdb`)** — embedded `heed` store with two named databases, `sessions`
  (hash → session) and `user_sessions` (user → set of hashes for revoke-all). Constructed with
  a path and a max map size in MB.
- **Valkey/Redis (`adapters/valkey`)** — `fred` client; keys `{prefix}session:{hash}`, a
  `{prefix}user_sessions:{user_id}` set, and a `{prefix}active_sessions` counter. A session
  write applies the hash, its TTL, the user-set membership, an `INCR` of the counter, and a
  bump of the user set's own TTL to the greatest member expiry — atomically (single
  pipeline). The set-TTL bump uses `EXPIRE … GT` (only-extend), so a concurrent
  shorter-lived write can never shorten the set's life, and idle users' index sets expire on
  their own. A session whose `expires_at` is not in the future is rejected, so no TTL-less
  key is ever created. `count_active_sessions` reads the counter, which is maintained by
  `INCR` on store and `DECR` on explicit revoke; natural TTL expiry cannot decrement it, so
  it drifts upward between cleanups. `cleanup_expired_sessions` prunes `user_sessions` set
  members whose session key no longer exists, reconciles the counter by recomputing it from
  a SCAN of live `{prefix}session:*` keys, and returns the number of members pruned; session
  bodies themselves need no sweep.

Both implement `SessionRepository` only and are selected via `[session_repository]`.

Every session adapter instruments its three session methods identically:
`#[instrument(skip(self, session), fields(user_id = %session.user_id))]` on the write path and
`#[instrument(skip(self, token_hash), fields(token_hash))]` on the lookup and revoke paths. The
token hash and the session's client provenance (`ip_address`, `user_agent`, `device_id`) never
become span field values on any backend.

## Logical schema (`schemas/datamodel.schema.json`)

The adapter-agnostic contract every store satisfies, defining `User`, `Session`, and
`AuditEvent` with their required fields and the `status` / `severity` / `outcome` enums. It is
the cross-adapter source of truth; the service's typed entities and
[canonical-types.schema.json](canonical-types.schema.json) mirror it.

## Assumptions and open questions

### Assumptions

- DynamoDB TTL is enabled on the `ttl` attribute in deployments that rely on automatic session
  expiry; otherwise an external scheduler calls `cleanup_expired_sessions`.
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
