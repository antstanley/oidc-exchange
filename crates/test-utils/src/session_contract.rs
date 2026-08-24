//! Shared session-store conformance harness — the SR1–SR5 obligations of
//! [`oidc_exchange_core::ports::SessionRepository`] as executable, generic
//! assertions.
//!
//! Every session adapter — DynamoDB, Postgres, SQLite, LMDB, Valkey — and
//! `MockRepository` invoke the same assertions from their own test modules, so
//! the guarantee is a property the project asserts rather than one it assumes.
//! The assertions are deliberately granular (one named function per obligation,
//! each with its own negative-space check inside) so an adapter can invoke a
//! subset while its implementation is still interim and the whole suite once
//! complete — without any of the assertions weakening.
//!
//! # Determinism
//!
//! Identifiers are fully deterministic: family ids and token hashes derive
//! from SHA-256 of the caller's `tag`, so two suites run against one shared
//! backend (a Valkey prefix, a DynamoDB table) never collide and re-runs are
//! reproducible. Time is deterministic *in relation*: each assertion captures
//! one base instant and derives every timestamp from it by explicit offsets,
//! so the relationships the contract cares about (`expires_at` equality,
//! ordering of retirements) hold exactly no matter when the suite runs.
//!
//! # Adapter-facing invocation pattern
//!
//! Normal-tier adapters (SQLite, LMDB, Valkey, `MockRepository`) call the
//! assertions directly from `#[tokio::test]`s. Environment-gated integration
//! adapters (DynamoDB) call the *same* functions from `#[ignore]`d tests —
//! the assertions are identical, only the gating differs:
//!
//! ```text
//! #[tokio::test]
//! #[ignore = "requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local"]
//! async fn dynamodb_session_store_meets_sr1_through_sr5() {
//!     let store = DynamoDbSessionStore::from_env().await.expect("dynamodb-local");
//!     oidc_exchange_test_utils::session_contract::assert_full_conformance(
//!         &store,
//!         "dynamo-session-conformance",
//!     )
//!     .await;
//! }
//! ```
//!
//! A tag must be unique per (backend, suite) pair — it namespaces every
//! fixture the suite creates, which is what makes concurrent or repeated runs
//! against one physical backend safe.
//!
//! # Obligation map
//!
//! | Assertion | Obligation |
//! |---|---|
//! | [`assert_resolution_classifies_all_four_shapes`] | SR1 (classification surface) |
//! | [`assert_rotation_installs_successor_and_demotes_presented`] | SR2 observable effects |
//! | [`assert_failed_cas_leaves_store_byte_identical`] | SR2 (negative space) |
//! | [`assert_concurrent_rotation_yields_exactly_one_winner`] | SR3 |
//! | [`assert_retirement_readable_immediately_after_rotation`] | SR4 |
//! | [`assert_older_generation_resolves_as_retired`] | SR1/SR4 (retained history) |
//! | [`assert_family_revocation_removes_everything_and_returns_count`] | SR5 |
//! | [`assert_resolution_unknown_immediately_after_revoke`] | SR1 (negative space) |
//! | [`assert_rotation_preserves_absolute_expiry`] | SR2 (expiry inheritance) |

use std::time::Duration;

use chrono::{DateTime, Utc};
use oidc_exchange_core::domain::{is_valid_family_id, RefreshResolution, Session};
use oidc_exchange_core::ports::SessionRepository;
use oidc_exchange_core::secret::Secret;
use sha2::{Digest, Sha256};
use tokio::time::timeout;

// ---------------------------------------------------------------------------
// Named constants bounding the suite's concurrency and time budgets
// ---------------------------------------------------------------------------

/// Competing rotations raced against one live hash in
/// [`assert_concurrent_rotation_yields_exactly_one_winner`]. Exactly two: the
/// minimum race that proves single-live-generation — one winner, one loser.
pub const CONCURRENT_ROTATIONS: usize = 2;

