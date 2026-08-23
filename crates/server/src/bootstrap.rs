use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use config::{Config, Environment, File, FileFormat, Value, ValueKind};
use tower::Layer;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::timeout::TimeoutLayer;

use oidc_exchange_core::config::{AppConfig, ProviderConfig};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{
    AuditLog, IdentityProvider, KeyManager, SessionRepository, UserRepository, UserSync,
};
use oidc_exchange_core::service::AppService;

use crate::middleware::audit_context::audit_context_layer;
use crate::middleware::base_path::with_base_path_strip;
use crate::middleware::error_handler::panic_handler;
use crate::middleware::request_id::request_id_layer;
use crate::routes;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Config loading
// ---------------------------------------------------------------------------

/// Name of the config directory relative to the process working directory.
const CONFIG_DIR: &str = "config";

/// Environment variable that selects the environment-specific overlay TOML
/// (`config/{OIDC_EXCHANGE_ENV}.toml`), e.g. `production`, `sqlite-only`.
const ENV_SELECTOR_VAR: &str = "OIDC_EXCHANGE_ENV";

/// Prefix required on environment variables that structurally override the
/// merged config (`OIDC_EXCHANGE__{section}__{key}`).
const ENV_OVERRIDE_PREFIX: &str = "OIDC_EXCHANGE";

/// Separator between path segments in `OIDC_EXCHANGE__{section}__{key}`
/// environment overrides. A double underscore separates segments; a single
/// underscore stays inside a segment (`OIDC_EXCHANGE__PROVIDERS__MY_IDP__…`
/// addresses `providers.my_idp`).
const ENV_OVERRIDE_SEPARATOR: &str = "__";

/// Sane upper bound, in seconds, on `server.request_timeout`: a value above this is almost
/// certainly a misconfiguration (e.g. a stray extra digit, or a TTL-style string pasted into
/// the wrong key) rather than an intentionally very slow deployment, so
/// [`request_timeout_duration`] asserts the parsed value never exceeds it. One hour is far
/// beyond any sane per-request bound for this service's synchronous HTTP handlers.
const REQUEST_TIMEOUT_MAX_SECS: u64 = 60 * 60;

/// Upper bound, in bytes, on how far the placeholder resolver scans past a
/// `${` opener looking for its closing `}` before giving up and treating the
/// `${` as ordinary text. Environment variable names are short; this bounds
/// the scan so a stray `${` inside free-form config text (e.g. a URL query
/// string) can never force an unbounded scan.
const PLACEHOLDER_NAME_LEN_MAX: usize = 256;

/// Load configuration from config files on disk, using the `OIDC_EXCHANGE_ENV`
/// environment variable to select the environment-specific config file, and
/// `OIDC_EXCHANGE__{section}__{key}` environment variables to override the
/// merged result afterward.
pub fn load_config() -> Result<AppConfig, Box<dyn std::error::Error>> {
    load_config_from_dir(CONFIG_DIR)
}

/// Core of [`load_config`], parameterized over the config directory so tests
/// can point it at a fixture directory instead of the process's `config/`.
///
/// Loading order:
/// 1. `{config_dir}/default.toml` — compiled-in defaults (missing file is not
///    an error).
/// 2. `{config_dir}/{OIDC_EXCHANGE_ENV}.toml` — deep-merged on top when
///    `OIDC_EXCHANGE_ENV` is set and non-empty (tables merge recursively,
///    scalars/arrays are replaced; missing/empty file is not an error).
/// 3. `OIDC_EXCHANGE__{section}__{key}` environment variables, applied on top
///    of the merged TOML and reaching every path including map-valued
///    sections.
/// 4. `${VAR_NAME}` placeholders anywhere in the merged config are resolved
///    from the environment; an unset variable is a fail-closed `ConfigError`
///    naming it, and `$${` escapes to a literal `${` rather than opening a
///    placeholder (see [`resolve_placeholders`]).
///
/// After deserialization, [`AppConfig::validate`] runs over the fully
/// merged, fully resolved config (role, TTLs, allowlist, internal API
/// secret) and a failure aborts before any adapter or router is built.
///
/// With no files present and no overriding environment variables, the
/// deserialized result is `AppConfig::default()` (every field carries
/// `#[serde(default)]`).
fn load_config_from_dir(config_dir: &str) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let mut builder = Config::builder().add_source(
        File::with_name(&format!("{config_dir}/default"))
            .format(FileFormat::Toml)
            .required(false),
    );

    if let Ok(env) = std::env::var(ENV_SELECTOR_VAR) {
        if !env.is_empty() {
            builder = builder.add_source(
                File::with_name(&format!("{config_dir}/{env}"))
                    .format(FileFormat::Toml)
                    .required(false),
            );
        }
    }

    builder = builder.add_source(
        Environment::with_prefix(ENV_OVERRIDE_PREFIX)
            .separator(ENV_OVERRIDE_SEPARATOR)
            .try_parsing(true),
    );

    let mut merged = builder.build()?;
    resolve_placeholders(&mut merged.cache)?;
    let mut config: AppConfig = merged.try_deserialize()?;
    // Normalise before validating so validation always sees the canonical forms
    // (e.g. a trailing-slash `base_path` is trimmed here, not rejected).
    config.normalise();
    config.validate()?;
    Ok(config)
}

/// Parse a TOML string directly into an `AppConfig`, validating it exactly as
/// [`load_config`] does so that config supplied through the FFI bindings
/// (`OidcExchange::new`/`from_file`) is rejected at construction on the same
/// terms as an invalid config on disk would be rejected at server startup.
pub fn parse_config(toml_str: &str) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let mut config: AppConfig = toml::from_str(toml_str)?;
    // Same normalise-then-validate order as `load_config_from_dir`, so config supplied
    // through the FFI bindings is canonicalised on identical terms.
    config.normalise();
    config.validate()?;
    Ok(config)
}

// ---------------------------------------------------------------------------
// Placeholder resolution
// ---------------------------------------------------------------------------

/// Recursively walk every string value in a merged `config::Value` tree,
/// resolving `${VAR}` placeholders from the environment in place.
///
/// Fails closed: a placeholder naming an unset environment variable aborts
/// the whole resolution with `Error::ConfigError` — the literal placeholder
/// text must never survive into a live secret. `$${` is the escape for a
/// literal `${`; the escaped text is never looked up in the environment.
fn resolve_placeholders(value: &mut Value) -> Result<(), Error> {
    match &mut value.kind {
        ValueKind::String(s) => {
            *s = resolve_placeholders_in_str(s)?;
        }
        ValueKind::Table(table) => {
            for nested in table.values_mut() {
                resolve_placeholders(nested)?;
            }
        }
        ValueKind::Array(items) => {
            for item in items.iter_mut() {
                resolve_placeholders(item)?;
            }
        }
        // Non-string scalars (bool, numbers, nil) carry no placeholders.
        _ => {}
    }
    Ok(())
}

/// Resolve every `${VAR}` placeholder and `$${` escape inside a single
/// string, returning the rewritten string.
fn resolve_placeholders_in_str(input: &str) -> Result<String, Error> {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut i = 0;

    // Bounded by `bytes.len()`, not recursive: each iteration consumes at
    // least one byte (asserted below), so the loop always terminates.
    while i < bytes.len() {
        let before = i;

        // Escape: `$${` rewrites to a literal `${` and is never treated as a
        // placeholder opener, even for the `{` it consumes.
        if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'$') && bytes.get(i + 2) == Some(&b'{') {
            output.push_str("${");
            i += 3;
            debug_assert!(i > before, "escape branch must consume input");
            continue;
        }

        // Placeholder open: `${NAME}`.
        if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{') {
            if let Some((name, consumed)) = scan_placeholder_name(&input[i + 2..]) {
                let resolved = std::env::var(name).map_err(|_| Error::ConfigError {
                    detail: format!(
                        "config placeholder '${{{name}}}' references unset environment \
                         variable '{name}'"
                    ),
                })?;
                output.push_str(&resolved);
                i += 2 + consumed;
                debug_assert!(i > before, "placeholder branch must consume input");
                continue;
            }
        }

        // Ordinary text: copy one full UTF-8 scalar value forward. `i` is
        // guaranteed to sit on a char boundary here because every branch
        // above only ever advances `i` past whole ASCII marker sequences.
        let ch = input[i..]
            .chars()
            .next()
            .expect("non-empty remainder has a leading char");
        output.push(ch);
        i += ch.len_utf8();
        debug_assert!(i > before, "ordinary-text branch must consume input");
    }

    assert!(
        i == bytes.len(),
        "resolver must consume the whole input, not overrun or stop short"
    );
    Ok(output)
}

