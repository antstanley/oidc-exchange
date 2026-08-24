use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fred::prelude::*;
use fred::types::ExpireOptions;
use oidc_exchange_core::domain::{
    is_valid_family_id, RefreshResolution, RetiredRefreshToken, Session,
};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::secret::Secret;
use oidc_exchange_core::ports::SessionRepository;
use tracing::instrument;

/// The minimum TTL, in seconds, a session may be stored with. `store_refresh_token` computes
/// `ttl_seconds` from `expires_at - now`; a value below this floor means `expires_at` is not
/// strictly in the future, so the write is rejected before any key is created (no immortal or
/// already-expired Valkey key is ever written).
const SESSION_TTL_SECONDS_MIN: i64 = 1;

/// The same floor for single-use records: a `put_single_use` whose `expires_at` is not
/// strictly in the future would need `SET … EX 0` (rejected server-side) or worse, so it
/// is refused before any key is created rather than writing a born-dead record.
const SINGLE_USE_TTL_SECONDS_MIN: i64 = 1;

/// The `COUNT` hint, in keys per page, passed to every `SCAN` issued by `cleanup_expired_sessions`.
/// This only advises the server on page size; it does not bound the total number of keys
/// visited (the client keeps paging until the cursor returns to 0).
const SCAN_BATCH_COUNT: u32 = 256;

/// The conditional rotation swap, run as one `EVAL`'d script (a pipeline
/// batches without atomicity or a condition; this operation's writes are
/// conditional, so it gets a script).
///
/// KEYS[1] live session hash · KEYS[2] retirement record for the presented
/// hash · KEYS[3] replacement session hash · KEYS[4] family set · KEYS[5]
/// user set. ARGV[1] replacement hash · ARGV[2] presented (live) hash ·
/// ARGV[3] owner user id · ARGV[4] replacement TTL seconds · ARGV[5]
/// retirement-record TTL seconds · ARGV[6] retired_at timestamp · ARGV[7]
/// retirement deadline timestamp · ARGV[8] family-set TTL seconds ·
/// ARGV[9..] the replacement's field name/value pairs.
///
/// Returns `0` when the CAS condition fails (the live generation moved —
/// nothing has been written), `-1` when the proposed replacement already
/// exists (a caller bug, surfaced to the caller as a store error), and `1`
/// after installing every effect of the swap: delete the live row, write the
/// retirement record naming its successor, install the replacement, and move
/// both set memberships. The `active_sessions` counter is deliberately
/// untouched: the swap removes one live generation and installs exactly one,
/// so its net effect on the counter is zero by construction, and a counter
/// comparison inside this script could only reintroduce the panic the clamp
/// elsewhere in this module exists to remove. A live row whose `family_id`
/// field is absent or empty is a pre-rotation legacy row: the swap installs
/// the replacement under whatever family the caller minted and writes *no*
/// retirement record — there is no prior generation to detect reuse against.
///
/// Every expiry is bumped with greatest-of semantics (`bump_ttl`), mirroring
/// the store path: no concurrent shorter-lived write may shorten a set's
/// life, and a brand-new set gets its first TTL here.
const ROTATION_SCRIPT: &str = r#"
local function bump_ttl(key, secs)
  local current = redis.call('TTL', key)
  if current < 0 or secs > current then
    redis.call('EXPIRE', key, secs)
  end
end

if redis.call('EXISTS', KEYS[1]) == 0 then
  return 0
end
local user_id = redis.call('HGET', KEYS[1], 'user_id')
if user_id ~= ARGV[3] then
  return redis.error_reply('rotate_refresh_token: live row belongs to a different user')
end
if redis.call('EXISTS', KEYS[3]) == 1 then
  return -1
end

local live_family = redis.call('HGET', KEYS[1], 'family_id')
local is_legacy = (not live_family) or live_family == ''

redis.call('DEL', KEYS[1])
redis.call('SREM', KEYS[5], ARGV[2])

for i = 9, #ARGV, 2 do
  redis.call('HSET', KEYS[3], ARGV[i], ARGV[i + 1])
end
redis.call('EXPIRE', KEYS[3], tonumber(ARGV[4]))
redis.call('SADD', KEYS[5], ARGV[1])
bump_ttl(KEYS[5], tonumber(ARGV[4]))

if not is_legacy then
  redis.call('HSET', KEYS[2],
    'user_id', user_id,
    'family_id', live_family,
    'successor_hash', ARGV[1],
    'retired_at', ARGV[6],
    'expires_at', ARGV[7])
  redis.call('EXPIRE', KEYS[2], tonumber(ARGV[5]))
  redis.call('SADD', KEYS[4], ARGV[2])
end
redis.call('SADD', KEYS[4], ARGV[1])
bump_ttl(KEYS[4], tonumber(ARGV[8]))

return 1
"#;

pub struct ValkeySessionRepository {
    client: fred::clients::Client,
    key_prefix: String,
    /// How long a retirement record stays readable after its rotation:
    /// `retired_at + reuse_retention_secs`, capped per record at the family's
    /// absolute `expires_at` by [`RetiredRefreshToken::retention_deadline`].
    /// Resolved from `[token] refresh_reuse_retention` at bootstrap; injected
    /// here because the store, not the caller, stamps every record's deadline.
    reuse_retention_secs: u64,
}

impl ValkeySessionRepository {
    pub async fn new(
        url: &str,
        key_prefix: String,
        reuse_retention_secs: u64,
    ) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        assert!(
            reuse_retention_secs > 0,
            "reuse retention must be greater than zero"
        );
        let config = Config::from_url(url)?;
        let client = fred::clients::Client::new(config, None, None, None);
        client.init().await?;
        Ok(Self {
            client,
            key_prefix,
            reuse_retention_secs,
        })
    }

    fn session_key(&self, token_hash: &str) -> String {
        format!("{}session:{}", self.key_prefix, token_hash)
    }

    fn retired_key(&self, token_hash: &str) -> String {
        format!("{}retired:{}", self.key_prefix, token_hash)
    }

    fn family_key(&self, family_id: &str) -> String {
        format!("{}family:{}", self.key_prefix, family_id)
    }

    fn user_sessions_key(&self, user_id: &str) -> String {
        format!("{}user_sessions:{}", self.key_prefix, user_id)
    }

    fn active_sessions_key(&self) -> String {
        format!("{}active_sessions", self.key_prefix)
    }

    /// Decrement the active-sessions counter by `amount`, clamping a negative
    /// observed value back to zero with one structured warning instead of
    /// asserting. The counter is reconciled state, not an invariant the
    /// adapter establishes (natural TTL expiry drives it above the live
    /// count; external administration can drive it below zero), and the
    /// shipped `assert!(counter >= 0)` panicked on exactly that drift —
    /// reachable from unauthenticated `POST /revoke`. No decrement path may
    /// unwind.
    async fn decr_counter_clamped(&self, amount: i64) -> Result<()> {
        debug_assert!(amount > 0, "decr_counter_clamped: amount must be positive");
        let active_sessions_key = self.active_sessions_key();
        let counter: i64 = self
            .client
            .decr_by(&active_sessions_key, amount)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        if counter < 0 {
            self.client
                .set::<(), _, _>(&active_sessions_key, 0_i64, None, None, false)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
            tracing::warn!(
                counter_clamped = true,
                observed = counter,
                amount,
                "active_sessions counter drifted below zero on a revoke path; \
                 clamped to 0 (the counter is reconciled state, repaired exactly \
                 by cleanup_expired_sessions)"
            );
        }
        Ok(())
    }

    /// Fetch one retirement record by hash, if it is still retained. Inherent
    /// helper (not a port method): `resolve_refresh_token` needs the raw
    /// record to evaluate the successor pointer, while `/revoke`'s liveness
    /// lookup deliberately does not see retirement records as sessions.
    async fn get_retired_record(&self, token_hash: &str) -> Result<Option<RetiredRefreshToken>> {
        let values: std::collections::HashMap<String, String> = self
            .client
            .hgetall(self.retired_key(token_hash))
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        if values.is_empty() {
            return Ok(None);
        }

        let get_field = |name: &str| -> Result<String> {
            values.get(name).cloned().ok_or_else(|| Error::StoreError {
                detail: format!("retired record is missing field: {name}"),
            })
        };
        let parse_dt = |name: &str| -> Result<DateTime<Utc>> {
            DateTime::parse_from_rfc3339(&get_field(name)?)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| Error::StoreError {
                    detail: format!("invalid {name} in retired record: {e}"),
                })
        };

        // Round-trip revalidation (store-read boundary): the script writes
        // every field, so a missing or mistyped field is corruption and must
        // surface as a store error rather than a half-record.
        Ok(Some(RetiredRefreshToken {
            refresh_token_hash: token_hash.to_string(),
            family_id: get_field("family_id")?,
            user_id: get_field("user_id")?,
            successor_hash: get_field("successor_hash")?,
            retired_at: parse_dt("retired_at")?,
            expires_at: parse_dt("expires_at")?,
        }))
    }

    fn single_use_key(&self, key: &str) -> String {
        format!("{}single_use:{}", self.key_prefix, key)
    }
}