/// Wall-clock bound for awaiting the two competing rotations. A store that
/// cannot answer two concurrent redemptions inside this budget fails the
/// suite rather than hanging the test run.
pub const RACE_JOIN_TIMEOUT: Duration = Duration::from_secs(10);

/// The family's absolute lifetime in fixture chains: `base + 24h`. Long
/// enough that no assertion runs past the deadline, and identical to the
/// retention arithmetic the mock store exercises.
pub const FIXTURE_FAMILY_TTL_SECS: i64 = 24 * 60 * 60;

/// The lowercase Crockford-base32 alphabet ULIDs render in — the same
/// alphabet `oidc_exchange_core::domain::is_valid_family_id` accepts.
const CROCKFORD_LOWERCASE: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

// ---------------------------------------------------------------------------
// Deterministic identifiers and clock
// ---------------------------------------------------------------------------

/// SHA-256 hex digest of `seed` — the exact form a refresh-token hash takes
/// everywhere in the system (the suite only ever handles hashes).
pub fn fixture_hash(seed: &str) -> String {
    hex::encode(Sha256::digest(seed.as_bytes()))
}

/// A well-formed session-family id (`fam_` + 26 lowercase Crockford
/// characters) derived deterministically from `seed`. Distinct seeds yield
/// distinct ids with overwhelming probability, so per-suite tags keep fixture
/// families disjoint even on one shared physical backend.
pub fn fixture_family_id(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    let mut ulid_part = String::with_capacity(26);
    for byte in digest.iter().take(26) {
        ulid_part.push(CROCKFORD_LOWERCASE[(byte % 32) as usize] as char);
    }
    format!("fam_{ulid_part}")
}

/// One captured instant from which every timestamp a suite run derives is
/// computed by explicit offsets. Callers capture it once per assertion; no
/// assertion reads the wall clock twice, so timestamp *relationships* are
/// exact even though the base itself is "now".
pub fn capture_base_instant() -> DateTime<Utc> {
    Utc::now()
}

/// Build one generation of a fixture family. `expires_at` and `created_at`
/// are passed in explicitly so successor generations inherit them exactly as
/// the rotation contract requires; `issued_at` backs `rotated_at` (`None` at
/// generation 0).
pub fn generation_session(
    user_id: &str,
    family_id: &str,
    generation: u32,
    token_hash: String,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    issued_at: Option<DateTime<Utc>>,
) -> Session {
    debug_assert!(
        is_valid_family_id(family_id),
        "fixture family id must be well-formed: {family_id}"
    );
    Session {
        user_id: user_id.to_string(),
        refresh_token_hash: Secret::new(token_hash),
        family_id: family_id.to_string(),
        generation,
        provider: "conformance".to_string(),
        expires_at,
        rotated_at: issued_at,
        device_id: None,
        user_agent: None,
        ip_address: None,
        created_at,
    }
}

/// A three-generation fixture chain for one family: `gen0` is the live
/// generation the suite stores, `gen1` and `gen2` are prepared successors
/// that already inherit `gen0`'s absolute `expires_at` and `created_at` per
/// the rotation contract. `alt_gen1` is a second, distinct candidate for the
/// same slot as `gen1` — same family, same inheritance, different hash —
/// which is what the concurrency assertion races `gen1` against (two clients
/// redeeming one token, each minting their own replacement). All identifiers
/// derive from `tag`, so two chains built from different tags never touch
/// the same keys.
pub struct FamilyChain {
    /// The family's identifier (`fam_…`).
    pub family_id: String,
    /// The owner the suite uses for the family.
    pub user_id: String,
    /// Generation 0 — the chain's live generation when stored.
    pub gen0: Session,
    /// Successor of gen0 (inherits gen0's expiry/creation timestamps).
    pub gen1: Session,
    /// A second candidate for gen1's slot: same family, same inheritance,
    /// different hash.
    pub alt_gen1: Session,
    /// Successor of gen1 (inherits gen0's expiry/creation timestamps).
    pub gen2: Session,
}