/// Scan forward from just past a `${` opener for its closing `}`, bounded by
/// [`PLACEHOLDER_NAME_LEN_MAX`]. Returns the placeholder name and the number
/// of bytes consumed (name plus the closing brace), or `None` when no `}` is
/// found within the bound — in which case the `${` is left as ordinary text
/// rather than treated as a malformed placeholder.
fn scan_placeholder_name(rest: &str) -> Option<(&str, usize)> {
    let bytes = rest.as_bytes();
    let scan_bound = bytes.len().min(PLACEHOLDER_NAME_LEN_MAX);
    let close = bytes[..scan_bound].iter().position(|&b| b == b'}')?;
    let consumed = close + 1;
    debug_assert!(
        consumed <= PLACEHOLDER_NAME_LEN_MAX + 1,
        "consumed byte count must stay within the declared scan bound"
    );
    debug_assert!(
        &rest[close..consumed] == "}",
        "must stop exactly on the closing brace"
    );
    Some((&rest[..close], consumed))
}

// ---------------------------------------------------------------------------
// Service builder
// ---------------------------------------------------------------------------

/// Build the full `AppService` from a loaded config, instantiating all
/// adapters (repositories, key manager, audit log, user sync, providers)
/// according to the configured role.
pub async fn build_service(config: &AppConfig) -> Result<AppService, Box<dyn std::error::Error>> {
    let role = config.server.role.as_str();

    // Build adapters (skip unused ones based on role)
    let user_repo = build_user_repository(config).await?;
    let session_repo = build_session_repository(config).await?;

    // Key manager and providers only needed for exchange role
    let keys: Box<dyn KeyManager> = if role == "admin" {
        Box::new(oidc_exchange_adapters::noop::NoopKeyManager)
    } else {
        build_key_manager(config)?
    };

    let audit = build_audit_log(config).await?;

    // User sync only needed for admin role
    let user_sync: Box<dyn UserSync> = if role == "exchange" {
        Box::new(oidc_exchange_adapters::noop::NoopUserSync::new())
    } else {
        build_user_sync(config)?
    };

    // Providers only needed for exchange role
    let providers = if role == "admin" {
        HashMap::new()
    } else {
        build_providers(config).await?
    };

    Ok(AppService::new(
        user_repo,
        session_repo,
        keys,
        audit,
        user_sync,
        providers,
        config.clone(),
    ))
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

/// Build the Axum `Router` from a config and service, applying role-based
/// route merging and middleware layers.
///
/// The internal routes (`/internal/*`) are mounted only when the role is
/// `admin`/`all` **and** `internal_api.enabled = true`; the flag being false
/// is not a startup error, it simply leaves the internal surface unmounted
/// (`AppConfig::validate` already requires a non-empty shared secret whenever
/// the flag is true and the role would serve it, so a missing/empty secret
/// is caught at startup, never discovered by an unauthenticated request).
///
/// Middleware stack, outermost first (`04-http-api.md` → Middleware stack): base-path strip,
/// request-id, request-timeout, audit-context, catch-panic. Axum/tower give the *last*
/// `.layer()` call the outermost position (it wraps every layer added before it as its
/// `next`), so the code below applies the per-route layers in the reverse of that list —
/// catch-panic first (innermost, nearest the handler), then audit-context, then the timeout
/// layer, then request-id last (outermost among them). This ordering is what makes a
/// request-timeout response still carry the `x-request-id` header: because request-id wraps
/// the timeout layer, its header-insertion code always runs on whatever `next.run()`
/// produces, including the timeout layer's own manufactured `408` when the inner future is
/// abandoned — whereas a layer *outside* request-id would have its future abandoned right
/// along with the rest and never see the response at all. The timeout layer itself wraps
/// audit-context and catch-panic so the bound covers them and the handler, not just the
/// handler.
///
/// The base-path strip is applied *last*, wrapping the entire already-assembled, already-
/// stated router from the outside via [`crate::middleware::base_path::with_base_path_strip`]
/// — not as one more `.layer()` call. `Router::layer` only wraps each already-registered
/// route's endpoint, which runs *after* axum has already decided which route (if any) matches
/// the request's current path; a layer added that way can never influence *which* route is
/// chosen, so it cannot be used to strip a prefix that needs to affect the routing decision
/// itself (`/prod/health` → `/health`). Wrapping the whole router from the outside is the one
/// way to rewrite the path early enough. This is applied unconditionally on this one shared
/// path used by both the hyper and Lambda runtimes — when `config.server.base_path` is `None`
/// the wrapper still runs on every request but is a pure pass-through, so there is no separate
/// Lambda-only branch that installs it only sometimes.
///
/// Finally a **second, outer** [`CatchPanicLayer`] wraps the base-path-aware service, giving
/// the stack two guards (`04-http-api.md` → Middleware stack): the inner one nearest the
/// handlers keeps a caught *handler* panic's response inside the request-id layer so it still
/// carries `x-request-id`, while the outer one contains panics raised in the request-id,
/// timeout, audit-context, or base-path layers themselves — turning what used to be a killed
/// connection into the standard structured `500`. The base-path layer is total over
/// host-supplied paths (its URI reconstruction degrades to pass-through rather than
/// panicking), so nothing is expected to reach the outer guard; it exists so a defect in any
/// of those layers degrades to a clean error response instead of taking the worker down.
pub fn build_router(config: &AppConfig, service: AppService) -> Router {
    let role = config.server.role.as_str();

    let state = AppState {
        service: Arc::new(service),
        config: Arc::new(config.clone()),
    };

    let mut app: Router<AppState> = Router::new();

    if role == "exchange" || role == "all" {
        app = app.merge(routes::public_routes());
    }
    if (role == "admin" || role == "all") && config.internal_api.enabled {
        app = app.merge(routes::internal_routes(state.clone()));
    }
    if role == "admin" {
        // Ensure /health is available even in admin-only mode, whether or
        // not the internal API is enabled — "admin" never merges
        // `public_routes`, which is the only other source of `/health`.
        app = app.route(
            "/health",
            axum::routing::get(routes::health::health_handler),
        );
    }

    #[cfg(feature = "conformance")]
    let app = app.fallback(conformance_observe);

    let router = apply_route_layers(
        app,
        request_timeout_duration(config),
        config.server.max_request_body_bytes,
    )
    .with_state(state);

    wrap_with_base_path_under_outer_guard(router, config.server.base_path.clone())
}

/// Apply the per-route middleware stack every router this crate builds shares — inner
/// catch-panic nearest the handler, then audit-context, then the request-timeout layer, then
/// request-id outermost among them (ordering rationale in [`build_router`]'s doc comment).
///
/// Generic over the router's state type and parameterised on the resolved timeout so the
/// panic-containment tests can compose this exact function over test routers — the tested
/// layer order cannot drift from the shipped one because they are the same code. The
/// `Clone + Send + Sync + 'static` bounds are what `Router::layer` demands of the state; both
/// callers (`Router<AppState>` in production, bare `Router<()>` in tests) satisfy them.
fn apply_route_layers<S>(
    app: Router<S>,
    request_timeout: std::time::Duration,
    max_request_body_bytes: usize,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    app.layer(axum::extract::DefaultBodyLimit::max(max_request_body_bytes))
        .layer(CatchPanicLayer::custom(panic_handler))
        .layer(axum::middleware::from_fn(audit_context_layer))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            request_timeout,
        ))
        .layer(axum::middleware::from_fn(request_id_layer))
}

