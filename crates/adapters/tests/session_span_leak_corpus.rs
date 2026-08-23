//! Cross-store session-span leak corpus (plan task 07).
//!
//! Drives every [`SessionRepository`] backend through its store → lookup → revoke →
//! revoke-all lifecycle under the shared capturing subscriber (`FmtSpan::NEW | CLOSE`,
//! so span teardown is actually rendered and an absence claim can never pass
//! vacuously), then asserts that neither the event stream nor the close-span output
//! contains the backend's secret/hash/provenance sentinels — matched after
//! percent-decoding as well as literally.
//!
//! Backend coverage and gating mirror the repository's existing conventions:
//!
//! - **LMDB** and **SQLite** run everywhere (embedded stores).
//! - **Valkey** (`docker run -p 6379:6379 valkey/valkey:8-alpine`, or `VALKEY_TEST_URL`),
//!   **Postgres** (`POSTGRES_TEST_URL` / local docker), and **DynamoDB Local**
//!   (`docker run -p 8000:8000 amazon/dynamodb-local`) are `#[ignore]`d like their
//!   sibling integration suites; run them with `cargo nextest run --run-ignored only`.
//! - **MockRepository** carries no `#[instrument]` today; driving it under the same
//!   capture is a tripwire: if anyone later adds unskipped instrumentation to the
//!   shared mock, this corpus fails instead of silently publishing sentinels.
//!
//! Each backend uses *distinct* sentinel values, so a failure names the leaking store.
//! Permitted fields stay asserted too: the write span still records `user_id`, and the
//! lookup/revoke spans keep their declared-but-valueless `token_hash` schema field.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Duration, Utc};

use oidc_exchange_adapters::lmdb::LmdbSessionRepository;
use oidc_exchange_core::domain::{NewUser, Session};
use oidc_exchange_core::ports::{SessionRepository, UserRepository};
use oidc_exchange_core::Secret;
use oidc_exchange_test_utils::telemetry::{
    assert_absent_plain_and_encoded, assert_declares, install_span_capture, SharedBuffer,
};
use oidc_exchange_test_utils::MockRepository;

/// The three instrumented lifecycle spans every session adapter shares. Non-vacuousness
/// of every absence claim below hangs on these having both opened AND closed inside the
/// capture.
const LIFECYCLE_SPANS: [&str; 3] = [
    "store_refresh_token",
    "get_session_by_refresh_token",
    "revoke_session",
];

/// One backend's sentinel set. Values are obviously fake but shaped like the real
/// thing (64-hex digest, documentation-range IP) so the rendering paths under test are
/// the ones production traffic exercises. The `user_id` is threaded separately: the
/// Postgres variant must drive a real JIT-provisioned user row to satisfy the
/// sessions table's foreign key, so its id is created at runtime.
struct Sentinels {
    hash: &'static str,
    device: &'static str,
    user_agent: &'static str,
    ip: &'static str,
}

fn sentinel_session(s: &Sentinels, user_id: &str) -> Session {
    let now = Utc::now();
    Session {
        user_id: user_id.to_string(),
        refresh_token_hash: Secret::new(s.hash.to_string()),
        provider: "google".to_string(),
        expires_at: now + Duration::hours(1),
        device_id: Some(s.device.to_string()),
        user_agent: Some(s.user_agent.to_string()),
        ip_address: Some(s.ip.to_string()),
        created_at: now,
    }
}

/// Drive the full session lifecycle against `repo`: two stored sessions (so revoke-all
/// has something to sweep), a lookup, a single revoke, then the user-wide revoke-all.
/// Returns nothing; every call must succeed or the test fails loudly.
async fn drive_lifecycle(repo: &dyn SessionRepository, s: &Sentinels, user_id: &str) {
    let first = sentinel_session(s, user_id);
    repo.store_refresh_token(&first)
        .await
        .expect("store_refresh_token");

    let fetched = repo
        .get_session_by_refresh_token(&first.refresh_token_hash)
        .await
        .expect("get_session_by_refresh_token")
        .expect("stored session must be retrievable");
    assert_eq!(fetched.user_id, user_id, "lookup must find the stored row");

    repo.revoke_session(&first.refresh_token_hash)
        .await
        .expect("revoke_session");

    let second = sentinel_session(s, user_id);
    repo.store_refresh_token(&second)
        .await
        .expect("store second session for revoke-all");
    repo.revoke_all_user_sessions(user_id)
        .await
        .expect("revoke_all_user_sessions");

    let gone = repo
        .get_session_by_refresh_token(&second.refresh_token_hash)
        .await
        .expect("post-revoke-all lookup");
    assert!(gone.is_none(), "revoke-all must have removed the session");
}