/// Build a [`FamilyChain`] namespaced by `tag`. `index` lets one suite run
/// several disjoint families (e.g. a family under test and a sibling family
/// that must survive revocation).
pub fn family_chain(tag: &str, index: usize, user_id: &str) -> FamilyChain {
    let family_id = fixture_family_id(&format!("{tag}:family:{index}"));
    let base = capture_base_instant();
    let created_at = base;
    let expires_at = base + chrono::Duration::seconds(FIXTURE_FAMILY_TTL_SECS);
    let hash = |label: &str| fixture_hash(&format!("{tag}:family:{index}:{label}"));
    let issued = |gen: u32| (gen > 0).then(|| base + chrono::Duration::seconds(gen as i64));

    FamilyChain {
        family_id: family_id.clone(),
        user_id: user_id.to_string(),
        gen0: generation_session(
            user_id,
            &family_id,
            0,
            hash("gen0"),
            expires_at,
            created_at,
            issued(0),
        ),
        gen1: generation_session(
            user_id,
            &family_id,
            1,
            hash("gen1"),
            expires_at,
            created_at,
            issued(1),
        ),
        alt_gen1: generation_session(
            user_id,
            &family_id,
            1,
            hash("gen1-alt"),
            expires_at,
            created_at,
            issued(1),
        ),
        gen2: generation_session(
            user_id,
            &family_id,
            2,
            hash("gen2"),
            expires_at,
            created_at,
            issued(2),
        ),
    }
}

// ---------------------------------------------------------------------------
// Generic assertions — one named function per SR obligation
// ---------------------------------------------------------------------------

/// **SR1 (classification surface).** Over one family's life the four
/// classification shapes appear in the contract's order: the live generation
/// resolves `Live`; after one rotation the presented hash resolves
/// `Superseded` naming the live successor; after the successor itself falls
/// the two-generations-old hash resolves `Retired` carrying family and user;
/// a never-seen hash resolves `Unknown`. Negative space: a fallen generation
/// must *not* keep claiming grace (`Superseded`) once its successor is gone,
/// and nothing ever resolves as an error.
pub async fn assert_resolution_classifies_all_four_shapes<S: SessionRepository + ?Sized>(
    repo: &S,
    tag: &str,
) {
    let chain = family_chain(tag, 0, "usr_conformance");
    repo.store_refresh_token(&chain.gen0)
        .await
        .expect("store generation 0");

    // Live: the hash is the family's current generation.
    assert_eq!(
        repo.resolve_refresh_token(chain.gen0.refresh_token_hash.expose())
            .await
            .expect("resolve live generation"),
        RefreshResolution::Live(chain.gen0.clone()),
        "the stored generation must resolve Live before any rotation"
    );

    assert!(
        repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
            .await
            .expect("first rotation"),
        "the first rotation must win its CAS"
    );

    // Superseded: gen0 is retired and its named successor is still live.
    match repo
        .resolve_refresh_token(chain.gen0.refresh_token_hash.expose())
        .await
        .expect("resolve retired generation 0")
    {
        RefreshResolution::Superseded { live, .. } => {
            assert!(
                live.refresh_token_hash == chain.gen1.refresh_token_hash,
                "Superseded must name the live successor, not the presented hash"
            );
        }
        other => panic!(
            "a retired generation whose successor is live must resolve Superseded, got {other:?}"
        ),
    }

    assert!(
        repo.rotate_refresh_token(chain.gen1.refresh_token_hash.expose(), &chain.gen2)
            .await
            .expect("second rotation"),
        "the second rotation must win its CAS"
    );

    // Retired: gen0's successor (gen1) is no longer live — reuse, not grace.
    match repo
        .resolve_refresh_token(chain.gen0.refresh_token_hash.expose())
        .await
        .expect("resolve two-generations-old hash")
    {
        RefreshResolution::Retired {
            family_id, user_id, ..
        } => {
            assert_eq!(
                family_id, chain.family_id,
                "Retired must carry the family id"
            );
            assert_eq!(user_id, chain.user_id, "Retired must carry the user id");
        }
        other => {
            panic!("a generation whose successor has fallen must resolve Retired, got {other:?}")
        }
    }
    // Unknown: nothing live and nothing retained matches.
    assert_eq!(
        repo.resolve_refresh_token(&fixture_hash(&format!("{tag}:never-seen")))
            .await
            .expect("resolve unknown hash"),
        RefreshResolution::Unknown,
        "a never-seen hash must resolve Unknown"
    );
}