/// Wrap an assembled router with the outer half of the production stack — the base-path
/// strip first, then the outer catch-panic guard around it (`04-http-api.md` → Middleware
/// stack, entries 1–2; rationale in [`build_router`]'s doc comment).
///
/// Composed through its own routeless router — the same shape `with_base_path_strip` uses —
/// because `Router::layer` cannot wrap a router as an opaque unit, and the guard must sit
/// outside the base-path rewrite, not merely around each already-matched endpoint. Shared
/// with the panic-containment tests for the same drift-proofing reason as
/// [`apply_route_layers`].
fn wrap_with_base_path_under_outer_guard(router: Router, base_path: Option<String>) -> Router {
    let base_path_aware = with_base_path_strip(router, base_path);
    Router::new().fallback_service(CatchPanicLayer::custom(panic_handler).layer(base_path_aware))
}

/// Parse `server.request_timeout` into the `Duration` the request-timeout layer is built
/// from — entry 2 of the outermost-first middleware ordering (inside the request-id layer,
/// so a timeout response still carries the request id; outside audit-context and
/// catch-panic, so the bound covers the remaining middleware and the handler).
///
/// Every production entry point (`load_config`, `parse_config`) runs `AppConfig::validate`
/// — which parses this same field via [`oidc_exchange_core::service::parse_duration_secs`] —
/// before a router is ever built, so an unparseable value fails config loading closed rather
/// than reaching this function. Reaching it here anyway (e.g. a hand-built `AppConfig` in a
/// test that skipped `validate`) is treated as a programmer error and panics loudly instead
/// of silently substituting [`oidc_exchange_core::config::DEFAULT_REQUEST_TIMEOUT`].
#[cfg(feature = "conformance")]
async fn conformance_observe(request: axum::extract::Request) -> axum::response::Response {
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use serde_json::json;

    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, usize::MAX).await {
        Ok(body) => body,
        Err(_) => return axum::http::StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let ordered_headers = parts
        .headers
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "host" | "connection"))
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| json!({"name": name.as_str(), "value": value}))
        })
        .collect::<Vec<_>>();
    let request_id = parts
        .headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok());
    let routed_status = if matches!(parts.uri.path(), "/health" | "/keys") {
        200
    } else {
        404
    };
    let response = json!({
        "method": parts.method.as_str(),
        "decodedPath": parts.uri.path(),
        "query": parts.uri.query(),
        "orderedHeaders": ordered_headers,
        "requestId": request_id,
        "bodyLength": body.len(),
        "status": routed_status,
        "downstreamMarker": "observed-after-routing"
    });
    (
        axum::http::StatusCode::from_u16(routed_status).expect("valid routed status"),
        axum::Json(response),
    )
        .into_response()
}

fn request_timeout_duration(config: &AppConfig) -> std::time::Duration {
    let secs = oidc_exchange_core::service::parse_duration_secs(&config.server.request_timeout)
        .unwrap_or_else(|err| {
            panic!(
                "server.request_timeout {:?} is invalid: {err} (AppConfig::validate should \
                     have rejected this before build_router was ever called)",
                config.server.request_timeout
            )
        });
    assert!(
        secs > 0,
        "parsed request_timeout must be non-zero, got {secs}s from {:?}",
        config.server.request_timeout
    );
    assert!(
        secs <= REQUEST_TIMEOUT_MAX_SECS,
        "parsed request_timeout of {secs}s from {:?} exceeds the sane upper bound of {REQUEST_TIMEOUT_MAX_SECS}s",
        config.server.request_timeout
    );
    std::time::Duration::from_secs(secs)
}

// ---------------------------------------------------------------------------
// Adapter builders (private)
// ---------------------------------------------------------------------------

async fn build_dynamo_client(
    config: &AppConfig,
) -> Result<(aws_sdk_dynamodb::Client, String), Box<dyn std::error::Error>> {
    let dynamo_cfg = config
        .repository
        .dynamodb
        .as_ref()
        .ok_or_else(|| Error::ConfigError {
            detail: "repository.adapter is 'dynamodb' but [repository.dynamodb] section is missing"
                .into(),
        })?;

    let mut aws_loader = aws_config::defaults(aws_config::BehaviorVersion::latest());

    if let Some(ref region) = dynamo_cfg.region {
        aws_loader = aws_loader.region(aws_config::Region::new(region.clone()));
    }

    let sdk_config = aws_loader.load().await;
    let client = aws_sdk_dynamodb::Client::new(&sdk_config);
    Ok((client, dynamo_cfg.table_name.clone()))
}

async fn build_user_repository(
    config: &AppConfig,
) -> Result<Box<dyn UserRepository>, Box<dyn std::error::Error>> {
    match config.repository.adapter.as_str() {
        "dynamodb" => {
            let (client, table_name) = build_dynamo_client(config).await?;
            Ok(Box::new(
                oidc_exchange_adapters::dynamo::DynamoRepository::new(client, table_name),
            ))
        }
        "postgres" => {
            let pg_cfg = config.repository.postgres.as_ref().ok_or_else(|| {
                Error::ConfigError {
                    detail:
                        "repository.adapter is 'postgres' but [repository.postgres] section is missing"
                            .into(),
                }
            })?;
            let pool = oidc_exchange_adapters::postgres::create_pool(
                &pg_cfg.url,
                pg_cfg.max_connections.unwrap_or(5),
                pg_cfg.run_migrations.unwrap_or(true),
            )
            .await?;
            Ok(Box::new(
                oidc_exchange_adapters::postgres::PostgresRepository::new(pool),
            ))
        }
        "sqlite" => {
            let sq_cfg = config
                .repository
                .sqlite
                .as_ref()
                .ok_or_else(|| Error::ConfigError {
                    detail:
                        "repository.adapter is 'sqlite' but [repository.sqlite] section is missing"
                            .into(),
                })?;
            let pool = oidc_exchange_adapters::sqlite::create_pool(&sq_cfg.path).await?;
            Ok(Box::new(
                oidc_exchange_adapters::sqlite::SqliteRepository::new(pool),
            ))
        }
        "" => Err(Box::new(Error::ConfigError {
            detail: "repository.adapter is not configured".into(),
        })),
        other => Err(Box::new(Error::ConfigError {
            detail: format!("unknown repository adapter: {other}"),
        })),
    }
}

async fn build_session_repository(
    config: &AppConfig,
) -> Result<Box<dyn SessionRepository>, Box<dyn std::error::Error>> {
    // If a separate session_repository adapter is configured, use it.
    // Otherwise, fall back to the same adapter as the user repository.
    let adapter = config
        .session_repository
        .adapter
        .as_deref()
        .unwrap_or(config.repository.adapter.as_str());

    match adapter {
        "dynamodb" => {
            let (client, table_name) = build_dynamo_client(config).await?;
            Ok(Box::new(
                oidc_exchange_adapters::dynamo::DynamoRepository::new(client, table_name),
            ))
        }
        "postgres" => {
            let pg_cfg = config.repository.postgres.as_ref().ok_or_else(|| {
                Error::ConfigError {
                    detail:
                        "session_repository adapter is 'postgres' but [repository.postgres] section is missing"
                            .into(),
                }
            })?;
            let pool = oidc_exchange_adapters::postgres::create_pool(
                &pg_cfg.url,
                pg_cfg.max_connections.unwrap_or(5),
                pg_cfg.run_migrations.unwrap_or(true),
            )
            .await?;
            Ok(Box::new(
                oidc_exchange_adapters::postgres::PostgresRepository::new(pool),
            ))
        }
        "sqlite" => {
            let sq_cfg = config.repository.sqlite.as_ref().ok_or_else(|| {
                Error::ConfigError {
                    detail:
                        "session_repository adapter is 'sqlite' but [repository.sqlite] section is missing"
                            .into(),
                }
            })?;
            let pool = oidc_exchange_adapters::sqlite::create_pool(&sq_cfg.path).await?;
            Ok(Box::new(
                oidc_exchange_adapters::sqlite::SqliteRepository::new(pool),
            ))
        }
        "valkey" => {
            let vk_cfg = config.session_repository.valkey.as_ref().ok_or_else(|| {
                Error::ConfigError {
                    detail:
                        "session_repository adapter is 'valkey' but [session_repository.valkey] section is missing"
                            .into(),
                }
            })?;
            let client = oidc_exchange_adapters::valkey::ValkeySessionRepository::new(
                &vk_cfg.url,
                vk_cfg
                    .key_prefix
                    .clone()
                    .unwrap_or_else(|| "oidc:".to_string()),
            )
            .await?;
            Ok(Box::new(client))
        }
        "lmdb" => {
            let lm_cfg = config.session_repository.lmdb.as_ref().ok_or_else(|| {
                Error::ConfigError {
                    detail:
                        "session_repository adapter is 'lmdb' but [session_repository.lmdb] section is missing"
                            .into(),
                }
            })?;
            let repo = oidc_exchange_adapters::lmdb::LmdbSessionRepository::new(
                &lm_cfg.path,
                lm_cfg.max_size_mb.unwrap_or(256),
            )?;
            Ok(Box::new(repo))
        }
        "" => Err(Box::new(Error::ConfigError {
            detail: "repository.adapter is not configured".into(),
        })),
        other => Err(Box::new(Error::ConfigError {
            detail: format!("unknown session_repository adapter: {other}"),
        })),
    }
}

