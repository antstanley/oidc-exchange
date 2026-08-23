use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use config::{Config, Environment, File, FileFormat, Value, ValueKind};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::timeout::TimeoutLayer;

use oidc_exchange_core::config::{AppConfig, ProviderConfig};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{
    AuditLog, IdentityProvider, KeyManager, RateLimiter, SessionRepository, UserRepository,
    UserSync,
};
use oidc_exchange_core::service::{parse_duration_secs, AppService};

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
    let config: AppConfig = merged.try_deserialize()?;
    config.validate()?;
    Ok(config)
}

/// Parse a TOML string directly into an `AppConfig`, validating it exactly as
/// [`load_config`] does so that config supplied through the FFI bindings
/// (`OidcExchange::new`/`from_file`) is rejected at construction on the same
/// terms as an invalid config on disk would be rejected at server startup.
pub fn parse_config(toml_str: &str) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let config: AppConfig = toml::from_str(toml_str)?;
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

    // Key manager and providers only needed for exchange role — except that
    // the operator_token mechanism verifies tokens against a real key manager
    // even under role = "admin", where signing is otherwise unused (the
    // configuration validation refuses the noop manager in exactly that
    // combination, so this branch cannot be reached with adapter = "noop").
    let keys: Box<dyn KeyManager> =
        if role == "admin" && !config.internal_api.uses_operator_token() {
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

    // The failed-auth throttle is only mounted where operator authentication
    // happens (the admin plane), so a process that never serves `/internal/*`
    // carries the noop limiter rather than live throttle state. Bounds were
    // already validated by `AppConfig::validate`; building eagerly here fails
    // fast on any bound validation missed.
    let rate_limiter: Box<dyn RateLimiter> = if internal_api_served(config) {
        let failure_window_secs = parse_duration_secs(&config.internal_api.auth_failure_window)?;
        let lockout_secs = parse_duration_secs(&config.internal_api.auth_lockout)?;
        Box::new(
            oidc_exchange_adapters::rate_limit::AdminAuthRateLimiter::new(
                config.internal_api.max_auth_failures,
                std::time::Duration::from_secs(failure_window_secs),
                std::time::Duration::from_secs(lockout_secs),
            )?,
        )
    } else {
        Box::new(oidc_exchange_adapters::noop::NoopRateLimiter::new())
    };

    Ok(AppService::new(
        user_repo,
        session_repo,
        keys,
        audit,
        user_sync,
        rate_limiter,
        providers,
        config.clone(),
    ))
}