#[async_trait]
impl SessionRepository for ValkeySessionRepository {
    // The whole `Session` carries the lookup-key digest and client provenance
    // (`ip_address`, `user_agent`, `device_id`), so it must be skipped wholesale rather
    // than left to `#[instrument]`'s default argument recording; only the permitted
    // `user_id` is captured, by value-shaping reference before any skip applies.
    #[instrument(skip(self, session), fields(user_id = %session.user_id))]
    async fn store_refresh_token(&self, session: &Session) -> Result<()> {
        // Compute TTL from expires_at first and reject before issuing any write: a session
        // whose expires_at is not strictly in the future would otherwise leave a TTL-less (or
        // already-dead) hash behind.
        let ttl_seconds = (session.expires_at - Utc::now()).num_seconds();
        if ttl_seconds < SESSION_TTL_SECONDS_MIN {
            return Err(Error::StoreError {
                detail: format!(
                    "refusing to store session with non-future expires_at (ttl_seconds={ttl_seconds})"
                ),
            });
        }
        debug_assert!(ttl_seconds >= SESSION_TTL_SECONDS_MIN);

        let key = self.session_key(session.refresh_token_hash.expose());
        let user_sessions_key = self.user_sessions_key(&session.user_id);
        let active_sessions_key = self.active_sessions_key();

        assert!(!key.is_empty(), "session key must not be empty");
        assert!(
            !user_sessions_key.is_empty(),
            "user_sessions key must not be empty"
        );
        assert!(
            session.family_id.is_empty() || is_valid_family_id(&session.family_id),
            "store_refresh_token: malformed family id {:?}",
            session.family_id
        );

        let fields: Vec<(&str, String)> = vec![
            ("user_id", session.user_id.clone()),
            (
                "refresh_token_hash",
                session.refresh_token_hash.expose().clone(),
            ),
            ("family_id", session.family_id.clone()),
            ("generation", session.generation.to_string()),
            ("provider", session.provider.clone()),
            (
                "rotated_at",
                session
                    .rotated_at
                    .map(|ts| ts.to_rfc3339())
                    .unwrap_or_default(),
            ),
            ("expires_at", session.expires_at.to_rfc3339()),
            ("device_id", session.device_id.clone().unwrap_or_default()),
            ("user_agent", session.user_agent.clone().unwrap_or_default()),
            ("ip_address", session.ip_address.clone().unwrap_or_default()),
            ("created_at", session.created_at.to_rfc3339()),
        ];

        // Batch the hash write, both TTLs, the user-set membership, and the counter INCR into
        // a single pipeline so a crash between commands can never leave a TTL-less hash, a
        // half-written index, or a missed count.
        let pipeline = self.client.pipeline();
        let _: () = pipeline
            .hset(&key, fields)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let _: () = pipeline
            .expire(&key, ttl_seconds, None)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let _: () = pipeline
            .sadd(&user_sessions_key, session.refresh_token_hash.expose())
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        // Bootstrap the set's TTL on its first-ever member (NX: only when the set currently
        // has no expiry — SADD above may have just created it with none), then only-extend on
        // every write (GT: Valkey treats "no expiry" as infinite for GT, so GT alone would
        // never TTL a brand-new set). Together these give "TTL bumped to the greatest member
        // expiry" without a concurrent shorter-lived write ever shortening the set's life.
        let _: () = pipeline
            .expire(&user_sessions_key, ttl_seconds, Some(ExpireOptions::NX))
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let _: () = pipeline
            .expire(&user_sessions_key, ttl_seconds, Some(ExpireOptions::GT))
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let _: () = pipeline
            .incr(&active_sessions_key)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        // Family-set membership indexes `revoke_family`: one SADD per store,
        // TTL'd by the same family bound. A sentinel-family (legacy) row
        // belongs to no family and gets no membership — `revoke_family`
        // rejects the empty id, so an entry filed under "" could never be
        // addressed.
        if !session.family_id.is_empty() {
            let family_key = self.family_key(&session.family_id);
            let _: () = pipeline
                .sadd(&family_key, session.refresh_token_hash.expose())
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
            let _: () = pipeline
                .expire(&family_key, ttl_seconds, Some(ExpireOptions::NX))
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
            let _: () = pipeline
                .expire(&family_key, ttl_seconds, Some(ExpireOptions::GT))
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        }

        pipeline.all::<()>().await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

        Ok(())
    }