fn build_key_manager(
    config: &AppConfig,
) -> Result<Box<dyn KeyManager>, Box<dyn std::error::Error>> {
    match config.key_manager.adapter.as_str() {
        "local" => {
            let local_cfg =
                config
                    .key_manager
                    .local
                    .as_ref()
                    .ok_or_else(|| {
                        Error::ConfigError {
                    detail:
                        "key_manager.adapter is 'local' but [key_manager.local] section is missing"
                            .into(),
                }
                    })?;

            let mgr = oidc_exchange_adapters::local_keys::LocalKeyManager::from_file(
                &local_cfg.private_key_path,
                &local_cfg.algorithm,
                &local_cfg.kid,
            )?;
            Ok(Box::new(mgr))
        }
        "kms" => {
            let kms_cfg = config
                .key_manager
                .kms
                .as_ref()
                .ok_or_else(|| Error::ConfigError {
                    detail: "key_manager.adapter is 'kms' but [key_manager.kms] section is missing"
                        .into(),
                })?;

            // Build KMS client synchronously using a blocking load.
            let sdk_config = futures::executor::block_on(
                aws_config::defaults(aws_config::BehaviorVersion::latest()).load(),
            );
            let client = aws_sdk_kms::Client::new(&sdk_config);

            Ok(Box::new(oidc_exchange_adapters::kms::KmsKeyManager::new(
                client,
                kms_cfg.key_id.clone(),
                kms_cfg.algorithm.clone(),
                kms_cfg.kid.clone(),
            )))
        }
        "" => Err(Box::new(Error::ConfigError {
            detail: "key_manager.adapter is not configured".into(),
        })),
        other => Err(Box::new(Error::ConfigError {
            detail: format!("unknown key_manager adapter: {other}"),
        })),
    }
}

async fn build_audit_log(
    config: &AppConfig,
) -> Result<Box<dyn AuditLog>, Box<dyn std::error::Error>> {
    match config.audit.adapter.as_str() {
        "noop" | "" => Ok(Box::new(oidc_exchange_adapters::noop::NoopAuditLog::new())),
        "stdout" | "stderr" | "auto" => {
            use oidc_exchange_adapters::stdout_audit::{OutputTarget, StdoutAuditLog};
            let target = match config.audit.adapter.as_str() {
                "stdout" => OutputTarget::Stdout,
                "stderr" => OutputTarget::Stderr,
                _ => OutputTarget::Auto,
            };
            Ok(Box::new(StdoutAuditLog::new(target)))
        }
        "sqs" => {
            let sqs_cfg = config
                .audit
                .sqs
                .as_ref()
                .ok_or_else(|| Error::ConfigError {
                    detail: "audit.adapter is 'sqs' but [audit.sqs] section is missing".into(),
                })?;

            let mut aws_loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
            if let Some(ref region) = sqs_cfg.region {
                aws_loader = aws_loader.region(aws_config::Region::new(region.clone()));
            }
            let sdk_config = aws_loader.load().await;
            let client = aws_sdk_sqs::Client::new(&sdk_config);

            Ok(Box::new(
                oidc_exchange_adapters::sqs_audit::SqsAuditLog::new(
                    client,
                    sqs_cfg.queue_url.clone(),
                ),
            ))
        }
        other => Err(Box::new(Error::ConfigError {
            detail: format!("unknown audit adapter: {other}"),
        })),
    }
}

fn build_user_sync(config: &AppConfig) -> Result<Box<dyn UserSync>, Box<dyn std::error::Error>> {
    if !config.user_sync.enabled {
        return Ok(Box::new(oidc_exchange_adapters::noop::NoopUserSync::new()));
    }

    match config.user_sync.adapter.as_deref() {
        Some("webhook") => {
            let wh_cfg = config
                .user_sync
                .webhook
                .as_ref()
                .ok_or_else(|| Error::ConfigError {
                    detail:
                        "user_sync.adapter is 'webhook' but [user_sync.webhook] section is missing"
                            .into(),
                })?;

            let timeout_secs = wh_cfg
                .timeout
                .as_deref()
                .and_then(|s| {
                    let s = s.trim();
                    if let Some(stripped) = s.strip_suffix('s') {
                        stripped.parse::<u64>().ok()
                    } else {
                        s.parse::<u64>().ok()
                    }
                })
                .unwrap_or(5);
            let retries = wh_cfg.effective_retries();

            Ok(Box::new(
                oidc_exchange_adapters::webhook::WebhookUserSync::new(
                    wh_cfg.url.clone(),
                    wh_cfg.secret.clone(),
                    std::time::Duration::from_secs(timeout_secs),
                    retries,
                ),
            ))
        }
        Some(other) => Err(Box::new(Error::ConfigError {
            detail: format!("unknown user_sync adapter: {other}"),
        })),
        None => {
            // enabled=true but no adapter specified — default to noop
            Ok(Box::new(oidc_exchange_adapters::noop::NoopUserSync::new()))
        }
    }
}

async fn build_providers(
    config: &AppConfig,
) -> Result<HashMap<String, Box<dyn IdentityProvider>>, Box<dyn std::error::Error>> {
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();

    for (name, provider_cfg) in &config.providers {
        let provider = build_single_provider(name, provider_cfg).await?;
        providers.insert(name.clone(), provider);
    }

    Ok(providers)
}

async fn build_single_provider(
    name: &str,
    config: &ProviderConfig,
) -> Result<Box<dyn IdentityProvider>, Box<dyn std::error::Error>> {
    match config.adapter.as_str() {
        "oidc" => {
            let oidc_config = provider_config_to_oidc(name, config)?;
            let provider =
                oidc_exchange_adapters::oidc::OidcProvider::from_config(name, &oidc_config).await?;
            Ok(Box::new(provider))
        }
        "apple" => {
            let provider =
                oidc_exchange_providers::apple::AppleProvider::from_config(&config.extra).await?;
            Ok(Box::new(provider))
        }
        other => Err(Box::new(Error::ConfigError {
            detail: format!("unknown provider adapter for '{name}': {other}"),
        })),
    }
}