/// **SR2 (observable effects).** A winning rotation installs the replacement
/// as the family's live generation and the presented hash stops resolving as
/// `Live` — it becomes a `Superseded` retirement naming the successor, and
/// `get_session_by_refresh_token` (the liveness lookup `/revoke` uses) no
/// longer returns it. Negative space: the retirement lookup must not keep
/// answering as though the presented generation were still live.
pub async fn assert_rotation_installs_successor_and_demotes_presented<
    S: SessionRepository + ?Sized,
>(
    repo: &S,
    tag: &str,
) {
    let chain = family_chain(tag, 0, "usr_conformance");
    repo.store_refresh_token(&chain.gen0)
        .await
        .expect("store generation 0");

    assert!(
        repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
            .await
            .expect("rotation"),
        "the rotation must win its CAS"
    );

    assert_eq!(
        repo.resolve_refresh_token(chain.gen1.refresh_token_hash.expose())
            .await
            .expect("resolve successor"),
        RefreshResolution::Live(chain.gen1.clone()),
        "the replacement must be the family's live generation after the swap"
    );
    assert!(
        matches!(
            repo.resolve_refresh_token(chain.gen0.refresh_token_hash.expose())
                .await
                .expect("resolve presented hash"),
            RefreshResolution::Superseded { .. }
        ),
        "the presented hash must demote to Superseded, never stay Live"
    );
    assert_eq!(
        repo.get_session_by_refresh_token(&chain.gen0.refresh_token_hash)
            .await
            .expect("liveness lookup for presented hash"),
        None,
        "a retired generation must not answer the live-session lookup"
    );
}

/// **SR2 (negative space).** A losing compare-and-swap is a complete no-op:
/// every piece of state observable through the port — the classification of
/// every hash in play, the live-session lookup for each, and the active
/// count — is identical before and after. The assertion loses the race two
/// ways (against a hash whose generation already moved, and against a hash
/// that never existed); both must return `false` rather than error, and the
/// loser's proposed replacement must never appear in the store in any form.
pub async fn assert_failed_cas_leaves_store_byte_identical<S: SessionRepository + ?Sized>(
    repo: &S,
    tag: &str,
) {
    let chain = family_chain(tag, 0, "usr_conformance");
    let sibling = family_chain(tag, 1, "usr_conformance");
    repo.store_refresh_token(&chain.gen0)
        .await
        .expect("store generation 0");
    repo.store_refresh_token(&sibling.gen0)
        .await
        .expect("store sibling family");
    assert!(
        repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
            .await
            .expect("rotation"),
        "the setup rotation must win"
    );

    // The losing caller proposes gen2 — a legitimate-looking next generation
    // of the same family — plus races a hash that never existed.
    let unknown_hash = fixture_hash(&format!("{tag}:unknown-cas-target"));

    /// Everything the port can observe about the hashes in play, captured as
    /// one comparable snapshot: the classification and the full live-session
    /// lookup for every hash, plus the store-wide active count. `PartialEq`
    /// on the results makes "identical before and after" exact.
    async fn snapshot<S: SessionRepository + ?Sized>(
        repo: &S,
        hashes: &[&str],
    ) -> (Vec<RefreshResolution>, Vec<Option<Session>>, u64) {
        let mut resolutions = Vec::with_capacity(hashes.len());
        let mut lookups = Vec::with_capacity(hashes.len());
        for hash in hashes {
            resolutions.push(
                repo.resolve_refresh_token(hash)
                    .await
                    .expect("snapshot resolve"),
            );
            lookups.push(
                repo.get_session_by_refresh_token(&Secret::new(hash.to_string()))
                    .await
                    .expect("snapshot liveness lookup"),
            );
        }
        let active = repo
            .count_active_sessions()
            .await
            .expect("snapshot active count");
        (resolutions, lookups, active)
    }

    let hashes = [
        chain.gen0.refresh_token_hash.expose().as_str(),
        chain.gen1.refresh_token_hash.expose().as_str(),
        sibling.gen0.refresh_token_hash.expose().as_str(),
        chain.gen2.refresh_token_hash.expose().as_str(),
        unknown_hash.as_str(),
    ];
    let before = snapshot(repo, &hashes).await;

    // Lose against a generation that already moved…
    assert!(
        !repo
            .rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen2)
            .await
            .expect("stale CAS must report false, not error"),
        "a CAS against a moved live generation must return false"
    );
    // …and against a hash that never existed.
    assert!(
        !repo
            .rotate_refresh_token(&unknown_hash, &chain.gen2)
            .await
            .expect("unknown-hash CAS must report false, not error"),
        "a CAS against an unknown hash must return false"
    );

    let after = snapshot(repo, &hashes).await;
    assert_eq!(
        before, after,
        "a losing compare-and-swap must leave every observable byte of state identical"
    );

    // The loser's proposal must not exist as live, retired, or anything else.
    assert_eq!(
        repo.resolve_refresh_token(chain.gen2.refresh_token_hash.expose())
            .await
            .expect("resolve loser's proposal"),
        RefreshResolution::Unknown,
        "the losing caller's replacement must never be installed in any form"
    );
}