/// Shared assertion set: the lifecycle spans opened and closed inside this capture,
/// permitted fields survived, and no sentinel reached any rendered fragment — plain or
/// percent-decoded.
fn assert_no_leak(
    rendered: &str,
    declared: &oidc_exchange_test_utils::telemetry::DeclaredFields,
    s: &Sentinels,
    user_id: &str,
) {
    for span_name in LIFECYCLE_SPANS {
        let mentions = rendered.matches(span_name).count();
        assert!(
            mentions >= 2,
            "span {span_name} must appear at both open and close for the absence claims \
             below to be non-vacuous, found {mentions}"
        );
    }

    // Permitted observability survives alongside redaction.
    assert!(
        rendered.contains(&format!("user_id={user_id}")),
        "the write span must still record user_id"
    );
    assert_declares(declared, "store_refresh_token", "user_id");
    assert_declares(declared, "get_session_by_refresh_token", "token_hash");
    assert_declares(declared, "revoke_session", "token_hash");

    // Negative space: hash and provenance never render, raw or percent-encoded.
    assert_absent_plain_and_encoded(rendered, s.hash);
    for provenance in [s.device, s.user_agent, s.ip] {
        assert_absent_plain_and_encoded(rendered, provenance);
    }
}

// ---------------------------------------------------------------------------
// LMDB — runs everywhere
// ---------------------------------------------------------------------------

const LMDB: Sentinels = Sentinels {
    hash: "cafebabedeadbeef0123456789abcdefcafebabedeadbeef0123456789abcdef",
    device: "corpus-lmdb-device",
    user_agent: "corpus-lmdb-agent/1.0",
    ip: "192.0.2.10",
};

#[tokio::test]
async fn lmdb_session_spans_never_render_sentinels() {
    let capture = install_span_capture(SharedBuffer::default());
    let dir = tempfile::TempDir::new().expect("temp dir for lmdb environment");
    let path = dir.path().join("data");
    let repo = LmdbSessionRepository::new(path.to_str().expect("utf-8 temp path"), 16)
        .expect("open lmdb environment");

    drive_lifecycle(&repo, &LMDB, "usr_corpus_lmdb").await;

    let declared = capture.declared();
    let rendered = capture.rendered();
    assert_no_leak(&rendered, &declared, &LMDB, "usr_corpus_lmdb");
}

// ---------------------------------------------------------------------------
// SQLite — runs everywhere
// ---------------------------------------------------------------------------

const SQLITE: Sentinels = Sentinels {
    hash: "abcdef0123456789cafebabedeadbeefabcdef0123456789cafebabedeadbeef",
    device: "corpus-sqlite-device",
    user_agent: "corpus-sqlite-agent/1.0",
    ip: "192.0.2.11",
};

#[tokio::test]
async fn sqlite_session_spans_never_render_sentinels() {
    let capture = install_span_capture(SharedBuffer::default());
    let dir = tempfile::TempDir::new().expect("temp dir for sqlite database");
    let db_path = dir.path().join("corpus.db");
    let pool = oidc_exchange_adapters::sqlite::create_pool(db_path.to_str().expect("utf-8 path"))
        .await
        .expect("open migrated sqlite pool");
    let repo = oidc_exchange_adapters::sqlite::SqliteRepository::new(pool);

    drive_lifecycle(&repo, &SQLITE, "usr_corpus_sqlite").await;

    let declared = capture.declared();
    let rendered = capture.rendered();
    assert_no_leak(&rendered, &declared, &SQLITE, "usr_corpus_sqlite");
}

// ---------------------------------------------------------------------------
// MockRepository — tripwire for future unskipped instrumentation
// ---------------------------------------------------------------------------