/// Convert the generic `ProviderConfig` (with its `extra` map) into the typed
/// `OidcProviderConfig` expected by the OIDC adapter.
fn provider_config_to_oidc(
    name: &str,
    config: &ProviderConfig,
) -> Result<oidc_exchange_core::domain::provider::OidcProviderConfig, Error> {
    use oidc_exchange_core::domain::provider::OidcProviderConfig;

    let get_str = |key: &str| -> Option<String> {
        config
            .extra
            .get(key)
            .and_then(|v| v.as_str())
            .map(String::from)
    };

    let issuer = get_str("issuer").ok_or_else(|| Error::ConfigError {
        detail: format!("provider '{name}': missing 'issuer'"),
    })?;

    let client_id = get_str("client_id").ok_or_else(|| Error::ConfigError {
        detail: format!("provider '{name}': missing 'client_id'"),
    })?;

    let scopes = config
        .extra
        .get("scopes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| vec!["openid".to_string()]);

    Ok(OidcProviderConfig {
        provider_id: name.to_string(),
        issuer,
        client_id,
        client_secret: get_str("client_secret"),
        jwks_uri: get_str("jwks_uri"),
        token_endpoint: get_str("token_endpoint"),
        revocation_endpoint: get_str("revocation_endpoint"),
        scopes,
        additional_params: HashMap::new(),
    })
}

// ---------------------------------------------------------------------------
// Config loading tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod load_config_tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard};

    // `std::env` is process-global. Keep every config-loading test in this
    // module exclusive so one test cannot observe another's partial override.
    static CONFIG_TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_test_environment() -> MutexGuard<'static, ()> {
        CONFIG_TEST_ENV_LOCK
            .lock()
            .expect("config test environment lock poisoned")
    }

    /// Restores every touched process-global variable to its prior state.
    /// Callers hold `lock_test_environment()` for the whole test.
    struct EnvVarGuard {
        snapshots: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl EnvVarGuard {
        fn set(vars: &[(&'static str, &str)]) -> Self {
            let snapshots = vars
                .iter()
                .map(|(key, _)| (*key, std::env::var_os(key)))
                .collect();
            for (key, value) in vars {
                std::env::set_var(key, value);
            }
            Self { snapshots }
        }

        fn remove(vars: &[&'static str]) -> Self {
            let snapshots = vars
                .iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect();
            for key in vars {
                std::env::remove_var(key);
            }
            Self { snapshots }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            for (key, value) in &self.snapshots {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn write_toml(dir: &Path, stem: &str, contents: &str) {
        fs::write(dir.join(format!("{stem}.toml")), contents).expect("write fixture toml");
    }

    fn dir_str(dir: &Path) -> &str {
        dir.to_str().expect("fixture dir path is valid UTF-8")
    }

    #[test]
    fn overlay_merges_over_default_rather_than_replacing_it() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [server]
                host = "0.0.0.0"
                port = 8080

                [registration]
                mode = "open"
            "#,
        );
        write_toml(
            dir.path(),
            "overlay-env",
            r#"
                [server]
                port = 9090
            "#,
        );
        let _guard = EnvVarGuard::set(&[("OIDC_EXCHANGE_ENV", "overlay-env")]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        // A key present only in default.toml survives the overlay...
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.registration.mode, "open");
        // ...while a key set in both takes the overlay's value.
        assert_eq!(config.server.port, 9090);
    }

    #[test]
    fn env_var_override_reaches_nested_and_map_valued_paths() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [server]
                port = 8080

                [providers.google]
                adapter = "oidc"
                client_id = "default-client"
            "#,
        );
        let _guard = EnvVarGuard::set(&[
            (
                "OIDC_EXCHANGE__PROVIDERS__GOOGLE__CLIENT_ID",
                "overridden-client",
            ),
            ("OIDC_EXCHANGE__SERVER__PORT", "9999"),
        ]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        assert_eq!(config.server.port, 9999);
        let google = config.providers.get("google").expect("google provider");
        assert_eq!(google.adapter, "oidc", "unrelated fields survive the merge");
        assert_eq!(
            google.extra.get("client_id").and_then(|v| v.as_str()),
            Some("overridden-client")
        );
    }

    #[test]
    fn single_underscore_provider_name_is_addressable_not_split() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [providers.my_idp]
                adapter = "oidc"
                client_id = "a"
            "#,
        );
        let _guard = EnvVarGuard::set(&[("OIDC_EXCHANGE__PROVIDERS__MY_IDP__CLIENT_ID", "b")]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        assert_eq!(
            config.providers.len(),
            1,
            "my_idp must address a single provider entry, not split into extra segments"
        );
        let provider = config
            .providers
            .get("my_idp")
            .expect("my_idp provider present under its full, unsplit name");
        assert_eq!(
            provider.extra.get("client_id").and_then(|v| v.as_str()),
            Some("b")
        );
    }

    #[test]
    fn missing_config_files_fall_back_to_compiled_in_defaults() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        // No files written at all, and no OIDC_EXCHANGE_ENV set.

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");
        let defaults = AppConfig::default();

        assert_eq!(config.server.host, defaults.server.host);
        assert_eq!(config.server.port, defaults.server.port);
        assert_eq!(config.registration.mode, defaults.registration.mode);
        assert_eq!(
            config.token.access_token_ttl,
            defaults.token.access_token_ttl
        );
    }

    #[test]
    fn missing_or_empty_env_overlay_file_is_not_an_error() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [server]
                host = "10.0.0.1"
            "#,
        );
        // Present but empty overlay file.
        write_toml(dir.path(), "empty-env", "");

        {
            let _guard = EnvVarGuard::set(&[("OIDC_EXCHANGE_ENV", "empty-env")]);
            let config = load_config_from_dir(dir_str(dir.path())).expect("load config");
            assert_eq!(config.server.host, "10.0.0.1");
        }

        // Overlay file that does not exist on disk at all.
        let _guard = EnvVarGuard::set(&[("OIDC_EXCHANGE_ENV", "does-not-exist")]);
        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");
        assert_eq!(config.server.host, "10.0.0.1");
    }

    /// End-to-end reviewability case: all three layers (default TOML, env
    /// overlay TOML, and an `OIDC_EXCHANGE__…` var) are exercised together,
    /// and the merged `AppConfig` carries the untouched default alongside
    /// the overlaid and env-overridden values.
    #[test]
    fn default_overlay_and_env_var_all_apply_together() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [server]
                host = "0.0.0.0"
                port = 8080

                [registration]
                mode = "open"
            "#,
        );
        write_toml(
            dir.path(),
            "full-stack",
            r#"
                [server]
                port = 9090
            "#,
        );
        let _guard = EnvVarGuard::set(&[
            ("OIDC_EXCHANGE_ENV", "full-stack"),
            ("OIDC_EXCHANGE__SERVER__HOST", "203.0.113.5"),
        ]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        // Untouched by either overlay or env var — the compiled default TOML value.
        assert_eq!(config.registration.mode, "open");
        // Set by the env overlay TOML, not present in the env var.
        assert_eq!(config.server.port, 9090);
        // Set by the OIDC_EXCHANGE__ env var, on top of the overlay and default.
        assert_eq!(config.server.host, "203.0.113.5");
    }

    // -----------------------------------------------------------------
    // Placeholder resolution
    // -----------------------------------------------------------------

    #[test]
    fn set_var_placeholder_resolves_to_its_environment_value() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [internal_api]
                shared_secret = "${INTERNAL_API_SECRET}"
            "#,
        );
        let _guard = EnvVarGuard::set(&[("INTERNAL_API_SECRET", "super-secret-value")]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        assert_eq!(
            config.internal_api.shared_secret.as_deref(),
            Some("super-secret-value")
        );
        assert_ne!(
            config.internal_api.shared_secret.as_deref(),
            Some("${INTERNAL_API_SECRET}"),
            "the literal placeholder must never survive resolution"
        );
    }

    #[test]
    fn unset_var_placeholder_fails_closed_and_produces_no_config() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [internal_api]
                shared_secret = "${INTERNAL_API_SECRET}"
            "#,
        );
        let _guard = EnvVarGuard::remove(&["INTERNAL_API_SECRET"]);

        let result = load_config_from_dir(dir_str(dir.path()));

        let err = result.expect_err("an unset placeholder variable must fail closed, not load");
        assert!(
            err.to_string().contains("INTERNAL_API_SECRET"),
            "the error must name the missing variable, got: {err}"
        );
    }

    #[test]
    fn escaped_placeholder_yields_literal_dollar_brace_without_env_lookup() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [internal_api]
                shared_secret = "$${LITERAL_NOT_A_VAR}"
            "#,
        );
        // LITERAL_NOT_A_VAR is deliberately left unset: if `$${` were ever
        // treated as a placeholder opener instead of an escape, resolution
        // would fail closed here instead of loading.
        let _guard = EnvVarGuard::remove(&["LITERAL_NOT_A_VAR"]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        assert_eq!(
            config.internal_api.shared_secret.as_deref(),
            Some("${LITERAL_NOT_A_VAR}")
        );
        assert_ne!(
            config.internal_api.shared_secret.as_deref(),
            Some("$${LITERAL_NOT_A_VAR}"),
            "the escape's leading '$$' must be collapsed to a single '$'"
        );
    }

    #[test]
    fn value_without_a_placeholder_is_unchanged() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [server]
                host = "plain-value.example.com"
            "#,
        );

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        assert_eq!(config.server.host, "plain-value.example.com");
        assert_eq!(config.server.port, AppConfig::default().server.port);
    }

    #[test]
    fn placeholder_inside_a_nested_map_valued_section_resolves() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [providers.google]
                adapter = "oidc"
                client_secret = "${GOOGLE_CLIENT_SECRET}"
            "#,
        );
        let _guard = EnvVarGuard::set(&[("GOOGLE_CLIENT_SECRET", "nested-secret")]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        let google = config
            .providers
            .get("google")
            .expect("google provider present");
        assert_eq!(google.adapter, "oidc");
        assert_eq!(
            google.extra.get("client_secret").and_then(|v| v.as_str()),
            Some("nested-secret")
        );
    }

    /// End-to-end reviewability case: a set variable resolves, an unset one
    /// aborts naming itself, and the escape produces a literal — the exact
    /// scenario in the task's Definition of done.
    #[test]
    fn set_unset_and_escaped_placeholders_together_match_reviewable_scenario() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [internal_api]
                shared_secret = "${SET_VAR}"
                auth_method = "${UNSET_VAR}"
            "#,
        );
        let _guard = EnvVarGuard::set(&[("SET_VAR", "resolved-value")]);
        let _unset_guard = EnvVarGuard::remove(&["UNSET_VAR"]);

        let err = load_config_from_dir(dir_str(dir.path()))
            .expect_err("UNSET_VAR must fail the whole load closed");
        assert!(
            err.to_string().contains("UNSET_VAR"),
            "the error must name the unset variable, got: {err}"
        );

        // Replace the unset placeholder with the escape form and reload —
        // the set variable still resolves and the escape becomes a literal.
        write_toml(
            dir.path(),
            "default",
            r#"
                [internal_api]
                shared_secret = "${SET_VAR}"
                auth_method = "$${LITERAL}"
            "#,
        );
        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");
        assert_eq!(
            config.internal_api.shared_secret.as_deref(),
            Some("resolved-value")
        );
        assert_eq!(
            config.internal_api.auth_method.as_deref(),
            Some("${LITERAL}")
        );
    }

    // -----------------------------------------------------------------
    // Validation wiring
    // -----------------------------------------------------------------

    /// A config with an invalid `server.role` must fail `load_config`
    /// itself — validation runs on the fully merged, fully resolved
    /// config, before any adapter or router is built.
    #[test]
    fn load_config_rejects_invalid_role_before_building_anything() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [server]
                role = "exchang"
            "#,
        );

        let err = load_config_from_dir(dir_str(dir.path()))
            .expect_err("an unknown server.role must be rejected at load, not absorbed");

        assert!(
            err.to_string().contains("role"),
            "the error must name the offending field, got: {err}"
        );
        assert!(
            err.to_string().contains("exchang"),
            "the error must name the offending value, got: {err}"
        );
    }

    /// A well-formed config must still load successfully once validation is
    /// wired in — the negative-space test above only proves half the
    /// contract without this counterpart.
    #[test]
    fn load_config_accepts_valid_config() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            r#"
                [server]
                role = "exchange"

                [registration]
                domain_allowlist = ["example.com", "*.example.org"]
            "#,
        );

        let config = load_config_from_dir(dir_str(dir.path()))
            .expect("a well-formed config must load and validate successfully");

        assert_eq!(config.server.role, "exchange");
        assert_eq!(
            config.registration.domain_allowlist,
            Some(vec!["example.com".to_string(), "*.example.org".to_string()])
        );
    }

    /// `parse_config` (the FFI entry point) must reject the same invalid
    /// config as `load_config`, so `OidcExchange::new`/`from_file` fail at
    /// construction rather than at request time.
    #[test]
    fn parse_config_rejects_invalid_role() {
        let _env_lock = lock_test_environment();
        let toml_str = r#"
            [server]
            role = "bogus"
        "#;

        let err =
            parse_config(toml_str).expect_err("an unknown server.role must be rejected at parse");

        assert!(
            err.to_string().contains("role"),
            "the error must name the offending field, got: {err}"
        );
        assert!(
            err.to_string().contains("bogus"),
            "the error must name the offending value, got: {err}"
        );
    }

    /// A well-formed config must still parse successfully through the FFI
    /// entry point.
    #[test]
    fn parse_config_accepts_valid_config() {
        let _env_lock = lock_test_environment();
        let toml_str = r#"
            [server]
            role = "all"
        "#;

        let config = parse_config(toml_str).expect("a well-formed config must parse and validate");

        assert_eq!(config.server.role, "all");
    }
}