/// **SR3 (single live generation).** Two concurrent `rotate_refresh_token`
/// calls against the same live hash produce exactly one `true`. The two
/// competing operations are awaited together — each makes progress while
/// the other is suspended — under a named timeout. The assertion then
/// verifies the *store* agrees: the winner's replacement is the only live
/// generation, the loser's replacement does not exist, and the presented
/// hash names the winner as its successor. Negative space: the loser
/// observes `false`, never an error, and never a second live generation.
pub async fn assert_concurrent_rotation_yields_exactly_one_winner<S: SessionRepository + ?Sized>(
    repo: &S,
    tag: &str,
) {
    let chain = family_chain(tag, 0, "usr_conformance");
    repo.store_refresh_token(&chain.gen0)
        .await
        .expect("store generation 0");

    let live_hash = chain.gen0.refresh_token_hash.clone();
    let proposal_a = chain.gen1.clone();
    let proposal_b = chain.alt_gen1.clone();

    // Await both competitors concurrently, bounded by the suite's join
    // timeout so a wedged store fails instead of hanging the run.
    let (result_a, result_b) = timeout(RACE_JOIN_TIMEOUT, async {
        tokio::join!(
            repo.rotate_refresh_token(live_hash.expose(), &proposal_a),
            repo.rotate_refresh_token(live_hash.expose(), &proposal_b)
        )
    })
    .await
    .expect("the concurrent rotations must complete inside RACE_JOIN_TIMEOUT");
    let outcome_a = result_a.expect("first competing CAS must not error");
    let outcome_b = result_b.expect("second competing CAS must not error");
    assert!(
        outcome_a ^ outcome_b,
        "exactly one of two competing rotations may win (SR3); got {outcome_a:?} / {outcome_b:?}"
    );

    // Exactly one successor was installed: whichever proposal returned true.
    let winner = if outcome_a {
        &chain.gen1
    } else {
        &chain.alt_gen1
    };
    let loser_hash = if outcome_a {
        &chain.alt_gen1.refresh_token_hash
    } else {
        &chain.gen1.refresh_token_hash
    };
    assert_eq!(
        repo.resolve_refresh_token(winner.refresh_token_hash.expose())
            .await
            .expect("resolve winner"),
        RefreshResolution::Live(winner.clone()),
        "the winning replacement must be the family's only live generation"
    );
    assert_eq!(
        repo.resolve_refresh_token(loser_hash.expose())
            .await
            .expect("resolve loser's proposal"),
        RefreshResolution::Unknown,
        "the losing replacement must not exist in the store"
    );
    match repo
        .resolve_refresh_token(chain.gen0.refresh_token_hash.expose())
        .await
        .expect("resolve presented hash")
    {
        RefreshResolution::Superseded { live, .. } => {
            assert!(
                live.refresh_token_hash == winner.refresh_token_hash,
                "the presented hash must name the winner — and only the winner — as its successor"
            );
        }
        other => panic!("the presented hash must resolve Superseded after the race, got {other:?}"),
    }
}