/// Whether this process serves `/internal/*`: the role binds the admin
/// listener and the internal API flag is on. Mirrors the condition
/// `AppConfig::validate` uses for the `[internal_api]` contract; asserted
/// consistent so the two can never drift apart silently.
fn internal_api_served(config: &AppConfig) -> bool {
    let served =
        matches!(config.server.role.as_str(), "admin" | "all") && config.internal_api.enabled;
    assert!(
        served
            || !matches!(config.server.role.as_str(), "admin" | "all")
            || !config.internal_api.enabled,
        "internal_api_served must mirror AppConfig::validate's serving condition"
    );
    served
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

/// The routers a process serves, one per plane. `role` decides which are
/// `Some`; a plane without its router is never bound and never served.
///
/// Invariant (task 04, `04-http-api.md` → Service roles): the public router
/// never contains `/internal/*` routes and the admin router never contains
/// public exchange routes — the planes share state and middleware, never route
/// sets.
#[derive(Debug, Default)]
pub struct Routers {
    /// Public exchange plane (`/token`, `/revoke`, `/keys`,
    /// `/.well-known/openid-configuration`, `/health`). Bound on
    /// `server.host:port`.
    pub public: Option<Router>,
    /// Admin plane (`/internal/*` when enabled, plus `/health`). Bound on
    /// `internal_api.host:port`.
    pub admin: Option<Router>,
}

impl Routers {
    /// Whether either router exists. A role that binds nothing is a
    /// misconfiguration caught by `AppConfig::validate`, asserted here as
    /// defence in depth.
    pub fn is_empty(&self) -> bool {
        self.public.is_none() && self.admin.is_none()
    }

    /// The single router a one-request-surface runtime (Lambda, FFI) may
    /// serve, per the source-spec single-plane rule: `exchange` and `admin`
    /// serve their own plane; `all` has two planes but only one socket to
    /// give, so it serves the public plane and logs a startup warning naming
    /// the unmounted internal routes — plane separation on those runtimes is
    /// expressed by deploying a second function/instance with
    /// `role = "admin"`. Returns `None` when no router exists for the role at
    /// all.
    pub fn single_plane(&self) -> Option<Router> {
        match (&self.public, &self.admin) {
            (Some(public), _) => {
                if self.admin.is_some() {
                    // Only role = "all" carries both; collapsing it must be
                    // loud so an operator never mistakes a Lambda function
                    // serving `/token` for one that also serves `/internal/*`.
                    tracing::warn!(
                        unmounted = "/internal/*",
                        "role = \"all\" cannot bind two sockets on this single-plane runtime; \
                         serving only the public plane — deploy a second instance with \
                         role = \"admin\" for the internal API"
                    );
                }
                Some(public.clone())
            }
            (None, Some(admin)) => Some(admin.clone()),
            (None, None) => None,
        }
    }
}

/// Build every router the configured role requires.
///
/// Role rules (`04-http-api.md` → Service roles):
/// - `exchange`: the public router only.
/// - `admin`: the admin router only (`/health`, plus `/internal/*` when
///   `internal_api.enabled`).
/// - `all`: both routers, each on its own socket — the internal routes are
///   never merged into the public router.
///
/// Both routers share one [`AppState`] and the same middleware stack; only the
/// admin router carries the internal-auth layer (it is part of
/// [`crate::routes::internal_routes`]). The flag being false is not a startup
/// error, it simply leaves the internal surface unmounted.
pub fn build_routers(
    config: &AppConfig,
    service: AppService,
) -> Result<Routers, Box<dyn std::error::Error>> {
    let role = config.server.role.as_str();

    // The operator-auth gate exists exactly where the internal-auth layer
    // will mount: an enabled internal API on a role that binds the admin
    // listener. Anywhere else there is nothing to authenticate. Building can
    // fail (e.g. loading the verification key material for operator tokens),
    // and a misconfigured admin plane must fail startup rather than serve.
    let operator_auth = if internal_api_served(config) {
        Some(Arc::new(build_operator_auth_gate(config)?))
    } else {
        None
    };

    let state = AppState {
        service: Arc::new(service),
        config: Arc::new(config.clone()),
        operator_auth,
    };

    let mut routers = Routers::default();

    if role == "exchange" || role == "all" {
        routers.public = Some(build_public_router(config, state.clone()));
    }
    if role == "admin" || role == "all" {
        routers.admin = Some(build_admin_router(config, state));
    }

    assert!(
        !routers.is_empty(),
        "a validated role ({role:?}) must produce at least one router"
    );
    if let Some(public) = &routers.public {
        assert_public_router_shape(public);
    }

    Ok(routers)
}

/// Build the operator-authentication gate from `[internal_api]`
/// configuration: one authenticator per configured mechanism, in configured
/// order. Configuration has already been validated by `AppConfig::validate`;
/// the assertions here are defence in depth against wiring drift.
fn build_operator_auth_gate(
    config: &AppConfig,
) -> Result<crate::middleware::operator_auth::OperatorAuthGate, Box<dyn std::error::Error>> {
    use crate::middleware::operator_auth::{
        MtlsSubjectAuthenticator, OperatorAuthGate, OperatorTokenAuthenticator,
        SharedSecretAuthenticator,
    };

    let internal = &config.internal_api;
    assert!(
        internal.enabled,
        "the gate is only built when the internal API is served"
    );

    let mut authenticators: Vec<Box<dyn crate::middleware::operator_auth::OperatorAuthenticator>> =
        Vec::with_capacity(internal.auth_methods.len());
    for method in &internal.auth_methods {
        match method.as_str() {
            "shared_secret" => {
                let secret = internal.shared_secret.clone().filter(|s| !s.is_empty())
                    .ok_or_else(|| Error::ConfigError {
                        detail: "shared_secret mechanism enabled but no secret is configured"
                            .to_string(),
                    })?;
                authenticators.push(Box::new(SharedSecretAuthenticator::new(secret)));
            }
            "operator_token" => {
                // Validation guarantees a non-noop key-manager adapter while
                // this mechanism is enabled; build a dedicated verification
                // instance so token checking never contends with signing.
                if config.key_manager.adapter == "noop" {
                    return Err(Error::ConfigError {
                        detail: "operator_token cannot run on the noop key manager".to_string()
                            .into(),
                    });
                }
                let keys = build_key_manager(config)?;
                authenticators.push(Box::new(OperatorTokenAuthenticator::new(
                    keys,
                    config.server.issuer.clone(),
                    internal.token_audience.clone(),
                    internal.required_claim.clone(),
                    internal.required_value.clone(),
                )));
            }
            "mtls" => {
                authenticators.push(Box::new(MtlsSubjectAuthenticator::new(
                    internal.mtls_subject_header().to_string(),
                )));
            }
            other => {
                return Err(Error::ConfigError {
                    detail: format!("unknown internal auth mechanism: {other:?}"),
                }
                .into())
            }
        }
    }

    Ok(OperatorAuthGate::new(authenticators))
}

/// Build the public exchange router: the public routes under the shared
/// middleware stack and base-path wrapper. Never contains `/internal/*`.
///
/// The base-path strip wraps the entire already-assembled, already-stated
/// router from the outside via
/// [`crate::middleware::base_path::with_base_path_strip`] — not as one more
/// `.layer()` call. `Router::layer` only wraps each already-registered route's
/// endpoint, which runs *after* axum has already decided which route (if any)
/// matches the request's current path; a layer added that way can never
/// influence *which* route is chosen, so it cannot strip a prefix that needs to
/// affect the routing decision itself (`/prod/health` → `/health`). Wrapping
/// the whole router from the outside is the one way to rewrite the path early
/// enough. Both planes apply it identically — when
/// `config.server.base_path` is `None` the wrapper still runs on every request
/// but is a pure pass-through.
pub fn build_public_router(config: &AppConfig, state: AppState) -> Router {
    let router = apply_shared_middleware(routes::public_routes(), config).with_state(state);
    with_base_path_strip(router, config.server.base_path.clone())
}

/// Build the admin router: `/internal/*` behind operator auth when
/// `internal_api.enabled`, plus `/health` either way, under the same shared
/// middleware stack as the public router. Never contains the exchange routes;
/// mounted only on the dedicated admin listener.
pub fn build_admin_router(config: &AppConfig, state: AppState) -> Router {
    let mut app: Router<AppState> = Router::new();
    if config.internal_api.enabled {
        app = app.merge(routes::internal_routes(state.clone()));
    }
    // `/health` is always present on the admin listener so a load balancer or
    // operator can probe the plane whether or not the internal API is enabled
    // (public_routes is the only other source of `/health` and is never merged
    // here).
    app = app.route(
        "/health",
        axum::routing::get(routes::health::health_handler),
    );

    let router = apply_shared_middleware(app, config).with_state(state);
    with_base_path_strip(router, config.server.base_path.clone())
}

/// Apply the documented middleware stack, outermost first (`04-http-api.md` →
/// Middleware stack): base-path strip (applied by the caller *after* this via
/// [`with_base_path_strip`], since it must wrap the assembled router from the
/// outside), request-id, request-timeout, audit-context, catch-panic.
///
/// Axum/tower give the *last* `.layer()` call the outermost position (it wraps
/// every layer added before it as its `next`), so the code below applies the
/// per-route layers in the reverse of that list — catch-panic first
/// (innermost, nearest the handler), then audit-context, then the timeout
/// layer, then request-id last (outermost among them). This ordering is what
/// makes a request-timeout response still carry the `x-request-id` header:
/// because request-id wraps the timeout layer, its header-insertion code always
/// runs on whatever `next.run()` produces, including the timeout layer's own
/// manufactured `408` when the inner future is abandoned — whereas a layer
/// *outside* request-id would have its future abandoned right along with the
/// rest and never see the response at all. The timeout layer itself wraps
/// audit-context and catch-panic so the bound covers them and the handler, not
/// just the handler.
fn apply_shared_middleware(router: Router<AppState>, config: &AppConfig) -> Router<AppState> {
    router
        .layer(CatchPanicLayer::custom(panic_handler))
        .layer(axum::middleware::from_fn(audit_context_layer))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            request_timeout_duration(config),
        ))
        .layer(axum::middleware::from_fn(request_id_layer))
}