// ---------------------------------------------------------------------------
// build_router tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod build_router_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use std::collections::HashMap;
    use tower::ServiceExt;

    use oidc_exchange_core::ports::IdentityProvider;
    use oidc_exchange_test_utils::{
        MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
    };

    const TEST_SECRET: &str = "test-internal-secret-build-router";

    /// Build an `AppService` backed entirely by in-memory mocks, matching
    /// the given `AppConfig`.
    fn build_test_service(config: &AppConfig) -> AppService {
        let provider = MockIdentityProvider::new("test");
        let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
        providers.insert("test".to_string(), Box::new(provider));

        AppService::new(
            Box::new(MockRepository::new()),
            Box::new(MockRepository::new()),
            Box::new(MockKeyManager::new()),
            Box::new(MockAuditLog::new()),
            Box::new(MockUserSync::new()),
            providers,
            config.clone(),
        )
    }

    async fn get(app: Router, uri: &str, bearer: Option<&str>) -> StatusCode {
        let mut builder = Request::builder().method("GET").uri(uri);
        if let Some(token) = bearer {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        response.status()
    }

    async fn body_to_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// `internal_api.enabled = true` with role `admin` mounts `/internal/*`
    /// behind the Bearer check: reachable with the right token, 401 without.
    #[tokio::test]
    async fn enabled_true_admin_mounts_internal_behind_bearer_auth() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(TEST_SECRET.to_string());
        let service = build_test_service(&config);

        let app = build_router(&config, service);
        assert_eq!(
            get(app.clone(), "/internal/stats", Some(TEST_SECRET)).await,
            StatusCode::OK,
            "the correct bearer token must reach the internal handler"
        );
        assert_eq!(
            get(app, "/internal/stats", None).await,
            StatusCode::UNAUTHORIZED,
            "a missing bearer token must be rejected"
        );
    }

    /// `internal_api.enabled = true` with role `all` mounts both the public
    /// routes and `/internal/*` behind the Bearer check.
    #[tokio::test]
    async fn enabled_true_all_mounts_public_and_internal() {
        let mut config = AppConfig::default();
        config.server.role = "all".to_string();
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(TEST_SECRET.to_string());
        let service = build_test_service(&config);

        let app = build_router(&config, service);
        assert_eq!(get(app.clone(), "/health", None).await, StatusCode::OK);
        assert_eq!(
            get(app, "/internal/stats", Some(TEST_SECRET)).await,
            StatusCode::OK
        );
    }

    /// `internal_api.enabled = false` mounts no `/internal/*` route for
    /// `role = "admin"`, which instead serves only `/health` — startup must
    /// not error and the instance stays observable.
    #[tokio::test]
    async fn enabled_false_admin_serves_only_health_no_internal_routes() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.internal_api.enabled = false;
        let service = build_test_service(&config);

        let app = build_router(&config, service);
        assert_eq!(
            get(app.clone(), "/health", None).await,
            StatusCode::OK,
            "an admin instance must stay observable via /health"
        );
        assert_eq!(
            get(app, "/internal/stats", Some("irrelevant")).await,
            StatusCode::NOT_FOUND,
            "with the flag off, /internal/* must not be mounted at all, not merely unauthorized"
        );
    }

    /// `internal_api.enabled = false` with role `all` still mounts the
    /// public routes and `/health`, but no `/internal/*` route.
    #[tokio::test]
    async fn enabled_false_all_serves_public_and_health_no_internal_routes() {
        let mut config = AppConfig::default();
        config.server.role = "all".to_string();
        config.internal_api.enabled = false;
        let service = build_test_service(&config);

        let app = build_router(&config, service);
        assert_eq!(get(app.clone(), "/health", None).await, StatusCode::OK);
        assert_eq!(
            get(app.clone(), "/keys", None).await,
            StatusCode::OK,
            "public routes must still be mounted"
        );
        assert_eq!(
            get(app, "/internal/stats", Some("irrelevant")).await,
            StatusCode::NOT_FOUND,
            "with the flag off, /internal/* must not be mounted at all"
        );
    }

    /// An empty configured `shared_secret` must never be treated as
    /// "configured" by the auth middleware, even when the request supplies
    /// an equally empty bearer token — defence in depth alongside
    /// `AppConfig::validate`, which already refuses to start a role that
    /// serves the internal API with an empty secret.
    #[tokio::test]
    async fn empty_shared_secret_is_never_accepted_as_configured() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(String::new());
        let service = build_test_service(&config);

        let app = build_router(&config, service);
        let request = Request::builder()
            .method("GET")
            .uri("/internal/stats")
            .header("authorization", "Bearer ")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["error_description"], "internal API not configured");
    }
}