/// **SR4 (retirement durability).** The retirement record a rotation writes
/// is readable the instant the rotation itself is observable: immediately
/// after a winning swap, the presented hash classifies `Superseded` — which
/// is only possible if the record exists and its successor pointer names the
/// now-live replacement. A store that installed the replacement before its
/// record would leave the presented hash reading `Unknown`, which this
/// assertion rejects.
pub async fn assert_retirement_readable_immediately_after_rotation<
    S: SessionRepository + ?Sized,
>(
    repo: &S,
    tag: &str,
) {
    let chain = family_chain(tag, 0, "usr_conformance");
    repo.store_refresh_token(&chain.gen0)
        .await
        .expect("store generation 0");

    assert!(
        repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
            .await
            .expect("rotation"),
        "the rotation must win its CAS"
    );

    // The first observation after the rotation must already see the record.
    match repo
        .resolve_refresh_token(chain.gen0.refresh_token_hash.expose())
        .await
        .expect("resolve presented hash immediately after rotation")
    {
        RefreshResolution::Superseded { live, .. } => {
            assert!(
                live.refresh_token_hash == chain.gen1.refresh_token_hash,
                "the immediately-readable record must name the new live generation"
            );
        }
        RefreshResolution::Unknown => panic!(
            "the retirement record was not readable the instant the rotation was (SR4 violation: reuse would read as Unknown)"
        ),
        other => panic!(
            "immediately after rotation the presented hash must resolve Superseded, got {other:?}"
        ),
    }
}

/// **SR1/SR4 (retained history).** A generation retired more than one
/// rotation ago — its successor is no longer live — resolves as `Retired`
/// carrying the family and user, never as `Unknown` and never as
/// `Superseded`. This is the shape that turns a replayed old token into a
/// reuse alarm instead of a silent rejection.
pub async fn assert_older_generation_resolves_as_retired<S: SessionRepository + ?Sized>(
    repo: &S,
    tag: &str,
) {
    let chain = family_chain(tag, 0, "usr_conformance");
    repo.store_refresh_token(&chain.gen0)
        .await
        .expect("store generation 0");
    assert!(
        repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
            .await
            .expect("first rotation"),
        "the first rotation must win"
    );
    assert!(
        repo.rotate_refresh_token(chain.gen1.refresh_token_hash.expose(), &chain.gen2)
            .await
            .expect("second rotation"),
        "the second rotation must win"
    );

    match repo
        .resolve_refresh_token(chain.gen0.refresh_token_hash.expose())
        .await
        .expect("resolve older generation")
    {
        RefreshResolution::Retired {
            family_id, user_id, ..
        } => {
            assert_eq!(
                family_id, chain.family_id,
                "Retired must carry the family id"
            );
            assert_eq!(user_id, chain.user_id, "Retired must carry the user id");
        }
        other => panic!("a two-generations-old hash must resolve Retired, got {other:?}"),
    }
}

/// **SR5 (revocation completeness).** `revoke_family` removes the family's
/// live generation *and* every retained retirement record, returns their
/// combined count, and touches nothing outside the family. Negative space:
/// a second revocation of the same (now empty) family reports `0` — the
/// count is honest about work not done — and an unknown but well-formed
/// family id also reports `Ok(0)` rather than erroring.
pub async fn assert_family_revocation_removes_everything_and_returns_count<
    S: SessionRepository + ?Sized,