const MOCK: Sentinels = Sentinels {
    hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    device: "corpus-mock-device",
    user_agent: "corpus-mock-agent/1.0",
    ip: "192.0.2.12",
};

#[tokio::test]
async fn mock_repository_emits_no_sentinels_if_instrumented_later() {
    let capture = install_span_capture(SharedBuffer::default());
    let repo = MockRepository::new();

    // Positive control: something DID render under this subscriber, so the absence
    // claims below are made against a live capture rather than an inert one.
    tracing::info!(target: "oidc_exchange_corpus", "corpus-marker: driving mock session lifecycle");
    drive_lifecycle(&repo, &MOCK, "usr_corpus_mock").await;

    let rendered = capture.rendered();
    assert!(
        rendered.contains("corpus-marker"),
        "the capture must be live for the tripwire to mean anything"
    );

    assert_absent_plain_and_encoded(&rendered, MOCK.hash);
    for provenance in [MOCK.device, MOCK.user_agent, MOCK.ip] {
        assert_absent_plain_and_encoded(&rendered, provenance);
    }
}

// ---------------------------------------------------------------------------
// Valkey — requires a local server
// ---------------------------------------------------------------------------

const VALKEY: Sentinels = Sentinels {
    hash: "feedface01234567899876543210abcdefeedface01234567899876543210abcd",
    device: "corpus-valkey-device",
    user_agent: "corpus-valkey-agent/1.0",
    ip: "198.51.100.21",
};