    // The digest is skipped explicitly (not left to a name collision with the schema
    // field), so a parameter rename cannot silently re-expose the session lookup key.
    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn get_session_by_refresh_token(
        &self,
        token_hash: &Secret<String>,
    ) -> Result<Option<Session>> {
        let key = self.session_key(token_hash.expose());

        let values: std::collections::HashMap<String, String> = self
            .client
            .hgetall(&key)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        if values.is_empty() {
            return Ok(None);
        }

        let get_field = |name: &str| -> Result<String> {
            values.get(name).cloned().ok_or_else(|| Error::StoreError {
                detail: format!("missing field: {}", name),
            })
        };

        let parse_dt = |s: &str| -> Result<DateTime<Utc>> {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })
        };

        let device_id =
            values
                .get("device_id")
                .and_then(|v| if v.is_empty() { None } else { Some(v.clone()) });
        let user_agent =
            values.get("user_agent").and_then(
                |v| {
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.clone())
                    }
                },
            );
        let ip_address =
            values.get("ip_address").and_then(
                |v| {
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.clone())
                    }
                },
            );

        let expires_at_str = get_field("expires_at")?;
        let created_at_str = get_field("created_at")?;

        // A hash written before rotation shipped has no family fields (the
        // hash write is not migratable in place). They read back with the same
        // sentinel values the SQL adapters use for a NULL `family_id` column —
        // an empty string that deliberately fails `is_valid_family_id`, so
        // downstream family operations visibly fail rather than silently
        // matching a family that does not exist.
        let legacy_family_id = values.get("family_id").cloned().unwrap_or_default();
        let legacy_generation = values
            .get("generation")
            .map(|g| g.parse::<u32>())
            .transpose()
            .map_err(|e| Error::StoreError {
                detail: format!("invalid generation field: {e}"),
            })?
            .unwrap_or(0);
        let rotated_at = match values.get("rotated_at") {
            Some(s) if !s.is_empty() => Some(
                DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| Error::StoreError {
                        detail: format!("invalid rotated_at field: {e}"),
                    })?,
            ),
            _ => None,
        };

        Ok(Some(Session {
            user_id: get_field("user_id")?,
            refresh_token_hash: Secret::new(get_field("refresh_token_hash")?),
            family_id: legacy_family_id,
            generation: legacy_generation,
            provider: get_field("provider")?,
            expires_at: parse_dt(&expires_at_str)?,
            rotated_at,
            device_id,
            user_agent,
            ip_address,
            created_at: parse_dt(&created_at_str)?,
        }))
    }

    /// Classify `token_hash` against live generations first, then retained
    /// retirement records (SR1). Valkey key reads are strongly consistent. A
    /// record past its retention deadline answers `Unknown` until its key
    /// expires natively or the sweep prunes it — reuse detection must not
    /// fire on a window that has closed.
    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn resolve_refresh_token(&self, token_hash: &str) -> Result<RefreshResolution> {
        assert!(
            !token_hash.is_empty(),
            "resolve_refresh_token: token_hash must not be empty"
        );
        if let Some(session) = self.get_session_by_refresh_token(&Secret::new(token_hash.to_string())).await? {
            return Ok(RefreshResolution::Live(session));
        }

        let Some(record) = self.get_retired_record(token_hash).await? else {
            return Ok(RefreshResolution::Unknown);
        };
        if record.expires_at <= Utc::now() {
            return Ok(RefreshResolution::Unknown);
        }

        match self
            .get_session_by_refresh_token(&Secret::new(record.successor_hash.clone()))
            .await?
        {
            Some(successor_live) => Ok(RefreshResolution::Superseded {
                live: successor_live,
                retired_at: record.retired_at,
            }),
            None => Ok(RefreshResolution::Retired {
                family_id: record.family_id,
                user_id: record.user_id,
                retired_at: record.retired_at,
            }),
        }
    }

    /// One `EVAL`'d script performing the whole conditional swap — delete the
    /// live row, write the retirement record, install the replacement, move
    /// both set memberships — or nothing (SR2/SR3/SR4). The CAS condition is
    /// the live key's existence inside the atomic script: a concurrent
    /// redemption that moved it first makes the script answer `0` and this
    /// returns `false` with no partial swap. See [`ROTATION_SCRIPT`] for the
    /// legacy-row rule and why the counter is deliberately untouched.
    #[instrument(skip(self, live_hash, replacement), fields(token_hash, user_id = %replacement.user_id))]
    async fn rotate_refresh_token(&self, live_hash: &str, replacement: &Session) -> Result<bool> {
        assert!(
            is_valid_family_id(&replacement.family_id),
            "rotate_refresh_token: malformed replacement family id {:?}",
            replacement.family_id
        );
        assert!(
            live_hash != replacement.refresh_token_hash.expose().as_str(),
            "rotate_refresh_token: replacement must be a fresh generation"
        );

        // Same future-expiry gate as the store path: a replacement whose
        // deadline is not strictly in the future must fail before any key is
        // touched, not halfway through a swap.
        let ttl_seconds = (replacement.expires_at - Utc::now()).num_seconds();
        if ttl_seconds < SESSION_TTL_SECONDS_MIN {
            return Err(Error::StoreError {
                detail: format!(
                    "refusing to rotate to a session with non-future expires_at \
                     (ttl_seconds={ttl_seconds})"
                ),
            });
        }
        debug_assert!(ttl_seconds >= SESSION_TTL_SECONDS_MIN);

        let now = Utc::now();
        let retired_expires_at = RetiredRefreshToken::retention_deadline(
            now,
            self.reuse_retention_secs,
            replacement.expires_at,
        );
        let retired_ttl_seconds = (retired_expires_at - now)
            .num_seconds()
            .max(SESSION_TTL_SECONDS_MIN);

        let mut args: Vec<String> = vec![
            /* ARGV[1] */ replacement.refresh_token_hash.expose().clone(),
            /* ARGV[2] */ live_hash.to_string(),
            /* ARGV[3] */ replacement.user_id.clone(),
            /* ARGV[4] */ ttl_seconds.to_string(),
            /* ARGV[5] */ retired_ttl_seconds.to_string(),
            /* ARGV[6] */ now.to_rfc3339(),
            /* ARGV[7] */ retired_expires_at.to_rfc3339(),
            /* ARGV[8] */ ttl_seconds.to_string(),
        ];
        // ARGV[8..]: the replacement's field pairs, HSET'ed verbatim so the
        // script's install writes exactly what `store_refresh_token` would.
        args.push("user_id".to_string());
        args.push(replacement.user_id.clone());
        args.push("refresh_token_hash".to_string());
        args.push(replacement.refresh_token_hash.expose().clone());
        args.push("family_id".to_string());
        args.push(replacement.family_id.clone());
        args.push("generation".to_string());
        args.push(replacement.generation.to_string());
        args.push("provider".to_string());
        args.push(replacement.provider.clone());
        args.push("rotated_at".to_string());
        args.push(
            replacement
                .rotated_at
                .map(|ts| ts.to_rfc3339())
                .unwrap_or_default(),
        );
        args.push("expires_at".to_string());
        args.push(replacement.expires_at.to_rfc3339());
        args.push("device_id".to_string());
        args.push(replacement.device_id.clone().unwrap_or_default());
        args.push("user_agent".to_string());
        args.push(replacement.user_agent.clone().unwrap_or_default());
        args.push("ip_address".to_string());
        args.push(replacement.ip_address.clone().unwrap_or_default());
        args.push("created_at".to_string());
        args.push(replacement.created_at.to_rfc3339());

        let keys: Vec<String> = vec![
            self.session_key(live_hash),
            self.retired_key(live_hash),
            self.session_key(replacement.refresh_token_hash.expose()),
            self.family_key(&replacement.family_id),
            self.user_sessions_key(&replacement.user_id),
        ];

        let outcome: i64 = self
            .client
            .eval(ROTATION_SCRIPT, keys, args)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        match outcome {
            0 => Ok(false),
            1 => Ok(true),
            -1 => Err(Error::StoreError {
                detail: "rotate_refresh_token: proposed replacement already exists in the store"
                    .to_string(),
            }),
            other => unreachable!("ROTATION_SCRIPT returns only -1, 0, 1; got {other}"),
        }
    }

    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn revoke_session(&self, token_hash: &Secret<String>) -> Result<()> {
        let token_hash = token_hash.expose().as_str();
        let key = self.session_key(token_hash);

        // Get user_id before deleting so we can clean up the user set
        let user_id: Option<String> =
            self.client
                .hget(&key, "user_id")
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

        // Capture DEL's return count: it is 0 when the key was already gone (already
        // revoked or naturally expired) and 1 when this call actually removed it. Only
        // decrement the counter on an actual delete, so a repeated or already-expired
        // revoke does not double-decrement.
        let deleted_count: u64 = self.client.del(&key).await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;
        assert!(
            deleted_count <= 1,
            "DEL of a single key must return 0 or 1, got {deleted_count}"
        );

        if deleted_count == 1 {
            // Reconciled state, not an invariant: a decrement landing below
            // zero is drift that cleanup repairs — clamped to zero with one
            // structured warning, never an assert (see `decr_counter_clamped`).
            self.decr_counter_clamped(1).await?;
        }

        if let Some(user_id) = user_id {
            self.client
                .srem::<(), _, _>(self.user_sessions_key(&user_id), token_hash)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        }

        Ok(())
    }

    /// Remove the family's live generation and every retained retirement
    /// record, returning the combined count (SR5), enumerating through the
    /// `{prefix}family:{family_id}` set. Each member is classified by which
    /// of its two keys still exists — a live session (deleted with full
    /// `revoke_session` bookkeeping) or a retirement record (deleted without
    /// touching the counter, which never counted it). Idempotent: an unknown
    /// (but well-formed) family id removes nothing and returns `Ok(0)`.
    #[instrument(skip(self), fields(family_id))]
    async fn revoke_family(&self, family_id: &str) -> Result<u64> {
        assert!(
            is_valid_family_id(family_id),
            "revoke_family: malformed family id {family_id:?}"
        );

        let family_key = self.family_key(family_id);
        let members: Vec<String> =
            self.client
                .smembers(&family_key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

        let mut removed: u64 = 0;
        let mut live_deleted: i64 = 0;
        for hash in &members {
            // Capture the owner before deleting so the user set stays
            // consistent with the removal.
            let user_id: Option<String> = self
                .client
                .hget(self.session_key(hash), "user_id")
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

            let deleted_live: u64 =
                self.client
                    .del(self.session_key(hash))
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            assert!(
                deleted_live <= 1,
                "DEL of a single key must return 0 or 1, got {deleted_live}"
            );
            if deleted_live == 1 {
                removed += 1;
                live_deleted += 1;
                if let Some(user_id) = user_id {
                    self.client
                        .srem::<(), _, _>(self.user_sessions_key(&user_id), hash)
                        .await
                        .map_err(|e| Error::StoreError {
                            detail: e.to_string(),
                        })?;
                }
                self.client
                    .srem::<(), _, _>(&family_key, hash.as_str())
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
                continue;
            }

            // Not live: either a retained retirement record or a stale
            // membership whose keys expired natively.
            let deleted_retired: u64 =
                self.client
                    .del(self.retired_key(hash))
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            assert!(
                deleted_retired <= 1,
                "DEL of a single key must return 0 or 1, got {deleted_retired}"
            );
            if deleted_retired == 1 {
                removed += 1;
            }
            self.client
                .srem::<(), _, _>(&family_key, hash.as_str())
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        }

        // One clamped decrement for the whole family, not per delete.
        if live_deleted > 0 {
            self.decr_counter_clamped(live_deleted).await?;
        }

        // The sweep empties the family; drop the set when nothing remains so
        // an idle family's index does not linger forever as an empty key.
        let remaining: u64 =
            self.client
                .scard(&family_key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        if remaining == 0 {
            self.client
                .del::<(), _>(&family_key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        }

        Ok(removed)
    }

    #[instrument(skip(self))]
    async fn count_active_sessions(&self) -> Result<u64> {
        // Read the maintained counter rather than DBSIZE (which would count every key in
        // the database, including the user_sessions index sets and any keys outside
        // key_prefix). A missing key (nothing ever stored under this prefix) reads as 0
        // rather than erroring.
        let active_sessions_key = self.active_sessions_key();
        assert!(
            !active_sessions_key.is_empty(),
            "active_sessions key must not be empty"
        );

        let count: Option<u64> =
            self.client
                .get(&active_sessions_key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

        let result = count.unwrap_or(0);
        assert!(
            count.is_some() || result == 0,
            "a missing active_sessions key must report 0, not an arbitrary count"
        );
        Ok(result)
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        use futures::stream::TryStreamExt;

        // Pass 1: prune dead members out of every `{prefix}user_sessions:*` index set. A
        // member is dead when its `{prefix}session:{hash}` key has already expired (or was
        // otherwise removed) server-side, so natural TTL expiry never shrinks the set on its
        // own — this pass is what reaps them.
        let user_sessions_pattern = format!("{}user_sessions:*", self.key_prefix);
        let set_keys: Vec<Key> = self
            .client
            .scan_buffered(user_sessions_pattern.as_str(), Some(SCAN_BATCH_COUNT), None)
            .try_collect()
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        let mut removed_members: u64 = 0;
        let mut members_scanned: u64 = 0;

        for set_key in set_keys {
            let set_key = set_key.as_str_lossy().into_owned();
            let members: Vec<String> =
                self.client
                    .smembers(&set_key)
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            members_scanned += members.len() as u64;

            let mut dead_members = Vec::new();
            for member in &members {
                let session_key = self.session_key(member);
                let exists: bool =
                    self.client
                        .exists(&session_key)
                        .await
                        .map_err(|e| Error::StoreError {
                            detail: e.to_string(),
                        })?;
                if !exists {
                    dead_members.push(member.clone());
                }
            }

            if !dead_members.is_empty() {
                let dead_count = dead_members.len() as u64;
                let srem_count: u64 =
                    self.client
                        .srem(&set_key, dead_members)
                        .await
                        .map_err(|e| Error::StoreError {
                            detail: e.to_string(),
                        })?;
                assert!(
                    srem_count <= dead_count,
                    "SREM must not remove more members than were identified as dead, \
                     removed={srem_count} dead={dead_count}"
                );
                removed_members += srem_count;
            }

            // Delete the set once it holds no members, so an idle user's index set does not
            // linger forever as an empty key.
            let remaining: u64 =
                self.client
                    .scard(&set_key)
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            if remaining == 0 {
                self.client
                    .del::<(), _>(&set_key)
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            }
        }

        assert!(
            removed_members <= members_scanned,
            "the returned removed-count must not exceed the total members scanned across \
             all user_sessions sets, removed={removed_members} scanned={members_scanned}"
        );

        // Pass 1b: prune dead members out of every `{prefix}family:*` set. A
        // member is dead when neither its session key nor its retirement
        // record exists any more (both expire natively; the set membership
        // never does on its own). The count covers these prunes together with
        // the user-set prunes above — the sweep's work across both index
        // structures.
        let family_pattern = format!("{}family:*", self.key_prefix);
        let family_set_keys: Vec<Key> = self
            .client
            .scan_buffered(family_pattern.as_str(), Some(SCAN_BATCH_COUNT), None)
            .try_collect()
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        for set_key in family_set_keys {
            let set_key = set_key.as_str_lossy().into_owned();
            let members: Vec<String> =
                self.client
                    .smembers(&set_key)
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;

            let mut dead_members = Vec::new();
            for member in &members {
                // Pipeline both existence probes so one member costs one round trip.
                let session_exists: bool = self
                    .client
                    .exists(self.session_key(member))
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
                if session_exists {
                    continue;
                }
                let retired_exists: bool = self
                    .client
                    .exists(self.retired_key(member))
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
                if !retired_exists {
                    dead_members.push(member.clone());
                }
            }

            if !dead_members.is_empty() {
                let dead_count = dead_members.len();
                let srem_count: u64 =
                    self.client
                        .srem(&set_key, dead_members)
                        .await
                        .map_err(|e| Error::StoreError {
                            detail: e.to_string(),
                        })?;
                assert!(
                    srem_count <= dead_count as u64,
                    "SREM must not remove more members than were identified as dead, \
                     removed={srem_count} dead={dead_count}"
                );
                removed_members += srem_count;
            }

            // Delete the set once it holds no members, so a fully-revoked or
            // naturally-expired family's set does not linger forever.
            let remaining: u64 =
                self.client
                    .scard(&set_key)
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            if remaining == 0 {
                self.client
                    .del::<(), _>(&set_key)
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            }
        }

        // Pass 2: reconcile the counter. INCR (on store) and DECR (on explicit revoke) never
        // account for natural TTL expiry, so the counter can only drift upward between
        // cleanups; recompute it here from a live SCAN of `{prefix}session:*` and SET it to
        // that exact count.
        let session_pattern = format!("{}session:*", self.key_prefix);
        let live_keys: Vec<Key> = self
            .client
            .scan_buffered(session_pattern.as_str(), Some(SCAN_BATCH_COUNT), None)
            .try_collect()
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let live_count = live_keys.len() as u64;

        let active_sessions_key = self.active_sessions_key();
        self.client
            .set::<(), _, _>(&active_sessions_key, live_count, None, None, false)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        let reconciled: Option<u64> =
            self.client
                .get(&active_sessions_key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        assert_eq!(
            reconciled.unwrap_or(0),
            live_count,
            "the reconciled active_sessions counter must equal the counted live session keys"
        );

        Ok(removed_members)
    }

    #[instrument(skip(self))]
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()> {
        let user_set_key = self.user_sessions_key(user_id);

        let token_hashes: Vec<String> =
            self.client
                .smembers(&user_set_key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

        // Delete every session key the user set names, then DECR the counter by exactly
        // the number of keys that actually existed (a stale/already-expired member in the
        // set must not decrement the counter). A single multi-key DEL both avoids an
        // unbounded round-trip loop and returns the live-key count directly.
        let deleted_count: u64 = if token_hashes.is_empty() {
            0
        } else {
            let keys: Vec<String> = token_hashes
                .iter()
                .map(|token_hash| self.session_key(token_hash))
                .collect();
            self.client.del(keys).await.map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?
        };
        assert!(
            deleted_count <= token_hashes.len() as u64,
            "deleted session-key count must not exceed the number of member hashes read, \
             got deleted={deleted_count} members={}",
            token_hashes.len()
        );

        if deleted_count > 0 {
            let deleted_count_i64 =
                i64::try_from(deleted_count).map_err(|_| Error::StoreError {
                    detail: format!(
                        "deleted session-key count {deleted_count} overflows i64 for DECRBY"
                    ),
                })?;
            // Reconciled state, not an invariant: clamped, never asserted
            // (see `decr_counter_clamped`).
            self.decr_counter_clamped(deleted_count_i64).await?;
        }

        self.client
            .del::<(), _>(&user_set_key)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        // The SR5 removal guarantee applied across all of the user's families:
        // their retained retirement records leave too. Retirement records are
        // keyed by hash with the owner stored inside, so the sweep scans this
        // prefix's `{prefix}retired:*` keys (bounded by the retention window's
        // steady-state size) and removes every record naming this user, plus
        // its family-set membership.
        let retired_pattern = format!("{}retired:*", self.key_prefix);
        let retired_keys: Vec<Key> = {
            use futures::stream::TryStreamExt;
            self.client
                .scan_buffered(retired_pattern.as_str(), Some(SCAN_BATCH_COUNT), None)
                .try_collect()
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?
        };
        for key in retired_keys {
            let key = key.as_str_lossy().into_owned();
            let fields: std::collections::HashMap<String, String> = self
                .client
                .hgetall(&key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
            if fields.get("user_id").map(String::as_str) != Some(user_id) {
                continue;
            }
            let record_hash = key
                .strip_prefix(&format!("{}retired:", self.key_prefix))
                .unwrap_or(&key)
                .to_string();
            let deleted: u64 = self.client.del(&key).await.map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
            assert!(
                deleted <= 1,
                "DEL of a single key must return 0 or 1, got {deleted}"
            );
            if deleted == 1 {
                if let Some(family_id) = fields.get("family_id") {
                    self.client
                        .srem::<(), _, _>(self.family_key(family_id), &record_hash)
                        .await
                        .map_err(|e| Error::StoreError {
                            detail: e.to_string(),
                        })?;
                }
            }
        }

        Ok(())
    }

    #[instrument(skip(self, key))]
    async fn put_single_use(&self, key: &str, expires_at: DateTime<Utc>) -> Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        let ttl_seconds = (expires_at - Utc::now()).num_seconds();
        if ttl_seconds < SINGLE_USE_TTL_SECONDS_MIN {
            return Err(Error::StoreError {
                detail: format!(
                    "refusing to store single-use record with non-future expires_at \
                     (ttl_seconds={ttl_seconds})"
                ),
            });
        }
        debug_assert!(ttl_seconds >= SINGLE_USE_TTL_SECONDS_MIN);

        // SET … NX EX is one atomic server-side operation: it writes only when the key
        // is absent and arms the native TTL in the same breath, so a lost race leaves
        // no partial state and an unswept record can never outlive its expiry.
        let value = expires_at.to_rfc3339();
        let outcome: Option<String> = self
            .client
            .set(
                self.single_use_key(key),
                value,
                Some(Expiration::EX(ttl_seconds)),
                Some(SetOptions::NX),
                false,
            )
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        Ok(outcome.is_some())
    }

    #[instrument(skip(self, key))]
    async fn take_single_use(&self, key: &str) -> Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        // GETDEL reads-and-removes atomically. The key carries the record's TTL, so any
        // value GETDEL returns is definitionally live — an expired record has already
        // been removed by the server, making expired indistinguishable from absent.
        let removed: Option<String> =
            self.client
                .getdel(self.single_use_key(key))
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

        debug_assert!(
            removed
                .as_deref()
                .map(|v| DateTime::parse_from_rfc3339(v).is_ok())
                .unwrap_or(true),
            "a stored single-use value must be the RFC3339 expiry it was written with"
        );

        Ok(removed.is_some())
    }
}

// ---------------------------------------------------------------------------
// Integration tests (require a live Valkey/Redis)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::LookupSpan;

    /// Reuse-retention window used by every test repository: one hour — short
    /// enough that deadline arithmetic stays inside a test's lifetime, and
    /// positive per the constructor's precondition.
    const TEST_REUSE_RETENTION_SECS: u64 = 3600;

    /// Env var carrying the Valkey/Redis URL used by the `#[ignore]`d tests below; defaults to
    /// a local server, matching how the DynamoDB Local tests hardcode their endpoint.
    fn valkey_test_url() -> String {
        std::env::var("VALKEY_TEST_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string())
    }

    /// Builds a repository against a unique `key_prefix` per call so concurrent/successive test
    /// runs never collide and self-clean (a fresh prefix has no pre-existing keys).
    async fn create_test_repo() -> ValkeySessionRepository {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let prefix = format!(
            "oidc_exchange_test:{}:{}:",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before epoch")
                .as_nanos(),
        );
        let prefix = format!("{prefix}{n}:");

        ValkeySessionRepository::new(&valkey_test_url(), prefix, TEST_REUSE_RETENTION_SECS)
            .await
            .expect("connect to local Valkey (VALKEY_TEST_URL or redis://localhost:6379)")
    }

    fn sample_session(user_id: &str, hash: &str, ttl_seconds: i64) -> Session {
        let now = Utc::now();
        Session {
            user_id: user_id.to_string(),
            refresh_token_hash: Secret::new(hash.to_string()),
            family_id: "fam_0000000000000000000000000a".to_string(),
            generation: 0,
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::seconds(ttl_seconds),
            rotated_at: None,
            device_id: Some("device-1".to_string()),
            user_agent: Some("test-agent".to_string()),
            ip_address: Some("10.0.0.1".to_string()),
            created_at: now,
        }
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn store_refresh_token_writes_ttld_hash_set_member_and_counter() {
        let repo = create_test_repo().await;
        let session = sample_session("usr_1", "hash_abc", 3600);

        repo.store_refresh_token(&session)
            .await
            .expect("store_refresh_token");

        let key = repo.session_key(session.refresh_token_hash.expose());
        let user_set_key = repo.user_sessions_key(&session.user_id);
        let counter_key = repo.active_sessions_key();

        let hash_ttl: i64 = repo.client.ttl(&key).await.expect("ttl on session hash");
        assert!(
            hash_ttl > 0 && hash_ttl <= 3600,
            "session hash TTL should be positive and bounded by the stored TTL, got {hash_ttl}"
        );

        let members: Vec<String> = repo
            .client
            .smembers(&user_set_key)
            .await
            .expect("smembers on user set");
        assert!(
            members.contains(session.refresh_token_hash.expose()),
            "user set should contain the stored session's hash"
        );

        let set_ttl: i64 = repo
            .client
            .ttl(&user_set_key)
            .await
            .expect("ttl on user set");
        assert!(
            set_ttl > 0 && set_ttl <= 3600,
            "user set TTL should be bumped to a positive value, got {set_ttl}"
        );

        let counter: u64 = repo.client.get(&counter_key).await.expect("get counter");
        assert_eq!(counter, 1, "counter should read 1 after one store");
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn store_refresh_token_set_ttl_only_extends() {
        let repo = create_test_repo().await;
        let user_id = "usr_2";

        let long_session = sample_session(user_id, "hash_long", 3600);
        repo.store_refresh_token(&long_session)
            .await
            .expect("store long-lived session");

        let user_set_key = repo.user_sessions_key(user_id);
        let ttl_after_long: i64 = repo
            .client
            .ttl(&user_set_key)
            .await
            .expect("ttl after long-lived write");

        // A second store for the same user with a much shorter TTL must not shorten the
        // already-longer set TTL (EXPIRE ... GT is only-extend).
        let short_session = sample_session(user_id, "hash_short", 5);
        repo.store_refresh_token(&short_session)
            .await
            .expect("store short-lived session");

        let ttl_after_short: i64 = repo
            .client
            .ttl(&user_set_key)
            .await
            .expect("ttl after short-lived write");

        assert!(
            ttl_after_short > 5,
            "GT bump must not shorten the set TTL down to the shorter write's TTL, got {ttl_after_short}"
        );
        assert!(
            ttl_after_short <= ttl_after_long,
            "set TTL should never exceed the longest TTL ever applied (allowing for clock drift), \
             got before={ttl_after_long} after={ttl_after_short}"
        );

        let counter: u64 = repo
            .client
            .get(&repo.active_sessions_key())
            .await
            .expect("get counter");
        assert_eq!(counter, 2, "counter should read 2 after two stores");
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn store_refresh_token_rejects_non_future_expiry() {
        let repo = create_test_repo().await;

        // expires_at at "now" (ttl_seconds == 0, below the floor of 1).
        let at_now = sample_session("usr_3", "hash_at_now", 0);
        let err = repo
            .store_refresh_token(&at_now)
            .await
            .expect_err("expires_at at now should be rejected");
        assert!(matches!(err, Error::StoreError { .. }));

        // expires_at in the past.
        let in_past = sample_session("usr_3", "hash_in_past", -60);
        let err = repo
            .store_refresh_token(&in_past)
            .await
            .expect_err("expires_at in the past should be rejected");
        assert!(matches!(err, Error::StoreError { .. }));

        let key = repo.session_key(at_now.refresh_token_hash.expose());
        let exists: bool = repo
            .client
            .exists(&key)
            .await
            .expect("exists on session hash");
        assert!(!exists, "rejected session must not create a hash key");

        let counter_key = repo.active_sessions_key();
        let counter_exists: bool = repo
            .client
            .exists(&counter_key)
            .await
            .expect("exists on counter key");
        assert!(
            !counter_exists,
            "rejected session must not increment (or create) the counter"
        );
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn count_active_sessions_on_untouched_prefix_returns_zero() {
        let repo = create_test_repo().await;

        let count = repo
            .count_active_sessions()
            .await
            .expect("count_active_sessions on a prefix with no counter key");

        assert_eq!(
            count, 0,
            "a prefix nothing has ever been stored under should report 0, not error"
        );
        let counter_exists: bool = repo
            .client
            .exists(&repo.active_sessions_key())
            .await
            .expect("exists on counter key");
        assert!(
            !counter_exists,
            "an untouched prefix must not have created a counter key as a side effect"
        );
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn revoke_session_decrements_counter_exactly_once() {
        let repo = create_test_repo().await;
        let session = sample_session("usr_revoke", "hash_revoke", 3600);

        repo.store_refresh_token(&session)
            .await
            .expect("store_refresh_token");
        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count after store"),
            1,
            "counter should read 1 after one store"
        );

        repo.revoke_session(&session.refresh_token_hash)
            .await
            .expect("first revoke_session");
        let count_after_first_revoke = repo
            .count_active_sessions()
            .await
            .expect("count after first revoke");
        assert_eq!(
            count_after_first_revoke, 0,
            "counter should drop from 1 to 0 after the session is revoked"
        );

        let key = repo.session_key(session.refresh_token_hash.expose());
        let exists: bool = repo
            .client
            .exists(&key)
            .await
            .expect("exists on session hash after revoke");
        assert!(!exists, "revoked session hash key must be gone");

        // A second revoke of the same (already-gone) token must not double-decrement: DEL
        // returns 0 because the key no longer exists, so the counter must stay at 0.
        repo.revoke_session(&session.refresh_token_hash)
            .await
            .expect("second revoke_session on an already-revoked token");
        let count_after_second_revoke = repo
            .count_active_sessions()
            .await
            .expect("count after second revoke");
        assert_eq!(
            count_after_second_revoke, 0,
            "a repeated revoke of an already-deleted session must not decrement the counter again"
        );
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn revoke_all_user_sessions_decrements_by_live_key_count() {
        let repo = create_test_repo().await;
        let user_id = "usr_revoke_all";

        let session_a = sample_session(user_id, "hash_all_a", 3600);
        let session_b = sample_session(user_id, "hash_all_b", 3600);
        let session_c = sample_session(user_id, "hash_all_c", 3600);
        repo.store_refresh_token(&session_a)
            .await
            .expect("store session a");
        repo.store_refresh_token(&session_b)
            .await
            .expect("store session b");
        repo.store_refresh_token(&session_c)
            .await
            .expect("store session c");
        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count after 3 stores"),
            3,
            "counter should read 3 after three stores for the same user"
        );

        // Revoke one session individually first. revoke_session also SREMs the user set, so
        // after this the set holds exactly the two remaining live members (hash_all_b,
        // hash_all_c) — not all three with one stale.
        repo.revoke_session(&session_a.refresh_token_hash)
            .await
            .expect("revoke session a individually");
        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count after single revoke"),
            2,
            "counter should read 2 after individually revoking one of three sessions"
        );

        // Delete session_b's hash directly through the client, bypassing revoke_session, so
        // the counter is NOT decremented for it. This leaves the user set naming a member
        // (hash_all_b) whose session key no longer exists — a stale member — while the
        // counter still reads 2 even though only one live key (session_c) remains. This
        // reproduces the exact drift revoke_all_user_sessions must handle: it must DECRBY
        // the counter by the number of keys its own DEL actually removed (1, for
        // session_c), not by the number of members read from the set (2).
        let deleted_directly: u64 = repo
            .client
            .del(repo.session_key("hash_all_b"))
            .await
            .expect("delete session_b's hash directly, bypassing revoke_session");
        assert_eq!(
            deleted_directly, 1,
            "the direct DEL of session_b's hash must have removed exactly one key"
        );
        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count before revoke_all_user_sessions"),
            2,
            "the counter must still read 2: the direct DEL above bypassed revoke_session, so \
             it never decremented the counter, leaving it stale relative to the one truly \
             live key (session_c)"
        );

        repo.revoke_all_user_sessions(user_id)
            .await
            .expect("revoke_all_user_sessions");

        let count_after_revoke_all = repo
            .count_active_sessions()
            .await
            .expect("count after revoke_all_user_sessions");
        assert_eq!(
            count_after_revoke_all, 1,
            "counter should drop from 2 to 1: revoke_all_user_sessions read two members from \
             the set (hash_all_b, hash_all_c) but its DEL only actually removed one live key \
             (session_c, since session_b's hash was already gone), so it must DECRBY 1 — the \
             actual delete count — not DECRBY 2, the set-membership count"
        );

        let user_set_key = repo.user_sessions_key(user_id);
        let set_exists: bool = repo
            .client
            .exists(&user_set_key)
            .await
            .expect("exists on user set after revoke_all_user_sessions");
        assert!(
            !set_exists,
            "the user set itself must be deleted by revoke_all_user_sessions"
        );

        for hash in ["hash_all_a", "hash_all_b", "hash_all_c"] {
            let key = repo.session_key(hash);
            let exists: bool = repo
                .client
                .exists(&key)
                .await
                .expect("exists on session hash after revoke_all_user_sessions");
            assert!(
                !exists,
                "session hash {hash} must be gone after revoke_all_user_sessions"
            );
        }
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn cleanup_expired_sessions_prunes_only_member_deletes_set_and_resets_counter() {
        let repo = create_test_repo().await;
        let user_id = "usr_cleanup_solo";

        // A 2s TTL (rather than the theoretical 1s floor) tolerates the sub-second delay
        // between computing `expires_at` here and `store_refresh_token` truncating it down
        // to whole seconds via `num_seconds()`, so the write is never spuriously rejected.
        let session = sample_session(user_id, "hash_cleanup_solo", 2);
        repo.store_refresh_token(&session)
            .await
            .expect("store short-lived session");

        let user_set_key = repo.user_sessions_key(user_id);
        // Extend the set's own TTL well beyond the session's short TTL so the set (with its
        // now-stale sole member) is still present when cleanup runs, decoupling this test
        // from a race against Valkey's own TTL expiry of the set itself.
        let _: bool = repo
            .client
            .expire(&user_set_key, 10, None)
            .await
            .expect("extend user set TTL for deterministic test setup");

        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count after store"),
            1,
            "counter should read 1 after one store"
        );

        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;

        let key = repo.session_key(session.refresh_token_hash.expose());
        let exists: bool = repo
            .client
            .exists(&key)
            .await
            .expect("exists on session hash after TTL expiry");
        assert!(!exists, "the short-TTL session hash must have expired");

        let set_exists_before: bool = repo
            .client
            .exists(&user_set_key)
            .await
            .expect("exists on user set before cleanup");
        assert!(
            set_exists_before,
            "the extended set TTL must have kept the set (with its stale member) alive"
        );

        // Natural TTL expiry never decrements the counter: it still over-reports the one
        // session that has since expired.
        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count before cleanup"),
            1,
            "the counter must still read 1, drifted above the zero truly live sessions"
        );

        let removed = repo
            .cleanup_expired_sessions()
            .await
            .expect("cleanup_expired_sessions");
        assert_eq!(
            removed, 1,
            "cleanup should prune exactly the one stale user_sessions member"
        );

        let set_exists_after: bool = repo
            .client
            .exists(&user_set_key)
            .await
            .expect("exists on user set after cleanup");
        assert!(
            !set_exists_after,
            "the now-empty user_sessions set must be deleted by cleanup"
        );

        let counter_after = repo
            .count_active_sessions()
            .await
            .expect("count after cleanup");
        assert_eq!(
            counter_after, 0,
            "the reconciled counter must equal the zero live session keys, resetting the drift"
        );
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn cleanup_expired_sessions_with_no_dead_members_returns_zero_and_matches_live_count() {
        let repo = create_test_repo().await;
        let user_id = "usr_cleanup_clean";

        let session = sample_session(user_id, "hash_cleanup_clean", 3600);
        repo.store_refresh_token(&session)
            .await
            .expect("store live session");

        let removed = repo
            .cleanup_expired_sessions()
            .await
            .expect("cleanup_expired_sessions over a clean prefix");
        assert_eq!(
            removed, 0,
            "cleanup over a prefix with no dead members must return 0"
        );

        let user_set_key = repo.user_sessions_key(user_id);
        let members: Vec<String> = repo
            .client
            .smembers(&user_set_key)
            .await
            .expect("smembers on user set after cleanup");
        assert!(
            members.contains(session.refresh_token_hash.expose()),
            "the live member must be untouched by cleanup"
        );

        let counter_after = repo
            .count_active_sessions()
            .await
            .expect("count after cleanup");
        assert_eq!(
            counter_after, 1,
            "the counter must equal the live-key count (1) after cleanup with no drift"
        );
    }

    // -----------------------------------------------------------------------
    // Rotation, reuse detection, and counter clamping (task 05)
    // -----------------------------------------------------------------------

    use oidc_exchange_test_utils::session_contract::{self, family_chain, fixture_family_id};

    /// The full SR1–SR5 shared suite against the Valkey store. One tag
    /// namespaces every fixture under one key prefix.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn valkey_session_store_meets_sr1_through_sr5() {
        let repo = create_test_repo().await;
        session_contract::assert_full_conformance(&repo, "valkey-session-conformance").await;

        // Self-clean: drop the prefix this suite created.
        let pattern = format!("{}*", repo.key_prefix);
        let keys: Vec<Key> = {
            use futures::stream::TryStreamExt;
            repo.client
                .scan_buffered(pattern.as_str(), Some(SCAN_BATCH_COUNT), None)
                .try_collect()
                .await
                .expect("scan own prefix for cleanup")
        };
        if !keys.is_empty() {
            repo.client
                .del::<(), _>(keys)
                .await
                .expect("delete own prefix");
        }
    }

    /// A legacy row's first redemption swaps atomically but writes no
    /// retirement record — there is no prior generation to detect reuse
    /// against — and the presented hash reads Unknown afterwards. The
    /// honest-count probe: revoking the replacement's family afterwards
    /// removes exactly one entry (the replacement itself); a stray retirement
    /// record would make it two.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn legacy_row_first_redemption_swaps_without_retirement_record() {
        let repo = create_test_repo().await;
        let legacy_hash = session_contract::fixture_hash("valkey-legacy:first-redemption");

        let mut legacy = sample_session("usr_legacy", &legacy_hash, 3600);
        legacy.family_id = String::new();
        repo.store_refresh_token(&legacy)
            .await
            .expect("store legacy row");

        match repo
            .resolve_refresh_token(&legacy_hash)
            .await
            .expect("resolve legacy")
        {
            RefreshResolution::Live(session) => {
                assert_eq!(session.family_id, "", "sentinel family on read");
                assert_eq!(session.generation, 0);
                assert_eq!(session.rotated_at, None);
            }
            other => panic!("the stored legacy row must resolve Live, got {other:?}"),
        }

        let new_family = fixture_family_id("valkey-legacy:new-fam");
        assert!(is_valid_family_id(&new_family));
        let replacement = Session {
            refresh_token_hash: Secret::new(format!("{legacy_hash}-next")),
            family_id: new_family.clone(),
            ..legacy.clone()
        };

        let won = repo
            .rotate_refresh_token(&legacy_hash, &replacement)
            .await
            .expect("legacy first-redemption swap");
        assert!(won, "an uncontended legacy redemption must win its CAS");

        assert_eq!(
            repo.resolve_refresh_token(&legacy_hash)
                .await
                .expect("resolve consumed hash"),
            RefreshResolution::Unknown,
            "a consumed legacy row has no retained record and must read Unknown"
        );
        match repo
            .resolve_refresh_token(replacement.refresh_token_hash.expose())
            .await
            .expect("resolve replacement")
        {
            RefreshResolution::Live(session) => {
                assert_eq!(session.family_id, new_family);
                assert_eq!(session.user_id, "usr_legacy");
            }
            other => panic!("replacement must be Live, got {other:?}"),
        }

        // No retired key was written...
        let retired_exists: bool = repo
            .client
            .exists(repo.retired_key(&legacy_hash))
            .await
            .expect("exists on retired key");
        assert!(
            !retired_exists,
            "a legacy swap must not write a retirement record"
        );

        // ...proven again through the honest count: the new family holds
        // exactly one removable thing (the replacement itself).
        let revoked = repo
            .revoke_family(&new_family)
            .await
            .expect("revoke new family");
        assert_eq!(
            revoked, 1,
            "only the replacement may exist for the new family"
        );

        cleanup_prefix(&repo).await;
    }

    /// Negative space: a losing CAS against a missing live generation writes
    /// nothing at all.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn legacy_row_failed_cas_leaves_store_untouched() {
        let repo = create_test_repo().await;
        let legacy_hash = session_contract::fixture_hash("valkey-legacy:cas-failure");

        let mut legacy = sample_session("usr_legacy", &legacy_hash, 3600);
        legacy.family_id = String::new();
        repo.store_refresh_token(&legacy)
            .await
            .expect("store legacy row");

        let replacement = Session {
            refresh_token_hash: Secret::new(format!("{legacy_hash}-next")),
            family_id: fixture_family_id("valkey-legacy:cas-fam"),
            ..legacy.clone()
        };

        let won = repo
            .rotate_refresh_token("no-such-live-hash", &replacement)
            .await
            .expect("rotation against an unknown live hash");
        assert!(!won, "a missing live generation must lose the CAS");
        assert_eq!(
            repo.resolve_refresh_token(&legacy_hash)
                .await
                .expect("resolve legacy"),
            RefreshResolution::Live(legacy.clone()),
            "the legacy row must survive the lost race"
        );
        assert_eq!(
            repo.resolve_refresh_token(replacement.refresh_token_hash.expose())
                .await
                .expect("resolve loser's proposal"),
            RefreshResolution::Unknown,
            "the loser's replacement must never be installed"
        );

        cleanup_prefix(&repo).await;
    }

    /// A winning rotation writes every effect atomically: retirement record,
    /// successor pointer, family membership for both generations, and user-set
    /// membership for the replacement.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn rotation_writes_retired_record_and_both_set_memberships() {
        let repo = create_test_repo().await;
        let chain = family_chain("valkey:rotate-effects", 0, "usr_rotate_effects");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");

        let won = repo
            .rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
            .await
            .expect("rotate");
        assert!(won, "uncontended rotation must win");

        // Retirement record readable immediately (SR4) with the right shape.
        match repo
            .resolve_refresh_token(chain.gen0.refresh_token_hash.expose())
            .await
            .expect("resolve presented hash")
        {
            RefreshResolution::Superseded { live, .. } => {
                assert!(live.refresh_token_hash == chain.gen1.refresh_token_hash);
            }
            other => panic!("presented hash must resolve Superseded, got {other:?}"),
        }
        let record = repo
            .get_retired_record(chain.gen0.refresh_token_hash.expose())
            .await
            .expect("read retired record")
            .expect("retirement record must exist");
        assert!(record.successor_hash == *chain.gen1.refresh_token_hash.expose());
        assert_eq!(record.family_id, chain.family_id);
        assert_eq!(record.user_id, "usr_rotate_effects");
        assert!(
            record.expires_at > record.retired_at,
            "the retention deadline must lie strictly after the retirement instant"
        );
        assert!(
            record.expires_at <= chain.gen0.expires_at,
            "no record may outlive its family"
        );

        // Family set holds both generations; user set holds only the live one.
        let family_members: Vec<String> = repo
            .client
            .smembers(repo.family_key(&chain.family_id))
            .await
            .expect("smembers family set");
        assert!(family_members.contains(chain.gen0.refresh_token_hash.expose()));
        assert!(family_members.contains(chain.gen1.refresh_token_hash.expose()));

        // Counter untouched by the net-zero swap.
        assert_eq!(
            repo.count_active_sessions().await.expect("count"),
            1,
            "rotation must not move the active-sessions counter"
        );

        cleanup_prefix(&repo).await;
    }

    /// Expiry is never omitted and never extended past the family bound: a
    /// retirement record inside the retention window carries roughly the full
    /// retention TTL, while one whose family dies sooner is capped at the
    /// family's remaining life.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn retired_key_ttl_is_capped_at_family_expiry() {
        // Case 1: long-lived family -> TTL tracks the retention window.
        let repo = create_test_repo().await;
        let chain = family_chain("valkey:ttl-long", 0, "usr_ttl_long");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");
        assert!(
            repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
                .await
                .expect("rotate"),
            "rotation must win"
        );
        let long_ttl: i64 = repo
            .client
            .ttl(repo.retired_key(chain.gen0.refresh_token_hash.expose()))
            .await
            .expect("ttl of retired key");
        assert!(
            long_ttl > 3000 && long_ttl <= TEST_REUSE_RETENTION_SECS as i64,
            "retention-bounded TTL should be just under {TEST_REUSE_RETENTION_SECS}s, got {long_ttl}s"
        );

        // Case 2: family expiring in ~5s -> the record's TTL is capped at the
        // family's remaining life (well under the retention window).
        let short_chain = family_chain("valkey:ttl-short", 0, "usr_ttl_short");
        let mut dying = short_chain.gen0.clone();
        dying.expires_at = Utc::now() + chrono::Duration::seconds(5);
        repo.store_refresh_token(&dying)
            .await
            .expect("store dying gen0");
        let mut successor = short_chain.gen1.clone();
        successor.expires_at = dying.expires_at;
        assert!(
            repo.rotate_refresh_token(dying.refresh_token_hash.expose(), &successor)
                .await
                .expect("rotate dying family"),
            "rotation must win"
        );
        let short_ttl: i64 = repo
            .client
            .ttl(repo.retired_key(dying.refresh_token_hash.expose()))
            .await
            .expect("ttl of capped retired key");
        assert!(
            (SESSION_TTL_SECONDS_MIN..=5).contains(&short_ttl),
            "a record whose family dies sooner must be capped at the family bound \
             (<=5s), got {short_ttl}s"
        );

        cleanup_prefix(&repo).await;
    }

    /// Seeding the counter below its true value and then revoking must clamp:
    /// the call returns `Ok`, the key reads zero, and no panic escapes an
    /// unauthenticated revoke. Covers `revoke_session`.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn counter_drift_below_zero_clamps_on_revoke_session() {
        let repo = create_test_repo().await;
        let session = sample_session("usr_clamp", "hash_clamp", 3600);
        repo.store_refresh_token(&session)
            .await
            .expect("store session");

        // Simulate drift: the counter says zero even though one session is live.
        let counter_key = repo.active_sessions_key();
        let _: () = repo
            .client
            .set::<(), _, _>(&counter_key, 0_i64, None, None, false)
            .await
            .expect("seed drifted counter");

        repo.revoke_session(&session.refresh_token_hash)
            .await
            .expect("revoking with a drifted counter must succeed, not panic");

        let observed: Option<i64> = repo.client.get(&counter_key).await.expect("read counter");
        assert_eq!(
            observed,
            Some(0),
            "the clamped counter must read exactly zero after the revoke"
        );

        cleanup_prefix(&repo).await;
    }

    /// Same clamp discipline through the family-revocation path.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn counter_drift_below_zero_clamps_on_revoke_family() {
        let repo = create_test_repo().await;
        let chain = family_chain("valkey:clamp-family", 0, "usr_clamp_family");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");

        let counter_key = repo.active_sessions_key();
        let _: () = repo
            .client
            .set::<(), _, _>(&counter_key, 0_i64, None, None, false)
            .await
            .expect("seed drifted counter");

        let removed = repo
            .revoke_family(&chain.family_id)
            .await
            .expect("family revocation with a drifted counter must succeed, not panic");
        assert_eq!(removed, 1, "exactly the live generation is removed");

        let observed: Option<i64> = repo.client.get(&counter_key).await.expect("read counter");
        assert_eq!(observed, Some(0), "the clamped counter must read zero");

        cleanup_prefix(&repo).await;
    }

    /// `revoke_family` removes the live generation *and* retained retirement
    /// records, returns their combined count, and leaves sibling families
    /// alone.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn revoke_family_removes_live_generation_and_retired_records() {
        let repo = create_test_repo().await;
        let target = family_chain("valkey:revoke-family", 0, "usr_shared");
        let sibling = family_chain("valkey:revoke-family", 1, "usr_shared");
        repo.store_refresh_token(&target.gen0)
            .await
            .expect("store target");
        repo.store_refresh_token(&sibling.gen0)
            .await
            .expect("store sibling");
        assert!(
            repo.rotate_refresh_token(target.gen0.refresh_token_hash.expose(), &target.gen1)
                .await
                .expect("rotate target"),
            "target rotation must win"
        );
        assert!(
            repo.rotate_refresh_token(sibling.gen0.refresh_token_hash.expose(), &sibling.gen1)
                .await
                .expect("rotate sibling"),
            "sibling rotation must win"
        );

        let removed = repo
            .revoke_family(&target.family_id)
            .await
            .expect("revoke family");
        assert_eq!(
            removed, 2,
            "one live generation plus one retirement record must be removed"
        );
        assert_eq!(
            repo.resolve_refresh_token(target.gen1.refresh_token_hash.expose())
                .await
                .expect("resolve revoked live"),
            RefreshResolution::Unknown,
            "the revoked live generation must read Unknown"
        );
        assert_eq!(
            repo.resolve_refresh_token(target.gen0.refresh_token_hash.expose())
                .await
                .expect("resolve revoked record"),
            RefreshResolution::Unknown,
            "the revoked retirement record must be gone"
        );
        assert_eq!(
            repo.revoke_family(&target.family_id)
                .await
                .expect("second revoke"),
            0,
            "an already-empty family reports zero removals"
        );
        match repo
            .resolve_refresh_token(sibling.gen1.refresh_token_hash.expose())
            .await
            .expect("resolve sibling")
        {
            RefreshResolution::Live(session) => assert_eq!(session.user_id, "usr_shared"),
            other => panic!("sibling family must survive, got {other:?}"),
        }

        cleanup_prefix(&repo).await;
    }

    /// `revoke_all_user_sessions` sweeps the user's retirement records too,
    /// leaving another user's state alone.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn revoke_all_user_sessions_sweeps_retired_records_of_that_user_only() {
        let repo = create_test_repo().await;
        let mine = family_chain("valkey:revoke-all", 0, "usr_mine");
        let theirs = family_chain("valkey:revoke-all", 1, "usr_theirs");
        repo.store_refresh_token(&mine.gen0)
            .await
            .expect("store mine");
        repo.store_refresh_token(&theirs.gen0)
            .await
            .expect("store theirs");
        assert!(
            repo.rotate_refresh_token(mine.gen0.refresh_token_hash.expose(), &mine.gen1)
                .await
                .expect("rotate mine"),
            "my rotation must win"
        );
        assert!(
            repo.rotate_refresh_token(theirs.gen0.refresh_token_hash.expose(), &theirs.gen1)
                .await
                .expect("rotate theirs"),
            "their rotation must win"
        );

        repo.revoke_all_user_sessions("usr_mine")
            .await
            .expect("revoke all mine");

        assert_eq!(
            repo.resolve_refresh_token(mine.gen1.refresh_token_hash.expose())
                .await
                .expect("resolve my live"),
            RefreshResolution::Unknown,
            "my live generation must be gone"
        );
        assert_eq!(
            repo.resolve_refresh_token(mine.gen0.refresh_token_hash.expose())
                .await
                .expect("resolve my retired"),
            RefreshResolution::Unknown,
            "my retirement record must be gone"
        );
        match repo
            .resolve_refresh_token(theirs.gen1.refresh_token_hash.expose())
            .await
            .expect("resolve their live")
        {
            RefreshResolution::Live(session) => assert_eq!(session.user_id, "usr_theirs"),
            other => panic!("another user's family must survive my revocation, got {other:?}"),
        }
        let their_record = repo
            .get_retired_record(theirs.gen0.refresh_token_hash.expose())
            .await
            .expect("read their record");
        assert!(
            their_record.is_some(),
            "another user's retirement record must survive my revoke-all"
        );

        cleanup_prefix(&repo).await;
    }

    /// Cleanup prunes dead members from `{prefix}family:*` sets (whose
    /// memberships never expire on their own), deletes emptied sets, and
    /// counts the pruned memberships together with the user-set work.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn cleanup_prunes_dead_family_set_members() {
        let repo = create_test_repo().await;
        let chain = family_chain("valkey:cleanup-family", 0, "usr_cleanup_family");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");
        assert!(
            repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
                .await
                .expect("rotate"),
            "rotation must win"
        );

        // Kill both generations directly (bypassing the port) so their index
        // memberships go stale: natural TTL expiry never shrinks a set on its
        // own, which is exactly what cleanup exists to reap. Afterwards the
        // user set holds one dead member (gen1) and the family set holds two
        // (gen0's retirement and gen1) — three stale memberships total.
        let deleted: u64 = repo
            .client
            .del(vec![
                repo.session_key(chain.gen0.refresh_token_hash.expose()),
                repo.retired_key(chain.gen0.refresh_token_hash.expose()),
                repo.session_key(chain.gen1.refresh_token_hash.expose()),
            ])
            .await
            .expect("delete both generations");
        assert_eq!(
            deleted, 2,
            "two keys existed (record + live); the third was absent"
        );

        let removed = repo.cleanup_expired_sessions().await.expect("cleanup");
        assert_eq!(
            removed, 3,
            "cleanup must prune exactly the stale memberships across both index \
             structures: one in the user set plus two in the family set"
        );

        let set_exists: bool = repo
            .client
            .exists(repo.family_key(&chain.family_id))
            .await
            .expect("exists on family set");
        assert!(
            !set_exists,
            "the emptied family set must be deleted by cleanup"
        );

        cleanup_prefix(&repo).await;
    }

    /// Drop every key under a test repository's prefix so successive runs
    /// leave nothing behind.
    async fn cleanup_prefix(repo: &ValkeySessionRepository) {
        let pattern = format!("{}*", repo.key_prefix);
        let keys: Vec<Key> = {
            use futures::stream::TryStreamExt;
            repo.client
                .scan_buffered(pattern.as_str(), Some(SCAN_BATCH_COUNT), None)
                .try_collect()
                .await
                .expect("scan own prefix for cleanup")
        };
        if !keys.is_empty() {
            repo.client
                .del::<(), _>(keys)
                .await
                .expect("delete own prefix");
        }
    }

    // -- Single-use conformance (shared suite in test-utils) --------------------
    // Valkey expires single-use records natively (`SET … EX`), so the cleanup-sweep
    // scenario does not apply and is deliberately not invoked here.

    use oidc_exchange_test_utils::single_use_conformance as conformance;

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn single_use_first_claim_wins_duplicate_loses() {
        let repo = create_test_repo().await;
        conformance::first_claim_wins_duplicate_loses(&repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn single_use_consume_live_record_exactly_once() {
        let repo = create_test_repo().await;
        conformance::consume_live_record_exactly_once(&repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn single_use_expired_record_is_absent_to_put_and_take() {
        let repo = create_test_repo().await;
        conformance::expired_record_is_absent_to_put_and_take(&repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn single_use_concurrent_put_has_exactly_one_winner() {
        let repo = std::sync::Arc::new(create_test_repo().await);
        conformance::concurrent_put_has_exactly_one_winner(repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn single_use_concurrent_take_has_exactly_one_winner() {
        let repo = std::sync::Arc::new(create_test_repo().await);
        conformance::concurrent_take_has_exactly_one_winner(repo).await;
    }

    /// Negative-space specific to the native-TTL store: a put whose expiry is not
    /// strictly ahead of now must be refused before any key exists, so no born-dead or
    /// TTL-less record is ever written.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn single_use_put_rejects_non_future_expiry_without_creating_a_key() {
        let repo = create_test_repo().await;

        let err = repo
            .put_single_use("nonce:past", Utc::now() - chrono::Duration::seconds(1))
            .await
            .expect_err("a past-expiry put must be refused");
        assert!(matches!(err, Error::StoreError { .. }));

        let key = repo.single_use_key("nonce:past");
        let exists: bool = repo.client.exists(&key).await.expect("exists probe");
        assert!(!exists, "a refused put must not leave any key behind");
    }

    // ---------------------------------------------------------------
    // Span-redaction regression tests (same capture technique as LMDB's)
    // ---------------------------------------------------------------

    /// Distinctive marker strings planted in a session's sensitive fields; none of them may
    /// ever surface in captured span output.
    const HASH_SENTINEL: &str = "feedface0123456789abcdefcafebabefeedface0123456789abcdef0987";
    const DEVICE_SENTINEL: &str = "valkey-span-device-sentinel";
    const USER_AGENT_SENTINEL: &str = "valkey-span-user-agent/1.0";
    const IP_SENTINEL: &str = "198.51.100.7";
    const USER_ID_SENTINEL: &str = "usr_valkey_span_redaction";

    /// All client-provenance values carried by a session; no span may record any of them,
    /// nor the hash.
    const PROVENANCE_SENTINELS: [&str; 3] = [DEVICE_SENTINEL, USER_AGENT_SENTINEL, IP_SENTINEL];

    /// A clonable in-memory writer the fmt subscriber renders into, so tests can assert on
    /// exactly what was produced rather than scraping stdout.
    #[derive(Clone, Default)]
    struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("capture mutex must not be poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Records the *declared* field schema `(span name, field name)` of every span created
    /// while installed. The fmt formatter renders only fields that actually received a
    /// value — an empty-but-declared schema field like `token_hash` never appears in the
    /// text stream — so schema observability can only be proven against the metadata.
    struct DeclaredFieldsLayer {
        declared: Arc<Mutex<HashSet<(String, String)>>>,
    }

    impl<S> Layer<S> for DeclaredFieldsLayer
    where
        S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            _id: &tracing::span::Id,
            _ctx: Context<'_, S>,
        ) {
            let span_name = attrs.metadata().name().to_string();
            let mut declared = self
                .declared
                .lock()
                .expect("capture mutex must not be poisoned");
            for field in attrs.metadata().fields() {
                declared.insert((span_name.clone(), field.name().to_string()));
            }
            assert!(
                !declared.is_empty(),
                "every instrumented span must declare its field schema"
            );
        }
    }

    /// The capture bundle a span-leak test needs: hold `_guard` for the whole test body,
    /// read rendered telemetry from `buffer`, and assert declared schema via `declared`.
    struct SpanCapture {
        _gate: std::sync::MutexGuard<'static, ()>,
        _guard: tracing::subscriber::DefaultGuard,
        buffer: SharedBuffer,
        declared: Arc<Mutex<HashSet<(String, String)>>>,
    }

    /// Install a fmt subscriber writing into `buffer` with explicit span-open/close events
    /// enabled, alongside the schema-capture layer. Without `FmtSpan::CLOSE` the stock
    /// subscriber would never render span teardown, letting a broken skip look clean
    /// because nothing was asserted against.
    ///
    /// Keep the returned handle alive for the whole test body: dropping it uninstalls the
    /// thread-local subscriber.
    fn install_span_capture(buffer: SharedBuffer) -> SpanCapture {
        let declared = Arc::new(Mutex::new(HashSet::new()));
        // The writer closure owns a clone; the test keeps the original for assertions.
        let writer_buffer = buffer.clone();
        let subscriber = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(move || writer_buffer.clone())
                    .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                    .with_ansi(false),
            )
            .with(DeclaredFieldsLayer {
                declared: declared.clone(),
            });
        // Serialize capture tests process-wide (concurrent thread-local
        // subscriber installs race tracing's callsite interest cache), then
        // rebuild the cache under the installed subscriber.
        let gate = oidc_exchange_test_utils::telemetry::CAPTURE_GATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let guard = tracing::subscriber::set_default(subscriber);
        tracing::callsite::rebuild_interest_cache();
        SpanCapture {
            _gate: gate,
            _guard: guard,
            buffer,
            declared,
        }
    }

    /// Asserts a span declared exactly one of its expected schema fields, so the log-schema
    /// contract survives even though the formatter prints nothing for empty fields.
    fn assert_declares(
        declared: &Mutex<HashSet<(String, String)>>,
        span_name: &str,
        field_name: &str,
    ) {
        let key = (span_name.to_string(), field_name.to_string());
        assert!(
            declared
                .lock()
                .expect("capture mutex must not be poisoned")
                .contains(&key),
            "span {span_name} must keep declaring the {field_name} schema field"
        );
    }

    fn sentinel_session() -> Session {
        let now = Utc::now();
        Session {
            user_id: USER_ID_SENTINEL.to_string(),
            refresh_token_hash: Secret::new(HASH_SENTINEL.to_string()),
            family_id: oidc_exchange_core::domain::new_family_id(),
            generation: 0,
            rotated_at: None,
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(1),
            device_id: Some(DEVICE_SENTINEL.to_string()),
            user_agent: Some(USER_AGENT_SENTINEL.to_string()),
            ip_address: Some(IP_SENTINEL.to_string()),
            created_at: now,
        }
    }

    fn rendered_output(buffer: &SharedBuffer) -> String {
        let bytes = buffer
            .0
            .lock()
            .expect("capture mutex must not be poisoned")
            .clone();
        String::from_utf8(bytes).expect("captured telemetry is utf-8")
    }

    /// Regression for the Valkey whole-session span exposure: across write, lookup, and
    /// revoke, neither the refresh-token hash nor the client provenance recorded on the
    /// session may render, while `user_id` and the declared-but-empty `token_hash` schema
    /// field stay observable. Requires a live Valkey like its sibling integration tests.
    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn session_spans_exclude_hash_and_provenance_but_keep_permitted_fields() {
        let buffer = SharedBuffer::default();
        // Single-threaded `#[tokio::test]`: every poll happens on this thread, so the
        // thread-local default subscriber sees every span open and close below.
        let capture = install_span_capture(buffer);
        let repo = create_test_repo().await;
        let session = sentinel_session();

        repo.store_refresh_token(&session)
            .await
            .expect("store_refresh_token");
        let fetched = repo
            .get_session_by_refresh_token(&session.refresh_token_hash)
            .await
            .expect("get_session_by_refresh_token")
            .expect("stored session must be retrievable");
        assert_eq!(fetched.user_id, session.user_id, "lookup must find the row");
        repo.revoke_session(&session.refresh_token_hash)
            .await
            .expect("revoke_session");

        let rendered = rendered_output(&capture.buffer);

        // Non-vacuousness: all three instrumented spans must have both opened and closed
        // inside this capture before any absence claim means anything.
        for span_name in [
            "store_refresh_token",
            "get_session_by_refresh_token",
            "revoke_session",
        ] {
            let mentions = rendered.matches(span_name).count();
            assert!(
                mentions >= 2,
                "span {span_name} must appear at both open and close, found {mentions}"
            );
        }
        assert_eq!(
            rendered.matches("close").count(),
            3,
            "exactly the three driven spans must have closed in this capture"
        );

        // Permitted observability survives: the write span records `user_id`, and the
        // lookup/revoke spans keep their (value-less) `token_hash` log-schema field.
        assert!(
            rendered.contains(&format!("user_id={USER_ID_SENTINEL}")),
            "the write span must still record user_id"
        );
        assert_declares(&capture.declared, "store_refresh_token", "user_id");
        assert_declares(
            &capture.declared,
            "get_session_by_refresh_token",
            "token_hash",
        );
        assert_declares(&capture.declared, "revoke_session", "token_hash");

        // Negative space: no sensitive value anywhere in the rendered telemetry.
        assert!(
            !rendered.contains(HASH_SENTINEL),
            "refresh-token hash must never reach span output"
        );
        for provenance in PROVENANCE_SENTINELS {
            assert!(
                !rendered.contains(provenance),
                "client provenance value ({provenance}) must never reach span output"
            );
        }
    }
}