>(
    repo: &S,
    tag: &str,
) {
    let family_a = family_chain(tag, 0, "usr_shared");
    let family_b = family_chain(tag, 1, "usr_shared");
    repo.store_refresh_token(&family_a.gen0)
        .await
        .expect("store family A");
    repo.store_refresh_token(&family_b.gen0)
        .await
        .expect("store family B");
    assert!(
        repo.rotate_refresh_token(family_a.gen0.refresh_token_hash.expose(), &family_a.gen1)
            .await
            .expect("rotate family A"),
        "family A's rotation must win"
    );

    // The store-wide active count is captured before revocation so the
    // removal check below is relative — the assertion must hold even on a
    // backend that carries unrelated live families.
    let active_before = repo
        .count_active_sessions()
        .await
        .expect("active count before revocation");

    // Family A: one live generation (gen1) + one retirement record (gen0).
    let removed = repo
        .revoke_family(&family_a.family_id)
        .await
        .expect("revoke family A");
    assert_eq!(
        removed, 2,
        "the count must cover the live row plus the retirement record (SR5)"
    );

    // Exactly one *live* session left the active set — the store-wide count
    // is read relatively so this assertion also holds against a backend
    // carrying unrelated live families.
    let active_after = repo
        .count_active_sessions()
        .await
        .expect("active count after revocation");
    assert_eq!(
        active_after,
        active_before - 1,
        "revocation must remove exactly the family's live generation from the active set"
    );

    assert_eq!(
        repo.resolve_refresh_token(family_a.gen1.refresh_token_hash.expose())
            .await
            .expect("resolve family A's live generation"),
        RefreshResolution::Unknown,
        "the revoked family's live generation must read Unknown immediately (SR1)"
    );
    assert_eq!(
        repo.resolve_refresh_token(family_a.gen0.refresh_token_hash.expose())
            .await
            .expect("resolve family A's retired generation"),
        RefreshResolution::Unknown,
        "the revoked family's retirement record must be gone, not Retired"
    );

    // The sibling family is untouched by family A's revocation.
    assert_eq!(
        repo.resolve_refresh_token(family_b.gen0.refresh_token_hash.expose())
            .await
            .expect("resolve sibling"),
        RefreshResolution::Live(family_b.gen0.clone()),
        "a sibling family must stay live through another family's revocation"
    );

    // The count is honest: nothing left to remove, nothing reported.
    assert_eq!(
        repo.revoke_family(&family_a.family_id)
            .await
            .expect("second revoke of family A"),
        0,
        "revoking an already-empty family must report zero removals"
    );
    assert_eq!(
        repo.revoke_family(&fixture_family_id(&format!("{tag}:family:never-created")))
            .await
            .expect("revoke of an unknown family"),
        0,
        "revoking an unknown well-formed family id must report zero, not error"
    );
}

/// **SR1 (negative space).** `resolve_refresh_token` answers from the store's
/// most recent state: immediately after `revoke_session` deletes the live
/// generation, that hash resolves `Unknown` — never `Live`, and never an
/// error. The family's retirement record for the *previous* generation
/// survives the single-session revocation (by contract `revoke_session`
/// touches only the live row), and now correctly classifies `Retired`
/// because its successor pointer no longer names a live generation.
pub async fn assert_resolution_unknown_immediately_after_revoke<S: SessionRepository + ?Sized>(
    repo: &S,
    tag: &str,
) {
    let chain = family_chain(tag, 0, "usr_conformance");
    repo.store_refresh_token(&chain.gen0)
        .await
        .expect("store generation 0");
    assert!(
        repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
            .await
            .expect("rotation"),
        "the rotation must win"
    );

    repo.revoke_session(&chain.gen1.refresh_token_hash)
        .await
        .expect("revoke the live generation");

    assert_eq!(
        repo.resolve_refresh_token(chain.gen1.refresh_token_hash.expose())
            .await
            .expect("resolve revoked hash immediately"),
        RefreshResolution::Unknown,
        "a just-revoked hash must resolve Unknown immediately — an eventually consistent answer would keep it Live (SR1)"
    );
    assert_eq!(
        repo.get_session_by_refresh_token(&chain.gen1.refresh_token_hash)
            .await
            .expect("liveness lookup for revoked hash"),
        None,
        "the live-session lookup must agree with the classification"
    );
    // The retained record for the predecessor survives revoke_session and
    // reclassifies: its successor is no longer live.
    match repo
        .resolve_refresh_token(chain.gen0.refresh_token_hash.expose())
        .await
        .expect("resolve predecessor after successor revocation")
    {
        RefreshResolution::Retired { family_id, user_id, .. } => {
            assert_eq!(family_id, chain.family_id);
            assert_eq!(user_id, chain.user_id);
        }
        other => panic!(
            "the predecessor's record must survive revoke_session and classify Retired, got {other:?}"
        ),
    }
}