#[tokio::test]
#[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
async fn valkey_session_spans_never_render_sentinels() {
    let url =
        std::env::var("VALKEY_TEST_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());
    // Unique prefix per run so concurrent/successive runs never collide and self-clean.
    let prefix = format!(
        "leak-corpus:{}:{}:",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos(),
    );
    let repo = oidc_exchange_adapters::valkey::ValkeySessionRepository::new(&url, prefix)
        .await
        .expect("connect to local Valkey (VALKEY_TEST_URL or redis://localhost:6379)");

    let capture = install_span_capture(SharedBuffer::default());
    drive_lifecycle(&repo, &VALKEY, "usr_corpus_valkey").await;

    let declared = capture.declared();
    let rendered = capture.rendered();
    assert_no_leak(&rendered, &declared, &VALKEY, "usr_corpus_valkey");
}

// ---------------------------------------------------------------------------
// Postgres — requires a live database
// ---------------------------------------------------------------------------

const POSTGRES: Sentinels = Sentinels {
    hash: "9876543210abcdef9876543210abcdef9876543210abcdef9876543210abcdef",
    device: "corpus-postgres-device",
    user_agent: "corpus-postgres-agent/1.0",
    ip: "198.51.100.22",
};

#[tokio::test]
#[ignore] // Requires a live Postgres: POSTGRES_TEST_URL, e.g.
          // docker run -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:16
async fn postgres_session_spans_never_render_sentinels() {
    let url = std::env::var("POSTGRES_TEST_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postgres".to_string());
    // Migrations are idempotent (`CREATE TABLE IF NOT EXISTS`), so unlike the adapter's
    // own reset-based fixtures this corpus needs no advisory lock: it never drops
    // anything, only creates what is missing.
    let pool = oidc_exchange_adapters::postgres::create_pool(&url, 5, true)
        .await
        .expect("connect to live Postgres and ensure schema");
    let repo = oidc_exchange_adapters::postgres::PostgresRepository::new(pool);

    // `sessions.user_id` carries a foreign key to `users`, so the lifecycle must run
    // against a real user row — the same JIT-provisioning order production uses:
    // look up by `(external_id, provider)` first, create only when absent. The seed
    // key is fixed, so blind creation here would panic with a duplicate-key conflict
    // on any second run against the same persistent database.
    const SEED_EXTERNAL_ID: &str = "corpus-postgres-sub";
    const SEED_PROVIDER: &str = "corpus-postgres";
    let created = match repo
        .get_user_by_external_id(SEED_EXTERNAL_ID, SEED_PROVIDER)
        .await
        .expect("look up seed user by (external_id, provider)")
    {
        Some(existing) => existing,
        None => repo
            .create_user(&NewUser {
                external_id: SEED_EXTERNAL_ID.to_string(),
                provider: SEED_PROVIDER.to_string(),
                email: Some("corpus-postgres@example.com".to_string()),
                display_name: None,
            })
            .await
            .expect("seed user row for the sessions foreign key"),
    };

    let capture = install_span_capture(SharedBuffer::default());
    drive_lifecycle(&repo, &POSTGRES, &created.id).await;

    let declared = capture.declared();
    let rendered = capture.rendered();
    assert_no_leak(&rendered, &declared, &POSTGRES, &created.id);
}

// ---------------------------------------------------------------------------
// DynamoDB Local — requires a live local endpoint
// ---------------------------------------------------------------------------

const DYNAMO: Sentinels = Sentinels {
    hash: "555544443333222211110000aaaabbbb555544443333222211110000aaaabbbb",
    device: "corpus-dynamo-device",
    user_agent: "corpus-dynamo-agent/1.0",
    ip: "203.0.113.31",
};

#[tokio::test]
#[ignore] // Requires DynamoDB Local: docker run -p 8000:8000 amazon/dynamodb-local
async fn dynamodb_session_spans_never_render_sentinels() {
    use aws_sdk_dynamodb::types::{
        AttributeDefinition, GlobalSecondaryIndex, KeySchemaElement, KeyType, Projection,
        ProjectionType, ProvisionedThroughput, ScalarAttributeType,
    };

    const GSI1_NAME: &str = "GSI1";

    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .endpoint_url("http://localhost:8000")
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(aws_sdk_dynamodb::config::Credentials::new(
            "fakeAccessKey",
            "fakeSecretKey",
            None,
            None,
            "test",
        ))
        .load()
        .await;
    let client = aws_sdk_dynamodb::Client::new(&config);

    // Same shape as the adapter's own DynamoDB Local fixtures: pk/sk primary key plus
    // the GSI1 user-index projection the repository queries.
    let _ = client.delete_table().table_name("leak-corpus").send().await;
    client
        .create_table()
        .table_name("leak-corpus")
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("pk")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("pk attribute definition"),
        )
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("sk")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("sk attribute definition"),
        )
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("GSI1pk")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("GSI1pk attribute definition"),
        )
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("GSI1sk")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("GSI1sk attribute definition"),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("pk")
                .key_type(KeyType::Hash)
                .build()
                .expect("pk key schema"),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("sk")
                .key_type(KeyType::Range)
                .build()
                .expect("sk key schema"),
        )
        .global_secondary_indexes(
            GlobalSecondaryIndex::builder()
                .index_name(GSI1_NAME)
                .key_schema(
                    KeySchemaElement::builder()
                        .attribute_name("GSI1pk")
                        .key_type(KeyType::Hash)
                        .build()
                        .expect("GSI1pk key schema"),
                )
                .key_schema(
                    KeySchemaElement::builder()
                        .attribute_name("GSI1sk")
                        .key_type(KeyType::Range)
                        .build()
                        .expect("GSI1sk key schema"),
                )
                .projection(
                    Projection::builder()
                        .projection_type(ProjectionType::All)
                        .build(),
                )
                .provisioned_throughput(
                    ProvisionedThroughput::builder()
                        .read_capacity_units(5)
                        .write_capacity_units(5)
                        .build()
                        .expect("GSI throughput"),
                )
                .build()
                .expect("GSI definition"),
        )
        .provisioned_throughput(
            ProvisionedThroughput::builder()
                .read_capacity_units(5)
                .write_capacity_units(5)
                .build()
                .expect("table throughput"),
        )
        .send()
        .await
        .expect("create leak-corpus table");

    let repo =
        oidc_exchange_adapters::dynamo::DynamoRepository::new(client, "leak-corpus".to_string());

    let capture = install_span_capture(SharedBuffer::default());
    drive_lifecycle(&repo, &DYNAMO, "usr_corpus_dynamo").await;

    let declared = capture.declared();
    let rendered = capture.rendered();
    assert_no_leak(&rendered, &declared, &DYNAMO, "usr_corpus_dynamo");
}