// ---------------------------------------------------------------------------
// Request-timeout layer tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod request_timeout_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use std::time::Duration;
    use tower::ServiceExt;

    /// `request_timeout_duration` resolves the config default (`"30s"`) to exactly 30
    /// seconds — the value `06-configuration.md`'s Defaults summary documents.
    #[test]
    fn request_timeout_duration_resolves_documented_default() {
        let config = AppConfig::default();

        let duration = request_timeout_duration(&config);

        assert_eq!(duration, Duration::from_secs(30));
        assert_eq!(
            config.server.request_timeout,
            oidc_exchange_core::config::DEFAULT_REQUEST_TIMEOUT
        );
    }

    /// An overridden `server.request_timeout` parses to the matching `Duration`, not just
    /// the default.
    #[test]
    fn request_timeout_duration_resolves_configured_override() {
        let mut config = AppConfig::default();
        config.server.request_timeout = "2m".to_string();

        let duration = request_timeout_duration(&config);

        assert_eq!(duration, Duration::from_secs(120));
    }

    /// Negative-space: an unparseable `server.request_timeout` must panic rather than
    /// silently building a `TimeoutLayer` from a fallback duration — `AppConfig::validate`
    /// is expected to have already rejected this at config load, so reaching this function
    /// with a bad value is a programmer error that must fail loudly.
    #[test]
    fn request_timeout_duration_panics_on_unparseable_value() {
        let mut config = AppConfig::default();
        config.server.request_timeout = "not-a-duration".to_string();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            request_timeout_duration(&config)
        }));

        assert!(
            result.is_err(),
            "an unparseable request_timeout must panic, not silently default"
        );
    }

    /// Negative-space: a zero-second `request_timeout` — a value `parse_duration_secs`
    /// happily parses but that would time out every request instantly — must trip the
    /// non-zero assertion in `request_timeout_duration` rather than build a degenerate
    /// `TimeoutLayer`.
    #[test]
    fn request_timeout_duration_panics_on_zero_seconds() {
        let mut config = AppConfig::default();
        config.server.request_timeout = "0s".to_string();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            request_timeout_duration(&config)
        }));

        assert!(
            result.is_err(),
            "a zero-second request_timeout must panic rather than build a degenerate timeout \
             layer"
        );
    }

    /// Builds the production middleware ordering around two bare test handlers by calling
    /// the same [`apply_route_layers`] / [`wrap_with_base_path_under_outer_guard`] functions
    /// `build_router` uses — request-id outermost, then the timeout layer, then
    /// audit-context/catch-panic, the whole thing wrapped by the base-path service (a
    /// pass-through with no prefix configured) under the outer catch-panic guard — so the
    /// layer-ordering contract is exercised directly against a slow and a fast handler.
    fn timeout_test_app(timeout: Duration) -> Router {
        async fn fast_handler() -> &'static str {
            "ok"
        }
        async fn slow_handler() -> &'static str {
            tokio::time::sleep(Duration::from_secs(60)).await;
            "too slow to ever return"
        }

        let inner = Router::new()
            .route("/fast", get(fast_handler))
            .route("/slow", get(slow_handler));
        let stated = apply_route_layers(
            inner,
            timeout,
            oidc_exchange_core::config::DEFAULT_MAX_REQUEST_BODY_BYTES,
        );
        wrap_with_base_path_under_outer_guard(stated, None)
    }

    /// A handler that runs longer than `server.request_timeout` is aborted with `408`, and
    /// — because the timeout layer sits inside the request-id layer — the response still
    /// carries the echoed `x-request-id` header.
    #[tokio::test]
    async fn slow_handler_past_timeout_yields_408_with_request_id() {
        let app = timeout_test_app(Duration::from_millis(50));

        let response = app
            .oneshot(Request::get("/slow").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        assert!(
            response.headers().get("x-request-id").is_some(),
            "a timeout response must still carry the request id: the timeout layer sits \
             inside (nearer the handler than) the request-id layer, so request-id processes \
             the response on the way back out"
        );
    }

    /// A handler that finishes well under `server.request_timeout` completes normally with
    /// `200`, proving the timeout layer does not interfere with ordinary fast requests.
    #[tokio::test]
    async fn fast_handler_under_timeout_yields_200() {
        let app = timeout_test_app(Duration::from_millis(50));

        let response = app
            .oneshot(Request::get("/fast").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers().get("x-request-id").is_some(),
            "a normal response must still carry the request id"
        );
    }
}