/// **Expiry inheritance.** The replacement inherits the family's absolute
/// `expires_at` and `created_at` unchanged: rotation advances the
/// generation, never the deadline. Recomputing either would convert a
/// bounded family into an unbounded one that never dies while used.
pub async fn assert_rotation_preserves_absolute_expiry<S: SessionRepository + ?Sized>(
    repo: &S,
    tag: &str,
) {
    let chain = family_chain(tag, 0, "usr_conformance");
    repo.store_refresh_token(&chain.gen0)
        .await
        .expect("store generation 0");

    assert!(
        repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
            .await
            .expect("rotation"),
        "the rotation must win"
    );

    let replacement = repo
        .get_session_by_refresh_token(&chain.gen1.refresh_token_hash)
        .await
        .expect("fetch the installed replacement")
        .expect("the replacement must be installed as a live session");
    assert_eq!(
        replacement.expires_at, chain.gen0.expires_at,
        "rotation must not move the family's absolute expiry"
    );
    assert_eq!(
        replacement.created_at, chain.gen0.created_at,
        "rotation must not move the family's creation instant"
    );
    assert_eq!(
        replacement.generation, chain.gen1.generation,
        "the replacement must carry its own generation number"
    );
    assert_eq!(
        replacement.rotated_at, chain.gen1.rotated_at,
        "the replacement must carry its own issue instant in rotated_at"
    );
    assert_eq!(
        replacement.family_id, chain.family_id,
        "rotation must not move the family identity"
    );
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Run the entire SR1–SR5 suite against one store. Every assertion builds its
/// fixture families from `tag`, so the whole suite is safe against a shared
/// backend and safe to re-run. Adapters invoke this once their implementation
/// is complete; interim adapters invoke the individual assertions they
/// already satisfy — each function stands alone and weakens nothing.
///
/// The first failing assertion panics with its obligation-specific message,
/// which is what fails the caller's `#[tokio::test]`.
pub async fn assert_full_conformance<S: SessionRepository + Send + Sync + ?Sized>(
    repo: &S,
    tag: &str,
) {
    assert_resolution_classifies_all_four_shapes(repo, &format!("{tag}:classify")).await;
    assert_rotation_installs_successor_and_demotes_presented(repo, &format!("{tag}:install")).await;
    assert_failed_cas_leaves_store_byte_identical(repo, &format!("{tag}:cas")).await;
    assert_concurrent_rotation_yields_exactly_one_winner(repo, &format!("{tag}:race")).await;
    assert_retirement_readable_immediately_after_rotation(repo, &format!("{tag}:sr4")).await;
    assert_older_generation_resolves_as_retired(repo, &format!("{tag}:older")).await;
    assert_family_revocation_removes_everything_and_returns_count(repo, &format!("{tag}:revoke"))
        .await;
    assert_resolution_unknown_immediately_after_revoke(repo, &format!("{tag}:unknown")).await;
    assert_rotation_preserves_absolute_expiry(repo, &format!("{tag}:expiry")).await;
}