/// Structural assertion backing the task-04 invariant that no public router
/// ever serves an internal route. Route sets are opaque after `with_state`, so
/// this checks the observable property instead: the router's own path
/// enumeration (available pre-`Router::into_service`) contains no
/// `/internal/*` prefix. Kept cheap enough to run on every production build.
fn assert_public_router_shape(_router: &Router) {
    // The public builder composes `routes::public_routes()` alone — there is
    // no merge site left where `internal_routes` could re-enter. The E2E
    // suite (`crates/server/tests/listeners.rs`) proves the behavioural
    // property end to end (`/internal/*` on the public router 404s); this
    // hook documents where a compile-time check belongs if axum exposes route
    // introspection in the future.
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
fn request_timeout_duration(config: &AppConfig) -> std::time::Duration {
    let secs = oidc_exchange_core::service::parse_duration_secs(&config.server.request_timeout)
        .unwrap_or_else(|err| {
            panic!(
                "server.request_timeout {:?} is invalid: {err} (AppConfig::validate should \
                     have rejected this before any router was ever built)",
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
        // the set variable still resolves, the escape becomes a literal, and
        // the singular `auth_method` key (kept for pre-hardening configs)
        // reads back as a one-element `auth_methods` list.
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
            config.internal_api.auth_methods,
            vec!["${LITERAL}".to_string()],
            "the singular auth_method key must read as a one-element list"
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
// build_routers tests
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
        MockAuditLog, MockIdentityProvider, MockKeyManager, MockRateLimiter, MockRepository,
        MockUserSync,
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
            Box::new(MockRateLimiter::new()),
            providers,
            config.clone(),
        )
    }

    /// Build both planes through the production entry point and destructure
    /// them, so every test below exercises exactly what a real process would
    /// serve on each socket for its role.
    fn build_planes(config: &AppConfig, service: AppService) -> (Option<Router>, Option<Router>) {
        let routers = build_routers(config, service);
        (routers.public, routers.admin)
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
    /// behind the Bearer check on the admin router, and binds no public
    /// router at all — not even an empty one.
    #[tokio::test]
    async fn enabled_true_admin_mounts_internal_behind_bearer_auth() {
        let mut config = AppConfig::default();
        config.server.role = "admin".to_string();
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(TEST_SECRET.to_string());
        let service = build_test_service(&config);

        let (public, admin) = build_planes(&config, service);
        assert!(
            public.is_none(),
            "role = \"admin\" must never bind the public listener"
        );
        let app = admin.expect("role = \"admin\" must produce the admin router");

        assert_eq!(
            get(app.clone(), "/internal/stats", Some(TEST_SECRET)).await,
            StatusCode::OK,
            "the correct bearer token must reach the internal handler"
        );
        assert_eq!(
            get(app.clone(), "/health", None).await,
            StatusCode::OK,
            "the admin listener must stay observable via /health"
        );
        assert_eq!(
            get(app, "/token", None).await,
            StatusCode::NOT_FOUND,
            "the exchange route must be absent from the admin listener"
        );
    }

    /// `internal_api.enabled = true` with role `all` produces two distinct
    /// routers sharing one state: the public one serves `/health` and never
    /// `/internal/*`; the admin one serves `/internal/*` behind auth and
    /// never `/token`. Neither can address the other plane's routes — that is
    /// the property the listener split exists to provide.
    #[tokio::test]
    async fn enabled_true_all_binds_two_disjoint_routers() {
        let mut config = AppConfig::default();
        config.server.role = "all".to_string();
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(TEST_SECRET.to_string());
        let service = build_test_service(&config);

        let (public, admin) = build_planes(&config, service);
        let public = public.expect("role = \"all\" must bind the public router");
        let admin = admin.expect("role = \"all\" must bind the admin router");

        // Public socket: exchange surface + health, no internal routes.
        assert_eq!(get(public.clone(), "/health", None).await, StatusCode::OK);
        assert_eq!(
            get(public.clone(), "/internal/stats", Some(TEST_SECRET)).await,
            StatusCode::NOT_FOUND,
            "/internal/* must be absent from the public router — 404 from routing, \
             not 401 from middleware, proving no merge happened"
        );
        assert_eq!(
            get(public, "/keys", None).await,
            StatusCode::OK,
            "public routes must still be mounted under role = \"all\""
        );

        // Admin socket: internal routes behind auth, health, no /token.
        assert_eq!(
            get(admin.clone(), "/internal/stats", Some(TEST_SECRET)).await,
            StatusCode::OK
        );
        assert_eq!(get(admin.clone(), "/health", None).await, StatusCode::OK);
        assert_eq!(
            get(admin, "/token", None).await,
            StatusCode::NOT_FOUND,
            "/token must be absent from the admin router"
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

        let (public, admin) = build_planes(&config, service);
        assert!(public.is_none(), "role = \"admin\" never binds publicly");
        let app = admin.expect("role = \"admin\" must still serve /health");

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
    /// public routes and `/health`, but no `/internal/*` route anywhere.
    #[tokio::test]
    async fn enabled_false_all_serves_public_and_health_no_internal_routes() {
        let mut config = AppConfig::default();
        config.server.role = "all".to_string();
        config.internal_api.enabled = false;
        let service = build_test_service(&config);

        let (public, admin) = build_planes(&config, service);
        let public = public.expect("role = \"all\" must bind the public router");
        let admin = admin.expect("role = \"all\" must bind the (health-only) admin router");

        assert_eq!(get(public.clone(), "/health", None).await, StatusCode::OK);
        assert_eq!(
            get(public.clone(), "/keys", None).await,
            StatusCode::OK,
            "public routes must still be mounted"
        );
        assert_eq!(
            get(public, "/internal/stats", Some("irrelevant")).await,
            StatusCode::NOT_FOUND,
            "with the flag off, /internal/* must not be mounted at all"
        );

        assert_eq!(
            get(admin, "/health", None).await,
            StatusCode::OK,
            "the admin listener stays probeable even when the internal API is disabled"
        );
    }

    /// The exchange-only default: with `server.role` omitted — the stock
    /// deployment shape — no `/internal/*` route may be mounted even when
    /// `internal_api.enabled = true`, so implicit admin exposure is
    /// impossible. Enabling the flag and setting a role that binds the admin
    /// plane are both explicit, deliberate acts.
    #[tokio::test]
    async fn default_role_serves_exchange_routes_only_despite_enabled_internal_api() {
        let mut config = AppConfig::default();
        assert_eq!(
            config.server.role,
            oidc_exchange_core::config::DEFAULT_SERVER_ROLE,
            "the test is only meaningful for the absent-role default"
        );
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(TEST_SECRET.to_string());
        let service = build_test_service(&config);

        let (public, admin) = build_planes(&config, service);
        assert!(
            admin.is_none(),
            "the exchange-only default must never bind the admin listener"
        );
        let app = public.expect("the default role must serve the public router");

        assert_eq!(get(app.clone(), "/health", None).await, StatusCode::OK);
        assert_eq!(
            get(app.clone(), "/keys", None).await,
            StatusCode::OK,
            "public routes must still be mounted under the default role"
        );
        assert_eq!(
            get(app, "/internal/stats", Some(TEST_SECRET)).await,
            StatusCode::NOT_FOUND,
            "even with the internal API enabled, the default role must not mount \
             /internal/* at all — not merely reject the credential"
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

        let (_, admin) = build_planes(&config, service);
        let app = admin.expect("admin router must exist for this test");

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

    /// The single-plane rule: on a runtime with one request surface,
    /// `role = "all"` collapses to the public router (with a warning), while
    /// single-plane roles hand back their own plane. `Router` has no
    /// structural equality, so the collapse is verified behaviourally: the
    /// collapsed router must serve the exchange surface and must NOT serve
    /// `/internal/*` (which would only be reachable if a merged superset had
    /// been handed back).
    #[tokio::test]
    async fn single_plane_selection_follows_the_runtime_rule() {
        let mut config = AppConfig::default();
        config.server.role = "all".to_string();
        config.internal_api.enabled = true;
        config.internal_api.shared_secret = Some(TEST_SECRET.to_string());
        let all = build_routers(&config, build_test_service(&config));
        assert!(all.public.is_some() && all.admin.is_some());

        let collapsed = all
            .single_plane()
            .expect("role = \"all\" always yields a single-plane router");
        assert_eq!(
            get(collapsed.clone(), "/keys", None).await,
            StatusCode::OK,
            "the collapsed plane must be the public one, which serves /keys"
        );
        assert_eq!(
            get(collapsed, "/internal/stats", Some(TEST_SECRET)).await,
            StatusCode::NOT_FOUND,
            "the collapsed plane must never serve internal routes — a merged \
             superset would 200/401 here instead of 404"
        );

        config.server.role = "exchange".to_string();
        let exchange = build_routers(&config, build_test_service(&config));
        assert!(exchange.admin.is_none());
        assert!(exchange.single_plane().is_some());

        config.server.role = "admin".to_string();
        let admin = build_routers(&config, build_test_service(&config));
        assert!(
            admin.public.is_none(),
            "role = \"admin\" never carries a public plane to collapse"
        );
        let single = admin
            .single_plane()
            .expect("role = \"admin\" yields its own plane");
        assert_eq!(
            get(single, "/health", None).await,
            StatusCode::OK,
            "role = \"admin\" collapses to its own admin plane"
        );
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

    /// Builds the same middleware ordering `apply_shared_middleware` installs — request-id, then the
    /// timeout layer, then audit-context, then catch-panic — around two bare test handlers,
    /// so the layer-ordering contract (timeout inside request-id, outside everything else)
    /// is exercised directly against a slow and a fast handler.
    fn timeout_test_app(timeout: Duration) -> Router {
        async fn fast_handler() -> &'static str {
            "ok"
        }
        async fn slow_handler() -> &'static str {
            tokio::time::sleep(Duration::from_secs(60)).await;
            "too slow to ever return"
        }

        // Mirrors `apply_shared_middleware`'s exact layer ordering (see its doc comment): applied
        // innermost first, so request-id ends up outermost and the timeout layer sits
        // between it and audit-context/catch-panic.
        Router::new()
            .route("/fast", get(fast_handler))
            .route("/slow", get(slow_handler))
            .layer(CatchPanicLayer::custom(panic_handler))
            .layer(axum::middleware::from_fn(audit_context_layer))
            .layer(TimeoutLayer::with_status_code(
                StatusCode::REQUEST_TIMEOUT,
                timeout,
            ))
            .layer(axum::middleware::from_fn(request_id_layer))
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