/// Forced-panic containment over the two-guard stack (`04-http-api.md` → Middleware stack):
/// the inner guard nearest the handlers must turn a *handler* panic into the standard
/// structured `500` while its response still passes back out through the request-id layer
/// and carries `x-request-id`, and the outer guard around the base-path service must contain
/// a panic raised *outside* that inner guard as the same `500` instead of letting an unwind
/// escape into an embedding host or drop the connection. Both tests compose
/// [`apply_route_layers`] / [`wrap_with_base_path_under_outer_guard`] — the very functions
/// `build_router` uses — so what they prove about layer order is true of the shipped router,
/// not merely of a hand-mirrored copy of it.
#[cfg(test)]
mod panic_containment_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// Handler whose only job is to panic: the forced inner-guard probe standing in for any
    /// handler defect.
    async fn panicking_handler() -> &'static str {
        panic!("forced handler panic for containment testing");
    }

    /// Middleware whose only job is to panic before calling `next`. It sits outside the
    /// per-route stack's inner catch-panic but inside the outer guard — the same belt the
    /// request-id, timeout, audit-context, and base-path layers occupy in production, where a
    /// panic would previously have escaped the inner guard entirely.
    async fn panicking_outer_layer_middleware(
        _request: Request<Body>,
        _next: axum::middleware::Next,
    ) -> axum::response::Response {
        panic!("forced outer-layer panic for containment testing");
    }

    /// The production stack over one panicking route: routes → [`apply_route_layers`]
    /// (inner catch-panic innermost) → [`wrap_with_base_path_under_outer_guard`].
    fn inner_panic_app() -> Router {
        let inner = Router::new().route("/boom", get(panicking_handler));
        let stated = apply_route_layers(
            inner,
            std::time::Duration::from_secs(30),
            oidc_exchange_core::config::DEFAULT_MAX_REQUEST_BODY_BYTES,
        );
        wrap_with_base_path_under_outer_guard(stated, None)
    }

    /// The production stack with one panic-probe middleware added just outside the per-route
    /// layers (so the inner guard never sees it) but still inside the outer guard.
    fn outer_panic_app() -> Router {
        let inner = Router::new().route("/fast", get(|| async { "ok" }));
        let stated = apply_route_layers(
            inner,
            std::time::Duration::from_secs(30),
            oidc_exchange_core::config::DEFAULT_MAX_REQUEST_BODY_BYTES,
        )
        .layer(axum::middleware::from_fn(panicking_outer_layer_middleware));
        wrap_with_base_path_under_outer_guard(stated, None)
    }

    async fn get_response(app: Router, uri: &str) -> axum::response::Response {
        app.oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    /// A handler panic becomes the standard structured `500` — not a dropped connection —
    /// and, because request-id wraps the inner catch-panic, the response still carries
    /// `x-request-id`. This is the property the two-guard design exists to preserve: moving
    /// the single guard outward would have cost this response its correlation header.
    #[tokio::test]
    async fn handler_panic_yields_structured_500_that_still_carries_request_id() {
        let response = get_response(inner_panic_app(), "/boom").await;

        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "a caught handler panic must surface as the standard 500"
        );
        assert!(
            response.headers().get("x-request-id").is_some(),
            "a caught handler panic's 500 must pass back out through the request-id layer \
             and carry x-request-id"
        );

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).expect("the standard 500 body is JSON");
        assert_eq!(
            body["error"], "server_error",
            "the 500 body must carry the fixed error code"
        );
        assert_eq!(
            body["error_description"], "internal server error",
            "the 500 body must carry the fixed description"
        );
    }

    /// A panic raised in a layer the inner guard does not cover (here a probe standing in
    /// for the request-id/timeout/audit-context/base-path belt) is contained by the outer
    /// guard as the same standard `500` instead of unwinding into the host. `x-request-id`
    /// is absent on purpose: the probe sits outside the layer that stamps it, which is the
    /// documented trade-off the second guard accepts rather than leaking the unwind.
    #[tokio::test]
    async fn outer_middleware_panic_is_contained_by_the_outer_guard_as_500() {
        let response = get_response(outer_panic_app(), "/fast").await;

        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "an outer-layer panic must be contained as the standard 500"
        );
        assert!(
            response.headers().get("x-request-id").is_none(),
            "a panic from a layer outside request-id cannot carry its header; asserting the \
             absence pins the two-guard placement rather than leaving it accidental"
        );

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).expect("the standard 500 body is JSON");
        assert_eq!(
            body["error"], "server_error",
            "the outer guard must produce the same fixed error code as the inner one"
        );
        assert_eq!(
            body["error_description"], "internal server error",
            "the outer guard must produce the same fixed description as the inner one"
        );
    }

    /// Negative control for the containment pair above: through the identical stack, a
    /// non-panicking request still reaches its handler and returns normally, proving both
    /// guards are transparent on the happy path (a guard that swallowed every request would
    /// also satisfy the two 500 tests).
    #[tokio::test]
    async fn happy_path_passes_through_both_guards_unchanged() {
        let inner = Router::new().route("/fast", get(|| async { "ok" }));
        let stated = apply_route_layers(
            inner,
            std::time::Duration::from_secs(30),
            oidc_exchange_core::config::DEFAULT_MAX_REQUEST_BODY_BYTES,
        );
        let app = wrap_with_base_path_under_outer_guard(stated, None);

        let response = get_response(app, "/fast").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers().get("x-request-id").is_some(),
            "an ordinary response through both guards still carries the request id"
        );
    }
}

/// Exercises `build_user_repository`/`build_session_repository`'s `postgres` arms end to
/// end — through the same `AppConfig` plumbing a real bootstrap uses — rather than calling
/// `oidc_exchange_adapters::postgres::create_pool` directly (which the adapter's own gated
/// suite already covers). This is the layer that changed in this task: reading
/// `pg_cfg.run_migrations` out of config and threading it into `create_pool`, for both the
/// user and session pools.
#[cfg(test)]
mod postgres_bootstrap_tests {
    use super::*;
    use chrono::{Duration, Utc};
    use oidc_exchange_core::config::PostgresConfig;
    use oidc_exchange_core::domain::{NewUser, Session};
    use uuid::Uuid;

    /// An `AppConfig` whose user repository (and, since `session_repository.adapter` is
    /// left unset, the session repository via its documented fallback) both target
    /// Postgres at `url`, with `run_migrations` left as given.
    fn postgres_config(url: &str, run_migrations: Option<bool>) -> AppConfig {
        let mut config = AppConfig::default();
        config.repository.adapter = "postgres".to_string();
        config.repository.postgres = Some(PostgresConfig {
            url: url.to_string(),
            max_connections: None,
            run_migrations,
        });
        config
    }

    /// Gated on `DATABASE_URL` (skips cleanly, not a failure, when unset, so
    /// `cargo nextest run --workspace` stays green without a live database configured).
    ///
    /// Manual boot check when `DATABASE_URL` is unset: start Postgres (e.g.
    /// `docker compose -f examples/linux-postgres/docker-compose.yml up -d`), point
    /// `[repository.postgres].url` at it with `repository.adapter = "postgres"` and no
    /// `[session_repository]` override, boot the server against a fresh database, and
    /// confirm a request that touches the repository (e.g. registering a user) succeeds
    /// instead of 500ing; then set `run_migrations = false` against that same
    /// already-migrated database and confirm startup still connects and serves requests.
    ///
    /// With `run_migrations` left absent, both `build_user_repository` and
    /// `build_session_repository` must leave a fresh database serviceable: the DDL is
    /// idempotent, so the session pool's migration run is a no-op over what the user
    /// pool already created.
    #[tokio::test]
    async fn postgres_bootstrap_migrates_both_pools_and_is_serviceable_on_startup() {
        let Ok(base_url) = std::env::var("DATABASE_URL") else {
            eprintln!(
                "skipping postgres_bootstrap_migrates_both_pools_and_is_serviceable_on_startup: \
                 DATABASE_URL is not set"
            );
            return;
        };

        let config = postgres_config(&base_url, None);

        // Build and exercise the user pool alone first, before the session pool is built,
        // so a wiring regression confined to `build_user_repository` (e.g. a hard-coded
        // `false` slipping back in) cannot be masked by the session pool's migration also
        // creating the `users` table as a side effect.
        let user_repo = build_user_repository(&config)
            .await
            .expect("build_user_repository must connect and migrate a fresh database");

        let external_id = format!("bootstrap-test|{}", Uuid::new_v4());
        let created = user_repo
            .create_user(&NewUser {
                external_id: external_id.clone(),
                provider: "bootstrap-test".to_string(),
                email: Some(format!("{external_id}@example.com")),
                display_name: None,
            })
            .await
            .expect("create_user must succeed once bootstrap has migrated the user pool");
        let fetched = user_repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("the user created through the bootstrap-built repository must round-trip");
        assert_eq!(fetched.id, created.id, "round-tripped user id must match");
        assert_eq!(
            fetched.external_id, external_id,
            "round-tripped user external_id must match"
        );

        let session_repo = build_session_repository(&config).await.expect(
            "build_session_repository must connect and migrate (a no-op the second time) \
                 the same database",
        );

        let now = Utc::now();
        let refresh_token_hash = format!("bootstrap-test-hash-{}", Uuid::new_v4());
        let session = Session {
            user_id: created.id.clone(),
            refresh_token_hash: refresh_token_hash.clone(),
            provider: "bootstrap-test".to_string(),
            expires_at: now + Duration::hours(1),
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: now,
        };
        session_repo.store_refresh_token(&session).await.expect(
            "store_refresh_token must succeed once bootstrap has migrated the session pool",
        );
        let fetched_session = session_repo
            .get_session_by_refresh_token(&refresh_token_hash)
            .await
            .expect("get_session_by_refresh_token")
            .expect(
                "the session stored through the bootstrap-built session repository must \
                 round-trip",
            );
        assert_eq!(
            fetched_session.user_id, created.id,
            "round-tripped session user_id must match"
        );
        assert_eq!(
            fetched_session.refresh_token_hash, refresh_token_hash,
            "round-tripped session refresh_token_hash must match"
        );
    }
}
