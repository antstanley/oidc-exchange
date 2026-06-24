# Persistence

**Status:** Implemented · **Date:** 2026-06-24 · **Owner:** Ant Stanley · **Scope:** crates/adapters storage, schemas/

How the domain entities ([01-domain-model.md](01-domain-model.md)) are stored by each
repository adapter. The adapter-agnostic logical model is `schemas/datamodel.schema.json`; the
DynamoDB physical layout is `schemas/dynamodb/table-design.json`.

## DynamoDB (`adapters/dynamo`)

Single-table design with one global secondary index (`GSI1`). Keys `pk`/`sk`, GSI keys
`GSI1pk`/`GSI1sk` (projection `ALL`).

| Item | pk | sk | GSI1pk | GSI1sk |
|---|---|---|---|---|
| User | `USER#<id>` | `PROFILE` | `EXT#<provider>#<external_id>` | `USER` |
| Session | `SESSION#<refresh_token_hash>` | `SESSION` | `USER#<user_id>` | `SESSION#<created_at>` |

Access patterns:

| Operation | DynamoDB call |
|---|---|
| `get_user_by_id` | `GetItem` on `USER#<id>` / `PROFILE` |
| `get_user_by_external_id` | `Query` GSI1 `EXT#<provider>#<external_id>` |
| `get_session_by_refresh_token` | `GetItem` on `SESSION#<hash>` |
| `revoke_all_user_sessions` | `Query` GSI1 `USER#<user_id>` then `BatchWrite` deletes |
| `count_by_status` / `count_active_sessions` | table scans / counts |

`metadata` and `claims` are stored as DynamoDB maps; `created_at`/`updated_at`/`expires_at`
are ISO-8601 strings. Sessions carry a numeric `ttl` (epoch seconds) so DynamoDB expires them
natively — `cleanup_expired_sessions` is a no-op cost there. The GSI1 user key includes the
provider prefix so the same external id from two providers does not collide.

## PostgreSQL (`adapters/postgres`)

Two tables via `sqlx`: `users` and `sessions`. `metadata` and `claims` are `JSONB`; indexes
cover `(external_id, provider)` and `sessions.user_id`. `create_pool(url, max_connections)`
builds the connection pool. Implements both repository traits.

## SQLite (`adapters/sqlite`)

Single-file `sqlx` store with WAL mode and foreign-key enforcement; `metadata`/`claims` stored
as JSON `TEXT`. `create_pool(path)` opens the database. Implements both repository traits — the
zero-dependency single-host option.

## Session-only stores

- **LMDB (`adapters/lmdb`)** — embedded `heed` store with two named databases, `sessions`
  (hash → session) and `user_sessions` (user → set of hashes for revoke-all). Constructed with
  a path and a max map size in MB.
- **Valkey/Redis (`adapters/valkey`)** — `fred` client; keys `{prefix}session:{hash}` and a
  `{prefix}user_sessions:{user_id}` set. Suited to high session churn alongside a durable user
  store.

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
