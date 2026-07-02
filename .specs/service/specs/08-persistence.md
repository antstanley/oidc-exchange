# Persistence

**Status:** Implemented · **Date:** 2026-07-02 · **Owner:** Ant Stanley · **Scope:** crates/adapters storage, schemas/

How the domain entities ([01-domain-model.md](01-domain-model.md)) are stored by each
repository adapter. The adapter-agnostic logical model is `schemas/datamodel.schema.json`; the
DynamoDB physical layout is `schemas/dynamodb/table-design.json`.

## DynamoDB (`adapters/dynamo`)

Single-table design with one global secondary index (`GSI1`). Keys `pk`/`sk`, GSI keys
`GSI1pk`/`GSI1sk` (projection `ALL`).

| Item | pk | sk | GSI1pk | GSI1sk |
|---|---|---|---|---|
| User | `USER#<id>` | `PROFILE` | `EXT#<provider>#<external_id>` | `USER` |
| User uniqueness guard | `EXT#<provider>#<external_id>` | `UNIQUE` | — | — |
| Session | `SESSION#<refresh_token_hash>` | `SESSION` | `USER#<user_id>` | `SESSION#<created_at>` |

Access patterns:

| Operation | DynamoDB call |
|---|---|
| `get_user_by_id` | `GetItem` on `USER#<id>` / `PROFILE` |
| `get_user_by_external_id` | `Query` GSI1 `EXT#<provider>#<external_id>` |
| `get_session_by_refresh_token` | `GetItem` on `SESSION#<hash>` |
| `revoke_all_user_sessions` | `Query` GSI1 `USER#<user_id>` then `BatchWrite` deletes (unprocessed items retried) |
| `count_by_status` / `count_active_sessions` | table scans / counts |

`metadata` and `claims` are stored as DynamoDB maps; `created_at`/`updated_at`/`expires_at`
are ISO-8601 strings. Sessions carry a numeric `ttl` (epoch seconds) so DynamoDB expires them
natively — `cleanup_expired_sessions` batch-deletes whatever TTL has not yet reaped, retrying
any `BatchWriteItem` `unprocessed_items` with capped exponential backoff until the batch drains
or a bounded retry budget is exhausted (then error), so a successful return means every expired
session found is gone. `revoke_all_user_sessions` retries the same way, so a successful return
means every targeted session item was deleted. The GSI1 user key includes the provider prefix
so the same external id from two providers does not collide.

`create_user` is a `TransactWriteItems` of two `Put`s — the user item and a uniqueness-guard
item (`EXT#<provider>#<external_id>` / `UNIQUE`, attribute `user_id`) — each conditioned on
`attribute_not_exists(pk)`, making `(provider, external_id)` unique at write time. A
`TransactionCanceledException` whose cancellation reasons include `ConditionalCheckFailed`
means the guard already existed (a racing duplicate `create_user`) and is mapped to
`Error::Conflict`; any other transaction failure (e.g. a missing table, throttling) maps to
`Error::StoreError`. Guard items for users written before this invariant existed are backfilled
by `DynamoRepository::backfill_uniqueness_guards`, a one-off, idempotent migration step (each
write conditioned on `attribute_not_exists(pk)`, so re-running after a partial failure is safe)
that must complete before `get_user_by_external_id` reads through the guard instead of GSI1 — a
guard-less pre-existing user would otherwise become invisible to that lookup.

## PostgreSQL (`adapters/postgres`)

Two tables via `sqlx`: `users` and `sessions`. `metadata` and `claims` are `JSONB`; `users`
carries a `version BIGINT NOT NULL DEFAULT 1` column — a store-managed optimistic-concurrency
counter that `create_user` writes as `1` and every read carries through unchanged. The
migration DDL adds the column with an idempotent `ALTER TABLE … ADD COLUMN IF NOT EXISTS`
step alongside the inline `CREATE TABLE`, so a `users` table that predates the column gets
it and an existing row reads back as `1`. Indexes cover `(external_id, provider)` and
`sessions.user_id`. A unique violation on insert (SQLSTATE `23505`) maps to
`Error::Conflict` rather than `Error::StoreError`, so a racing duplicate `create_user` is
distinguishable from an infrastructure failure. `create_pool(url, max_connections)` builds
the connection pool. Implements both repository traits.

## SQLite (`adapters/sqlite`)

Single-file `sqlx` store with WAL mode and foreign-key enforcement; `metadata`/`claims` stored
as JSON `TEXT`. `users` carries the same store-managed `version INTEGER NOT NULL DEFAULT 1`
column as PostgreSQL; `create_pool(path)` runs the inline DDL and then an idempotent
`ALTER TABLE` step (SQLite's `ADD COLUMN` has no `IF NOT EXISTS` form) that adds the column
to a `users` table that predates it, defaulting existing rows to `1`. The same
`(external_id, provider)` unique index applies, and SQLite's unique-violation extended
result code (`2067`, `SQLITE_CONSTRAINT_UNIQUE`) maps to `Error::Conflict` the same way as
Postgres's `23505`. Implements both repository traits — the zero-dependency single-host
option.

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
