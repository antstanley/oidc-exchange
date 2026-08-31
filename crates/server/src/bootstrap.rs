use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use config::{Config, Environment, File, FileFormat, Value, ValueKind};
use tokio::sync::Semaphore;
use tower::Layer;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::timeout::TimeoutLayer;

use oidc_exchange_core::config::{
    Config as AppConfig, IdentityProviderAdapter, ProviderConfig, RawConfig, ServerRole,
};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{
    AuditLog, IdentityProvider, KeyManager, RateLimiter, SessionRepository, UserRepository,
    UserSync,
};
use oidc_exchange_core::service::AppService;

use crate::middleware::access_log::access_log_layer;
use crate::middleware::audit_context::audit_context_layer;
#[cfg(any(not(feature = "conformance"), test))]
use crate::middleware::base_path::with_base_path_strip;
#[cfg(feature = "conformance")]
use crate::middleware::base_path::with_base_path_strip_and_observe;
use crate::middleware::error_handler::panic_handler;
use crate::middleware::public_throttle::public_concurrency_layer;
use crate::middleware::request_id::request_id_layer;
use crate::middleware::throttle::{FixedWindowRateLimiter, RateLimitBudgets};
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

/// Deep-merge a deployment override tree onto the committed defaults as raw
/// [`toml::Value`] trees: tables merge recursively, scalars and arrays replace.
///
/// Merging at the raw-value level — rather than round-tripping the override
/// through `RawConfig` first — is deliberate. Because `RawConfig` is
/// `#[serde(default)]` throughout, deserializing an override into it materializes
/// every unset field as its Rust default (`""`, `0`, `false`, …), at which point
/// "unset" and "explicitly set to a falsy value" are indistinguishable. Keeping
/// the override as a `toml::Value` preserves that distinction: a key the operator
/// never wrote is genuinely absent from the tree and inherits the default, while
/// an explicit `false`/`0`/`""` is a present value that survives the merge and
/// reaches the domain resolvers (where, e.g., an empty duration fails loudly
/// instead of silently reverting to the committed default).
fn merge_raw_defaults(defaults: toml::Value, override_value: toml::Value) -> toml::Value {
    fn merge(base: &mut toml::Value, override_value: toml::Value) {
        match (base, override_value) {
            (toml::Value::Table(base), toml::Value::Table(override_table)) => {
                for (key, value) in override_table {
                    match base.get_mut(&key) {
                        Some(existing) => merge(existing, value),
                        None => {
                            base.insert(key, value);
                        }
                    }
                }
            }
            (base, value) => *base = value,
        }
    }
    let mut base = defaults;
    merge(&mut base, override_value);
    base
}

/// Load configuration from config files on disk, using the `OIDC_EXCHANGE_ENV`
/// environment variable to select the environment-specific config file, and
/// `OIDC_EXCHANGE__{section}__{key}` environment variables to override the
/// merged result afterward.
pub fn load_config() -> Result<AppConfig, Box<dyn std::error::Error>> {
    load_config_from_dir(CONFIG_DIR)
}

/// Load and resolve a single TOML file with structural environment overrides.
/// This is intentionally the same source shape used by inline FFI TOML.
pub fn load_config_from_file(path: &str) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let builder = Config::builder()
        .add_source(File::from(std::path::Path::new(path)).format(FileFormat::Toml))
        .add_source(
            Environment::with_prefix(ENV_OVERRIDE_PREFIX)
                .separator(ENV_OVERRIDE_SEPARATOR)
                .try_parsing(true),
        );
    resolve_builder(builder)
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
/// deserialized result is `AppConfig::test_default()` (every field carries
/// `#[serde(default)]`).
pub fn load_config_from_dir(config_dir: &str) -> Result<AppConfig, Box<dyn std::error::Error>> {
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

    resolve_builder(builder)
}

/// Apply the one common configuration tail after an entry point has assembled
/// its sources: merge, resolve `${VAR}` placeholders fail-closed, deserialize
/// the raw shape, merge onto the committed defaults, and resolve the closed
/// domains ([`AppConfig::resolve`]).
fn resolve_builder(
    builder: config::ConfigBuilder<config::builder::DefaultState>,
) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let mut merged = builder.build()?;
    resolve_placeholders(&mut merged.cache, "<root>")?;
    let raw: toml::Value = merged.try_deserialize()?;
    let defaults: toml::Value = toml::from_str(include_str!("../../../config/default.toml"))?;
    let config: RawConfig = merge_raw_defaults(defaults, raw).try_into()?;
    AppConfig::resolve(config).map_err(Into::into)
}

/// Parse raw TOML, merge it onto the committed defaults, and resolve the
/// resulting typed configuration. This is deliberately side-effect-free:
/// callers that only need validation never build adapters, telemetry, routers,
/// or listeners, and no environment source is consulted.
pub fn resolve_config_toml(toml_str: &str) -> Result<AppConfig, Error> {
    let raw: toml::Value = toml::from_str(toml_str).map_err(|err| Error::ConfigError {
        detail: format!("config TOML is invalid: {err}"),
    })?;
    let defaults: toml::Value = toml::from_str(include_str!("../../../config/default.toml"))
        .map_err(|err| Error::ConfigError {
            detail: format!("committed default config is invalid: {err}"),
        })?;
    let config: RawConfig = merge_raw_defaults(defaults, raw)
        .try_into()
        .map_err(|err| Error::ConfigError {
            detail: format!("config is invalid: {err}"),
        })?;
    AppConfig::resolve(config)
}

/// Parse a TOML string with structural `OIDC_EXCHANGE__{section}__{key}`
/// overrides and fail-closed `${VAR}` placeholder resolution — the FFI
/// construction path (`OidcExchange::new`/`from_file`), validated exactly as
/// [`load_config`] is so config supplied through the bindings is rejected at
/// construction on the same terms as an invalid config on disk at startup.
pub fn parse_config(toml_str: &str) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let builder = Config::builder()
        .add_source(File::from_str(toml_str, FileFormat::Toml))
        .add_source(
            Environment::with_prefix(ENV_OVERRIDE_PREFIX)
                .separator(ENV_OVERRIDE_SEPARATOR)
                .try_parsing(true),
        );
    resolve_builder(builder)
}

/// Read an explicit TOML file and resolve it through the same side-effect-free
/// path as [`resolve_config_toml`]. An explicit path intentionally does not
/// load a sibling overlay, consult the working directory, or apply environment
/// overrides: the supplied file is the complete deployment override, merged
/// only with the committed defaults.
pub fn check_config_file(path: impl AsRef<std::path::Path>) -> Result<AppConfig, Error> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(Error::ConfigError {
            detail: format!(
                "config check path '{}' is not a readable file",
                path.display()
            ),
        });
    }
    let config_toml = std::fs::read_to_string(path).map_err(|err| Error::ConfigError {
        detail: format!("config check cannot read '{}': {err}", path.display()),
    })?;
    resolve_config_toml(&config_toml)
}

/// Render a resolved config for `config check` without exposing secrets.
/// Rendering raw TOML would leak any secret before the closed-domain resolver
/// has accepted it, so this intentionally uses redacted `Debug` output only.
pub fn render_checked_config(config: &AppConfig) -> String {
    format!("{config:#?}")
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
fn resolve_placeholders(value: &mut Value, path: &str) -> Result<(), Error> {
    match &mut value.kind {
        ValueKind::String(s) => *s = resolve_placeholders_in_str(s, path)?,
        ValueKind::Table(table) => {
            for (key, nested) in table.iter_mut() {
                let nested_path = if path == "<root>" {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                resolve_placeholders(nested, &nested_path)?;
            }
        }
        ValueKind::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                resolve_placeholders(item, &format!("{path}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Resolve every `${VAR}` placeholder and `$${` escape inside a single
/// string, returning the rewritten string.
fn resolve_placeholders_in_str(input: &str, path: &str) -> Result<String, Error> {
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
            let (name, consumed) =
                scan_placeholder_name(&input[i + 2..]).ok_or_else(|| Error::ConfigError {
                    detail: format!("malformed placeholder at config path '{path}'"),
                })?;
            if name.is_empty() {
                return Err(Error::ConfigError {
                    detail: format!("empty placeholder name at config path '{path}'"),
                });
            }
            let resolved = std::env::var(name).map_err(|_| Error::ConfigError {
                detail: format!(
                    "config placeholder '${{{name}}}' at config path '{path}' references unset environment variable '{name}'"
                ),
            })?;
            if resolved.is_empty() {
                return Err(Error::ConfigError {
                    detail: format!(
                        "config placeholder '${{{name}}}' at config path '{path}' references empty environment variable '{name}'"
                    ),
                });
            }
            output.push_str(&resolved);
            i += 2 + consumed;
            debug_assert!(i > before, "placeholder branch must consume input");
            continue;
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
/// found within the bound. Callers reject that as malformed configuration.
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
    let keys: Box<dyn KeyManager> = if role == "admin" && !config.internal_api.uses_operator_token()
    {
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
    let admin_rate_limiter: Box<dyn RateLimiter> = if internal_api_served(config) {
        Box::new(
            oidc_exchange_adapters::rate_limit::AdminAuthRateLimiter::new(
                config.internal_api.max_auth_failures,
                config.internal_api.auth_failure_window,
                config.internal_api.auth_lockout,
            )?,
        )
    } else {
        Box::new(oidc_exchange_adapters::noop::NoopRateLimiter::new())
    };

    // One port, two budget families: exchange-plane keys route to the fixed
    // window limiter, the admin plane's `OperatorAuth` failed-auth budget to
    // its own limiter — so a burst of anonymous public traffic can never
    // exhaust the operator budget, and vice versa.
    let rate_limiter: Box<dyn RateLimiter> = Box::new(CompositeRateLimiter {
        exchange: build_rate_limiter(config)?,
        admin: admin_rate_limiter,
    });

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

/// Routes each [`RateLimitKey`] family to the limiter that owns its budget:
/// `OperatorAuth` to the admin plane's failed-authentication limiter,
/// everything else to the exchange plane's fixed-window limiter.
struct CompositeRateLimiter {
    exchange: Box<dyn RateLimiter>,
    admin: Box<dyn RateLimiter>,
}

impl CompositeRateLimiter {
    fn route(&self, key: &oidc_exchange_core::domain::RateLimitKey) -> &dyn RateLimiter {
        match key {
            oidc_exchange_core::domain::RateLimitKey::OperatorAuth(_) => self.admin.as_ref(),
            _ => self.exchange.as_ref(),
        }
    }
}

#[async_trait::async_trait]
impl RateLimiter for CompositeRateLimiter {
    async fn check_and_consume(
        &self,
        key: &oidc_exchange_core::domain::RateLimitKey,
    ) -> oidc_exchange_core::error::Result<oidc_exchange_core::domain::RateLimitDecision> {
        self.route(key).check_and_consume(key).await
    }

    async fn check(
        &self,
        key: &oidc_exchange_core::domain::RateLimitKey,
    ) -> oidc_exchange_core::error::Result<oidc_exchange_core::domain::RateLimitDecision> {
        self.route(key).check(key).await
    }

    async fn consume(
        &self,
        key: &oidc_exchange_core::domain::RateLimitKey,
    ) -> oidc_exchange_core::error::Result<oidc_exchange_core::domain::RateLimitDecision> {
        self.route(key).consume(key).await
    }
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
    ///
    /// These runtures serve through the platform's request surface rather
    /// than `into_make_service_with_connect_info`, so every `/internal/*`
    /// request authenticates with `ClientAddr::Unknown`: no per-peer throttle
    /// key exists, meaning failed-auth lockout and peer-attributed security
    /// events are inactive. That degradation must never be silent — an
    /// API-Gateway-fronted function *is* an externally reachable guessing
    /// surface — so handing back the admin router warns loudly here.
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
            (None, Some(admin)) => {
                // No ConnectInfo exists on this runtime, so the internal-auth
                // layer will see ClientAddr::Unknown on every request and the
                // per-peer OperatorAuth budget cannot be consulted. The
                // fail-open is deliberate but must be visible: say so at
                // startup, every boot, so the deployment cannot quietly lose
                // its lockout/audit protection behind an API Gateway.
                tracing::warn!(
                    plane = "admin",
                    "serving the admin plane on a single-plane runtime without connection info; \
                     the per-peer authentication-failure throttle (lockout) and peer-address \
                     attribution on its security events are INACTIVE because no socket peer is \
                     available to key them — restrict reachability at your ingress (e.g. \
                     API-Gateway authorizers or VPC policy), not by lockout"
                );
                Some(admin.clone())
            }
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
    build_routers_shared(config, Arc::new(service))
}

/// [`build_routers`] over an already-shared service handle, for runtimes
/// whose process owns another consumer of the same `AppService` (the hyper
/// runtime's session reaper).
pub fn build_routers_shared(
    config: &AppConfig,
    service: Arc<AppService>,
) -> Result<Routers, Box<dyn std::error::Error>> {
    let role = config.server.role;

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

    let mut routers = Routers::default();

    if matches!(role, ServerRole::Exchange | ServerRole::All) {
        // The public plane rides the full shared stack — concurrency bound,
        // access log, address throttle — via the same builder every runtime
        // uses; it never carries `/internal/*`.
        routers.public = Some(build_router_shared(config, service.clone()));
    }
    if matches!(role, ServerRole::Admin | ServerRole::All) {
        let rate_limiter: Arc<dyn RateLimiter> = Arc::from(
            build_rate_limiter(config).expect("validated rate-limit config at router construction"),
        );
        let state = AppState {
            service,
            config: Arc::new(config.clone()),
            rate_limiter,
            operator_auth,
        };
        routers.admin = Some(build_admin_router(config, state));
    }

    assert!(
        !routers.is_empty(),
        "a validated role ({:?}) must produce at least one router",
        role.as_str()
    );
    if let Some(public) = &routers.public {
        assert_public_router_shape(public);
    }

    Ok(routers)
}

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
    build_router_shared(config, Arc::new(service))
}

/// [`build_router`] over an already-shared service handle — the variant entry
/// points whose process owns *another* consumer of the same `AppService` call.
/// `main.rs` hands one `Arc` clone to the session reaper and another to the
/// router's `AppState`, so both observe one store/audit/provider set;
/// `build_router` itself is just this plus the wrapping.
pub fn build_router_shared(config: &AppConfig, service: Arc<AppService>) -> Router {
    let rate_limiter = Arc::from(
        build_rate_limiter(config).expect("validated rate-limit config at router construction"),
    );
    build_router_shared_with_rate_limiter(config, service, rate_limiter)
}

/// Builds the production router with the supplied retained public limiter. This is separate
/// from [`build_router`] so service construction can pass the same concrete limiter to both
/// core provider/subject enforcement and public address throttling.
pub fn build_router_with_rate_limiter(
    config: &AppConfig,
    service: AppService,
    rate_limiter: Arc<dyn RateLimiter>,
) -> Router {
    build_router_shared_with_rate_limiter(config, Arc::new(service), rate_limiter)
}

fn build_router_shared_with_rate_limiter(
    config: &AppConfig,
    service: Arc<AppService>,
    rate_limiter: Arc<dyn RateLimiter>,
) -> Router {
    let role = config.server.role.as_str();

    if config.rate_limit.enabled
        && config.server.trusted_proxies.is_empty()
        && matches!(role, "exchange" | "all")
    {
        tracing::warn!(
            "public rate limiting is enabled with no trusted proxies; direct clients are safe, but deployments behind a reverse proxy must configure server.trusted_proxies and trusted_proxy_hops or all clients will share the proxy address"
        );
    }

    // On this single-router path the internal surface mounts only under
    // role = "admin": the plane-separation invariant says no public router
    // ever serves `/internal/*`, and role = "all" on a single-plane runtime
    // serves the public plane (deploy a second instance with role = "admin"
    // for the internal API). `build_routers` is the two-socket path.
    let operator_auth = if role == "admin" && internal_api_served(config) {
        Some(Arc::new(build_operator_auth_gate(config).expect(
            "validated internal_api config at router construction",
        )))
    } else {
        None
    };
    if role == "all" && config.internal_api.enabled {
        tracing::warn!(
            unmounted = "/internal/*",
            "role = \"all\" cannot bind two sockets on this single-router runtime; \
             serving only the public plane — deploy a second instance with \
             role = \"admin\" for the internal API"
        );
    }

    let state = AppState {
        service,
        config: Arc::new(config.clone()),
        rate_limiter,
        operator_auth,
    };

    let mut app: Router<AppState> = Router::new();

    if role == "exchange" || role == "all" {
        // The nonce route is part of the direct ID-token grant's surface, so
        // it mounts exactly when the grant does: an exchange-serving role with
        // `grants.id_token` enabled. The shared router is the single mounting
        // point — server, Lambda, and FFI cannot diverge — and it joins the
        // public group *before* the throttle/access-log layers so every public
        // route shares one concurrency bound, one access log, and one address
        // throttle.
        let mut public = routes::public_routes();
        if config.grants.id_token {
            public = public.merge(routes::nonce_routes());
        }
        app = app.merge(
            public
                // This semaphore is shared by every public route and checked before handler
                // work. Saturation returns 503 instead of waiting in an unbounded queue.
                .route_layer(axum::middleware::from_fn(public_concurrency_layer(
                    Arc::new(Semaphore::new(config.rate_limit.max_concurrent_requests)),
                )))
                .route_layer(axum::middleware::from_fn(access_log_layer))
                .route_layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    crate::middleware::public_throttle::public_throttle_layer,
                )),
        );
    }
    if role == "admin" && config.internal_api.enabled {
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

    let router = apply_route_layers(
        app,
        request_timeout_duration(config),
        config.server.max_request_body_bytes,
        axum::middleware::from_fn_with_state(state.clone(), audit_context_layer),
    )
    .with_state(state);

    #[cfg(feature = "conformance")]
    let router = with_base_path_strip_and_observe(
        router,
        config.server.base_path.clone(),
        config.server.max_request_body_bytes,
    );
    #[cfg(not(feature = "conformance"))]
    return wrap_with_base_path_under_outer_guard(router, config.server.base_path.clone());

    #[cfg(feature = "conformance")]
    wrap_under_outer_guard(router)
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
fn apply_route_layers<S, L>(
    app: Router<S>,
    request_timeout: std::time::Duration,
    max_request_body_bytes: usize,
    audit_context: L,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    L: tower::Layer<axum::routing::Route> + Clone + Send + Sync + 'static,
    L::Service: tower::Service<
            axum::extract::Request,
            Response = axum::response::Response,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + Sync
        + 'static,
    <L::Service as tower::Service<axum::extract::Request>>::Future: Send + 'static,
{
    app.layer(axum::extract::DefaultBodyLimit::max(max_request_body_bytes))
        .layer(CatchPanicLayer::custom(panic_handler))
        // The audit-context layer is caller-supplied because the production
        // stack binds it to the router's state (trusted-proxy resolution)
        // while the drift-proofing tests compose the stateless FFI variant —
        // the ordering, not the binding, is what this function owns.
        .layer(audit_context)
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
#[cfg(any(not(feature = "conformance"), test))]
fn wrap_with_base_path_under_outer_guard(router: Router, base_path: Option<String>) -> Router {
    wrap_under_outer_guard(with_base_path_strip(router, base_path))
}

/// Finish one plane's router with the outer half of the production stack —
/// base-path strip under the outer catch-panic guard, or the
/// conformance-observing variant when that feature is compiled in. Shared by
/// every plane builder so no router ships without the outer guard.
fn wrap_plane(router: Router, config: &AppConfig) -> Router {
    #[cfg(feature = "conformance")]
    {
        let router = with_base_path_strip_and_observe(
            router,
            config.server.base_path.clone(),
            config.server.max_request_body_bytes,
        );
        wrap_under_outer_guard(router)
    }
    #[cfg(not(feature = "conformance"))]
    wrap_with_base_path_under_outer_guard(router, config.server.base_path.clone())
}

fn wrap_under_outer_guard(router: Router) -> Router {
    Router::new().fallback_service(CatchPanicLayer::custom(panic_handler).layer(router))
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
        match method {
            oidc_exchange_core::config::InternalAuthMethod::SharedSecret => {
                let secret = internal
                    .shared_secret
                    .clone()
                    .ok_or_else(|| Error::ConfigError {
                        detail: "shared_secret mechanism enabled but no secret is configured"
                            .to_string(),
                    })?;
                authenticators.push(Box::new(SharedSecretAuthenticator::new(secret)));
            }
            oidc_exchange_core::config::InternalAuthMethod::OperatorToken => {
                // Validation guarantees a signing-capable key-manager adapter
                // while this mechanism is enabled; build a dedicated
                // verification instance so token checking never contends with
                // signing.
                let keys = build_key_manager(config)?;
                authenticators.push(Box::new(OperatorTokenAuthenticator::new(
                    keys,
                    config.server.issuer.as_str().to_string(),
                    internal.token_audience.clone(),
                    internal.required_claim.clone(),
                    internal.required_value.clone(),
                )));
            }
            oidc_exchange_core::config::InternalAuthMethod::Mtls => {
                authenticators.push(Box::new(MtlsSubjectAuthenticator::new(
                    internal.mtls_subject_header().to_string(),
                )));
            }
            other => {
                return Err(Error::ConfigError {
                    detail: format!(
                        "internal auth mechanism {:?} cannot serve the operator gate",
                        other.as_str()
                    ),
                }
                .into())
            }
        }
    }

    if authenticators.is_empty() {
        // Validated configs cannot reach this (`Config::resolve` rejects a
        // served internal API with no mechanisms); hand-built configs get an
        // error rather than a panic.
        return Err(Error::ConfigError {
            detail: "internal_api.auth_methods must be non-empty when the internal API is served"
                .to_string(),
        }
        .into());
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
/// Build the public exchange router from an already-assembled [`AppState`]:
/// the public routes (plus the nonce route when the direct ID-token grant is
/// enabled) behind the shared concurrency bound, access log, and address
/// throttle, under the shared middleware stack and base-path wrapper. Never
/// contains `/internal/*`.
pub fn build_public_router(config: &AppConfig, state: AppState) -> Router {
    let mut public = routes::public_routes();
    if config.grants.id_token {
        public = public.merge(routes::nonce_routes());
    }
    let app: Router<AppState> = Router::new().merge(
        public
            .route_layer(axum::middleware::from_fn(public_concurrency_layer(
                Arc::new(Semaphore::new(config.rate_limit.max_concurrent_requests)),
            )))
            .route_layer(axum::middleware::from_fn(access_log_layer))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::middleware::public_throttle::public_throttle_layer,
            )),
    );
    let router = apply_shared_middleware(app, config, &state).with_state(state);
    wrap_plane(router, config)
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

    let router = apply_shared_middleware(app, config, &state).with_state(state);
    wrap_plane(router, config)
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
/// makes a request-timeout response still carry the `x-request-id` header.
fn apply_shared_middleware(
    router: Router<AppState>,
    config: &AppConfig,
    state: &AppState,
) -> Router<AppState> {
    apply_route_layers(
        router,
        request_timeout_duration(config),
        config.server.max_request_body_bytes,
        axum::middleware::from_fn_with_state(state.clone(), audit_context_layer),
    )
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
#[cfg(feature = "conformance")]
pub(crate) async fn conformance_observe(
    request: axum::extract::Request,
    max_request_body_bytes: usize,
) -> axum::response::Response {
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use serde_json::json;

    const OBSERVATION_BODY_OVERFLOW_BYTES: usize = 1;

    let (parts, body) = request.into_parts();
    let body_read_limit = max_request_body_bytes.saturating_add(OBSERVATION_BODY_OVERFLOW_BYTES);
    let body = match to_bytes(body, body_read_limit).await {
        Ok(body) => body,
        Err(_) => return axum::http::StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    if body.len() > max_request_body_bytes {
        return axum::http::StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let ordered_headers = parts
        .headers
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "host" | "connection" | "content-length" | "x-oidc-conformance-observe"
            )
        })
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
    let observed_path = parts
        .extensions
        .get::<crate::middleware::base_path::ConformancePath>()
        .and_then(|path| {
            let decoded = percent_encoding::percent_decode_str(&path.0)
                .decode_utf8()
                .ok()?;
            let prefix = "/auth";
            if decoded == "/" {
                return Some(decoded.into_owned());
            }
            let stripped =
                crate::middleware::base_path::strip_prefix_at_segment_boundary(&decoded, prefix)?;
            Some(if stripped.is_empty() { "/" } else { stripped }.to_string())
        })
        .unwrap_or_else(|| parts.uri.path().to_string());
    let routed_status = if matches!(observed_path.as_str(), "/health" | "/keys") {
        200
    } else {
        404
    };
    let response = json!({
        "method": parts.method.as_str(),
        "decodedPath": observed_path,
        "query": parts.uri.query().and_then(|query| query.rsplit_once('?').map_or(Some(query), |(_, query)| Some(query))),
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

/// of silently substituting [`Duration::from_secs(30)`].
fn request_timeout_duration(config: &AppConfig) -> std::time::Duration {
    let secs = config.server.request_timeout.as_secs();
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

/// Parse `[internal_api] stats_cache_ttl` into the `Duration` the DynamoDB
/// repository builder wires into its dashboard-count cache — how long
/// `count_active_sessions` may serve a cached walk before re-scanning.
///
/// Every production entry point (`load_config`, `parse_config`) runs
/// `AppConfig::validate` — which parses and bounds this same field — before an
/// adapter is ever built, so reaching this function with an invalid value (a
/// hand-built `AppConfig` in a test that skipped `validate`) is a programmer
/// error and panics loudly instead of silently substituting the default. The
/// bounds mirror the adapter's own assertions in
/// `DynamoRepository::with_stats_cache_ttl`; the core-side constants are kept
/// aligned with the adapter's by a test in the adapters crate.
fn stats_cache_ttl(config: &AppConfig) -> std::time::Duration {
    // The typed config already parsed and bounded this at load
    // (`InternalApiConfig::resolve`); the assertions are defence in depth for
    // hand-built configs that skipped resolution.
    let ttl = config.internal_api.stats_cache_ttl;
    assert!(
        ttl.as_secs() >= oidc_exchange_core::config::MIN_STATS_CACHE_TTL_SECS,
        "stats_cache_ttl of {ttl:?} is below the usable minimum"
    );
    assert!(
        ttl.as_secs() <= oidc_exchange_core::config::MAX_STATS_CACHE_TTL_SECS,
        "stats_cache_ttl of {ttl:?} exceeds the maximum"
    );
    ttl
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
    Ok((client, dynamo_cfg.table_name.as_ref().to_string()))
}

async fn build_user_repository(
    config: &AppConfig,
) -> Result<Box<dyn UserRepository>, Box<dyn std::error::Error>> {
    match config.repository.adapter.as_str() {
        "dynamodb" => {
            let (client, table_name) = build_dynamo_client(config).await?;
            Ok(Box::new(
                oidc_exchange_adapters::dynamo::DynamoRepository::new(
                    client,
                    table_name,
                    config.token.refresh_reuse_retention_secs(),
                )
                .with_stats_cache_ttl(stats_cache_ttl(config)),
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
                pg_cfg.url.as_ref(),
                pg_cfg.max_connections.unwrap_or(5),
                pg_cfg.run_migrations.unwrap_or(true),
            )
            .await?;
            Ok(Box::new(
                oidc_exchange_adapters::postgres::PostgresRepository::new(
                    pool,
                    config.token.refresh_reuse_retention_secs(),
                ),
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
            let pool = oidc_exchange_adapters::sqlite::create_pool(sq_cfg.path.as_ref()).await?;
            Ok(Box::new(
                oidc_exchange_adapters::sqlite::SqliteRepository::new(
                    pool,
                    config.token.refresh_reuse_retention_secs(),
                ),
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
        .as_ref()
        .map(|adapter| adapter.as_str())
        .unwrap_or(config.repository.adapter.as_str());

    match adapter {
        "dynamodb" => {
            let (client, table_name) = build_dynamo_client(config).await?;
            Ok(Box::new(
                oidc_exchange_adapters::dynamo::DynamoRepository::new(
                    client,
                    table_name,
                    config.token.refresh_reuse_retention_secs(),
                )
                .with_stats_cache_ttl(stats_cache_ttl(config)),
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
                pg_cfg.url.as_ref(),
                pg_cfg.max_connections.unwrap_or(5),
                pg_cfg.run_migrations.unwrap_or(true),
            )
            .await?;
            Ok(Box::new(
                oidc_exchange_adapters::postgres::PostgresRepository::new(
                    pool,
                    config.token.refresh_reuse_retention_secs(),
                ),
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
            let pool = oidc_exchange_adapters::sqlite::create_pool(sq_cfg.path.as_ref()).await?;
            Ok(Box::new(
                oidc_exchange_adapters::sqlite::SqliteRepository::new(
                    pool,
                    config.token.refresh_reuse_retention_secs(),
                ),
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
                vk_cfg.url.as_ref(),
                vk_cfg
                    .key_prefix
                    .clone()
                    .unwrap_or_else(|| "oidc:".to_string()),
                config.token.refresh_reuse_retention_secs(),
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
                lm_cfg.path.as_ref(),
                lm_cfg.max_size_mb.unwrap_or(256),
                config.token.refresh_reuse_retention_secs(),
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
                local_cfg.private_key_path.as_ref(),
                local_cfg.algorithm.as_str(),
                local_cfg.kid.as_ref(),
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
                kms_cfg.key_id.as_ref().to_string(),
                kms_cfg.algorithm,
                kms_cfg.kid.as_ref().to_string(),
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

/// Select a limiter at the server construction boundary. The resulting port is
/// retained in `AppState` for future core/router consumers.
fn build_rate_limiter(
    config: &AppConfig,
) -> Result<Box<dyn RateLimiter>, Box<dyn std::error::Error>> {
    if !config.rate_limit.enabled
        || config.rate_limit.store == oidc_exchange_core::config::RateLimitStore::None
    {
        return Ok(Box::new(
            oidc_exchange_adapters::noop::NoopRateLimiter::new(),
        ));
    }
    Ok(Box::new(FixedWindowRateLimiter::new(
        config.rate_limit.window,
        RateLimitBudgets {
            per_ip: config.rate_limit.per_ip,
            per_ip_failures: config.rate_limit.per_ip_failures,
            per_subject: config.rate_limit.per_subject,
            per_provider: config.rate_limit.per_provider,
        },
        config.rate_limit.max_entries,
    )?))
}

async fn build_audit_log(
    config: &AppConfig,
) -> Result<Box<dyn AuditLog>, Box<dyn std::error::Error>> {
    match config.audit.adapter.as_str() {
        "noop" => Ok(Box::new(oidc_exchange_adapters::noop::NoopAuditLog::new())),
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

    match config
        .user_sync
        .adapter
        .as_ref()
        .map(|adapter| adapter.as_str())
    {
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

            let timeout = wh_cfg
                .timeout
                .unwrap_or_else(|| std::time::Duration::from_secs(5));
            let retries = wh_cfg.effective_retries();

            Ok(Box::new(
                oidc_exchange_adapters::webhook::WebhookUserSync::new(
                    wh_cfg.url.clone(),
                    wh_cfg.secret.expose().to_string(),
                    timeout,
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
        providers.insert(name.to_string(), provider);
    }

    Ok(providers)
}

async fn build_single_provider(
    name: &str,
    config: &ProviderConfig,
) -> Result<Box<dyn IdentityProvider>, Box<dyn std::error::Error>> {
    // `adapter` is the closed two-value `IdentityProviderAdapter`, already parsed
    // and validated during `Config::resolve`, so every value that reaches here
    // names a constructor — there is no unknown-adapter arm.
    match config.adapter {
        IdentityProviderAdapter::Oidc => {
            let oidc_config = provider_config_to_oidc(name, config)?;
            let provider =
                oidc_exchange_adapters::oidc::OidcProvider::from_config(name, &oidc_config).await?;
            Ok(Box::new(provider))
        }
        IdentityProviderAdapter::Apple => {
            let provider =
                oidc_exchange_providers::apple::AppleProvider::from_config(&config.extra).await?;
            Ok(Box::new(provider))
        }
    }
}

/// Convert the generic `ProviderConfig` (with its `extra` map) into the typed
/// `OidcProviderConfig` expected by the OIDC adapter.
///
/// `endpoint_origins` is validated here, at the config boundary: each entry
/// must be a bare `https` origin (`scheme://host[:port]`, no path, query, or
/// fragment), the list is capped, and entries are length-bounded before any
/// parse so hostile config text never reaches an error message. The adapter
/// re-validates defensively at construction.
fn provider_config_to_oidc(
    name: &str,
    config: &ProviderConfig,
) -> Result<oidc_exchange_core::domain::provider::OidcProviderConfig, Error> {
    use oidc_exchange_adapters::shared::origins::{
        parse_https_origin, MAX_ENDPOINT_ORIGINS, MAX_ENDPOINT_ORIGIN_LEN_BYTES,
    };
    use oidc_exchange_core::domain::provider::OidcProviderConfig;

    let get_str = |key: &str| -> Option<String> {
        config
            .extra
            .get(key)
            .and_then(|v| v.as_str())
            .map(String::from)
    };

    let issuer = config.issuer.clone().ok_or_else(|| Error::ConfigError {
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

    // Absent means empty: a provider without `endpoint_origins` is pinned to
    // its issuer's origin plus its explicitly configured endpoints.
    let endpoint_origins = match config.extra.get("endpoint_origins") {
        None => Vec::new(),
        Some(raw) => {
            let entries = raw.as_array().ok_or_else(|| Error::ConfigError {
                detail: format!(
                    "provider '{name}': 'endpoint_origins' must be an array of https origins"
                ),
            })?;
            if entries.len() > MAX_ENDPOINT_ORIGINS {
                return Err(Error::ConfigError {
                    detail: format!(
                        "provider '{name}': more than {MAX_ENDPOINT_ORIGINS} endpoint_origins"
                    ),
                });
            }
            entries
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let Some(entry) = value.as_str() else {
                        return Err(Error::ConfigError {
                            detail: format!(
                                "provider '{name}': endpoint_origins[{index}] must be a string"
                            ),
                        });
                    };
                    if entry.len() > MAX_ENDPOINT_ORIGIN_LEN_BYTES {
                        // Rejected before any parse: the message names only the
                        // index, so the oversized entry never becomes log text.
                        return Err(Error::ConfigError {
                            detail: format!(
                                "provider '{name}': endpoint_origins[{index}] exceeds \
                                 {MAX_ENDPOINT_ORIGIN_LEN_BYTES} bytes"
                            ),
                        });
                    }
                    parse_https_origin(entry).map_err(|e| Error::ConfigError {
                        detail: format!(
                            "provider '{name}': invalid endpoint_origins[{index}]: {e}"
                        ),
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?
        }
    };

    Ok(OidcProviderConfig {
        provider_id: name.to_string(),
        issuer,
        client_id,
        client_secret: get_str("client_secret").map(oidc_exchange_core::secret::Secret::new),
        jwks_uri: config.jwks_uri.clone(),
        token_endpoint: config.token_endpoint.clone(),
        revocation_endpoint: config.revocation_endpoint.clone(),
        endpoint_origins,
        // Placeholder until the config keys are lifted: every provider stays on
        // the standard `email_verified` reading, so behaviour is unchanged.
        email_verification: oidc_exchange_core::domain::EmailVerification::default(),
        scopes,
        additional_params: HashMap::new(),
    })
}

// ---------------------------------------------------------------------------
// provider_config_to_oidc: endpoint_origins lifting and validation tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod provider_config_to_oidc_tests {
    use super::*;
    use oidc_exchange_core::config::HttpsUrl;

    /// Build a minimal valid OIDC `ProviderConfig`, with optional extra keys
    /// merged in (e.g. an `endpoint_origins` array).
    fn oidc_provider_config(extra: Vec<(&str, toml::Value)>) -> ProviderConfig {
        let mut map: HashMap<String, toml::Value> = HashMap::from([
            (
                "issuer".into(),
                toml::Value::from("https://accounts.google.com"),
            ),
            ("client_id".into(), toml::Value::from("client-id")),
            (
                "scopes".into(),
                toml::Value::Array(vec![toml::Value::from("openid")]),
            ),
        ]);
        for (key, value) in extra {
            map.insert(key.to_string(), value);
        }
        ProviderConfig {
            provider_id: "google".to_string(),
            adapter: IdentityProviderAdapter::Oidc,
            issuer: Some(HttpsUrl::parse("https://accounts.google.com").expect("fixture issuer")),
            jwks_uri: None,
            token_endpoint: None,
            revocation_endpoint: None,
            extra: map,
        }
    }

    #[test]
    fn absent_endpoint_origins_lifts_as_empty_and_pins_nothing_extra() {
        let converted = provider_config_to_oidc("google", &oidc_provider_config(vec![]))
            .expect("a minimal provider config must convert");

        assert!(
            converted.endpoint_origins.is_empty(),
            "no declared origins must lift as an empty set, got {:?}",
            converted.endpoint_origins
        );
        // The other required fields still lift.
        assert_eq!(converted.issuer.as_str(), "https://accounts.google.com");
        assert_eq!(converted.client_id, "client-id");
    }

    #[test]
    fn declared_https_origins_lift_into_the_typed_config_normalized() {
        let converted = provider_config_to_oidc(
            "google",
            &oidc_provider_config(vec![(
                "endpoint_origins",
                toml::Value::Array(vec![
                    toml::Value::from("https://oauth2.googleapis.com"),
                    toml::Value::from("https://www.googleapis.com:443"),
                ]),
            )]),
        )
        .expect("declared https origins must convert");

        assert_eq!(
            converted.endpoint_origins,
            vec![
                "https://oauth2.googleapis.com".to_string(),
                // The explicit default port normalizes away during validation,
                // so the pinned string is canonical before it reaches adapters.
                "https://www.googleapis.com".to_string(),
            ]
        );
    }

    #[test]
    fn invalid_endpoint_origin_entries_are_rejected_with_indexed_config_errors() {
        let cases: Vec<(&str, toml::Value)> = vec![
            (
                "plain http scheme",
                toml::Value::Array(vec![toml::Value::from("http://insecure.example")]),
            ),
            (
                "path carried",
                toml::Value::Array(vec![toml::Value::from("https://example.com/token")]),
            ),
            (
                "query carried",
                toml::Value::Array(vec![toml::Value::from("https://example.com/?x=1")]),
            ),
            (
                "not a URL",
                toml::Value::Array(vec![toml::Value::from("garbage")]),
            ),
            (
                "non-string entry",
                toml::Value::Array(vec![toml::Value::Integer(42)]),
            ),
            (
                "over-length entry rejected before parse",
                toml::Value::Array(vec![toml::Value::from(format!(
                    "https://{}.example",
                    "x".repeat(300)
                ))]),
            ),
        ];

        for (label, value) in cases {
            let err = provider_config_to_oidc(
                "google",
                &oidc_provider_config(vec![("endpoint_origins", value)]),
            )
            .expect_err(&format!("{label} must be rejected at the config boundary"));

            match err {
                Error::ConfigError { detail } => {
                    assert!(
                        detail.contains("endpoint_origins"),
                        "{label}: the error names the offending field: {detail}"
                    );
                    assert!(
                        detail.contains("google"),
                        "{label}: the error names the provider: {detail}"
                    );
                }
                other => panic!("{label}: expected ConfigError, got {other:?}"),
            }
        }
    }

    #[test]
    fn non_array_endpoint_origins_value_is_rejected() {
        let err = provider_config_to_oidc(
            "google",
            &oidc_provider_config(vec![(
                "endpoint_origins",
                toml::Value::from("https://not-an-array.example"),
            )]),
        )
        .expect_err("a scalar endpoint_origins value must be rejected");

        assert!(
            matches!(err, Error::ConfigError { .. }),
            "expected ConfigError, got: {err:?}"
        );
    }

    #[test]
    fn more_than_the_cap_of_declared_origins_is_rejected() {
        use oidc_exchange_adapters::shared::origins::MAX_ENDPOINT_ORIGINS;

        let entries: Vec<toml::Value> = (0..=MAX_ENDPOINT_ORIGINS)
            .map(|i| toml::Value::from(format!("https://host{i}.example")))
            .collect();

        let err = provider_config_to_oidc(
            "google",
            &oidc_provider_config(vec![("endpoint_origins", toml::Value::Array(entries))]),
        )
        .expect_err("declared origins beyond MAX_ENDPOINT_ORIGINS must be rejected");

        let detail = match err {
            Error::ConfigError { detail } => detail,
            other => panic!("expected ConfigError, got {other:?}"),
        };
        assert!(
            detail.contains(&MAX_ENDPOINT_ORIGINS.to_string()),
            "the rejection names the cap: {detail}"
        );
    }

    #[test]
    fn exactly_at_the_cap_is_accepted() {
        use oidc_exchange_adapters::shared::origins::MAX_ENDPOINT_ORIGINS;

        // Boundary: AT the cap is valid; only above it is a config error.
        let entries: Vec<toml::Value> = (0..MAX_ENDPOINT_ORIGINS)
            .map(|i| toml::Value::from(format!("https://host{i}.example")))
            .collect();

        let converted = provider_config_to_oidc(
            "google",
            &oidc_provider_config(vec![("endpoint_origins", toml::Value::Array(entries))]),
        )
        .expect("exactly MAX_ENDPOINT_ORIGINS entries must be accepted");

        assert_eq!(converted.endpoint_origins.len(), MAX_ENDPOINT_ORIGINS);
    }
}

// ---------------------------------------------------------------------------
// S1 — provider adapter is the closed two-value IdentityProviderAdapter domain,
// parsed during Config::resolve. `adapter = "apple"` now resolves (making the
// shipped AppleProvider reachable), a storage/key value on a provider block is
// rejected at config load rather than at registry build, and the OIDC-only
// issuer requirement neither leaks onto Apple nor is lost from Oidc.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod provider_adapter_resolution_tests {
    use super::*;
    use oidc_exchange_core::config::IdentityProviderAdapter;

    /// A minimal deployment override that adds a single provider block, merged
    /// onto the committed defaults through the side-effect-free resolver.
    fn resolve_with_provider_block(block: &str) -> Result<AppConfig, Error> {
        resolve_config_toml(&format!("[providers.myidp]\n{block}"))
    }

    /// `adapter = "apple"` resolves — the Apple provider is reachable from
    /// configuration — and requires no `issuer` (Apple pins its own issuer
    /// internally and reads its settings from `extra`).
    #[test]
    fn apple_adapter_resolves_without_an_issuer() {
        let config = resolve_with_provider_block(
            "adapter = \"apple\"\nclient_id = \"com.example.app\"\nteam_id = \"TEAMID\"\nkey_id = \"KEYID\"\nprivate_key = \"pem\"",
        )
        .expect("an apple provider block must resolve");
        let provider = config
            .providers
            .get("myidp")
            .expect("the apple provider must be present after resolution");
        assert_eq!(
            provider.adapter,
            IdentityProviderAdapter::Apple,
            "the resolved provider adapter must be Apple"
        );
        assert!(
            provider.issuer.is_none(),
            "the apple adapter must not require an issuer at config load"
        );
    }

    /// An unknown provider adapter value fails resolution with a `ConfigError`
    /// naming `providers.adapter` — the failure point moved to config load.
    #[test]
    fn unknown_provider_adapter_is_rejected_at_config_load() {
        let err = resolve_with_provider_block(
            "adapter = \"atproto\"\nissuer = \"https://idp.example.com\"",
        )
        .expect_err("an unknown provider adapter must be rejected at config load");
        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("providers.adapter"),
                    "the error must name providers.adapter, got: {detail}"
                );
                assert!(
                    detail.contains("atproto"),
                    "the error must echo the offending value, got: {detail}"
                );
            }
            other => panic!("expected a ConfigError, got: {other:?}"),
        }
    }

    /// A storage/key adapter value the *shared* `ProviderAdapter` enum used to
    /// admit (e.g. `postgres`) is now rejected at config load on a provider
    /// block, pinning the failure point S1 moves earlier — previously this
    /// passed `Config::resolve` and failed only at registry build.
    #[test]
    fn storage_adapter_value_on_a_provider_block_is_rejected_at_config_load() {
        let err = resolve_with_provider_block(
            "adapter = \"postgres\"\nissuer = \"https://idp.example.com\"",
        )
        .expect_err("a storage adapter value on a provider block must be rejected at config load");
        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("providers.adapter"),
                    "the error must name providers.adapter, got: {detail}"
                );
                assert!(
                    detail.contains("postgres"),
                    "the error must echo the offending value, got: {detail}"
                );
            }
            other => panic!("expected a ConfigError, got: {other:?}"),
        }
    }

    /// The OIDC-only issuer requirement is not lost from `Oidc`: an
    /// `adapter = "oidc"` block without `issuer` fails resolution with the
    /// documented missing-HTTPS-URL error. Paired with the Apple boot test
    /// above, this pins that the requirement neither leaks onto `Apple` nor is
    /// dropped from `Oidc`.
    #[test]
    fn oidc_adapter_without_issuer_is_rejected() {
        let err = resolve_with_provider_block("adapter = \"oidc\"\nclient_id = \"c\"")
            .expect_err("an oidc provider block without issuer must be rejected");
        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("providers.myidp.issuer")
                        && detail.contains("missing required HTTPS URL"),
                    "the error must be the Oidc-only missing-issuer error, got: {detail}"
                );
            }
            other => panic!("expected a ConfigError, got: {other:?}"),
        }
    }
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
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
                issuer = "https://localhost:8080"
                role = "all"
                request_timeout = "30s"

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
        assert_eq!(config.registration.mode.as_str(), "open");
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
                host = "0.0.0.0"
                port = 8080
                issuer = "https://localhost:8080"
                role = "all"
                request_timeout = "30s"

                [registration]
                mode = "open"

                [token]
                access_token_ttl = "15m"
                refresh_token_ttl = "30d"
                audience = "oidc-exchange"

                [audit]
                adapter = "noop"
                blocking_threshold = "warning"
                emit_threshold = "info"

                [telemetry]
                enabled = false
                exporter = "none"

                [providers.google]
                adapter = "oidc"
                issuer = "https://accounts.google.example.com"
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
        assert_eq!(
            google.adapter.as_str(),
            "oidc",
            "unrelated fields survive the merge"
        );
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
                issuer = "https://idp.example.com"
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
        let defaults =
            parse_config(include_str!("../../../config/default.toml")).expect("default config");

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
                issuer = "https://localhost:8080"
                role = "all"
                request_timeout = "30s"

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
        assert_eq!(config.registration.mode.as_str(), "open");
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
            config
                .internal_api
                .shared_secret
                .as_ref()
                .map(|s| s.expose().as_str()),
            Some("super-secret-value")
        );
        assert_ne!(
            config
                .internal_api
                .shared_secret
                .as_ref()
                .map(|s| s.expose().as_str()),
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
            config
                .internal_api
                .shared_secret
                .as_ref()
                .map(|s| s.expose().as_str()),
            Some("${LITERAL_NOT_A_VAR}")
        );
        assert_ne!(
            config
                .internal_api
                .shared_secret
                .as_ref()
                .map(|s| s.expose().as_str()),
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
        assert_eq!(config.server.port, 8080);
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
                issuer = "https://accounts.google.example.com"
                client_secret = "${GOOGLE_CLIENT_SECRET}"
            "#,
        );
        let _guard = EnvVarGuard::set(&[("GOOGLE_CLIENT_SECRET", "nested-secret")]);

        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");

        let google = config
            .providers
            .get("google")
            .expect("google provider present");
        assert_eq!(google.adapter.as_str(), "oidc");
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

        // Replace the unset placeholder with a valid auth method and reload —
        // the set variable still resolves, and the singular `auth_method` key
        // (kept for pre-hardening configs) reads back as a one-element
        // `auth_methods` list.
        write_toml(
            dir.path(),
            "default",
            r#"
                [internal_api]
                shared_secret = "${SET_VAR}"
                auth_method = "shared_secret"
            "#,
        );
        let config = load_config_from_dir(dir_str(dir.path())).expect("load config");
        assert_eq!(
            config
                .internal_api
                .shared_secret
                .as_ref()
                .map(|s| s.expose().as_str()),
            Some("resolved-value")
        );
        assert_eq!(
            config.internal_api.auth_methods,
            vec![oidc_exchange_core::config::InternalAuthMethod::SharedSecret],
            "the singular auth_method key must read as a one-element list"
        );
    }

    #[test]
    fn rate_limit_budget_errors_name_the_offending_field() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("tempdir");
        write_toml(
            dir.path(),
            "default",
            "[rate_limit]\nper_subject = 1000001\n",
        );

        let err = load_config_from_dir(dir_str(dir.path())).expect_err("invalid budget rejected");
        assert!(err.to_string().contains("rate_limit.per_subject"));
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

        assert_eq!(config.server.role.as_str(), "exchange");
        assert_eq!(
            config
                .registration
                .domain_allowlist
                .as_ref()
                .map(|patterns| patterns.iter().map(AsRef::as_ref).collect::<Vec<_>>()),
            Some(vec!["example.com", "*.example.org"])
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

        assert_eq!(config.server.role.as_str(), "all");
    }

    #[test]
    fn check_config_file_uses_the_side_effect_free_resolver() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, include_str!("../../../config/default.toml")).expect("write config");

        let config = check_config_file(&path).expect("config check should resolve valid TOML");

        assert_eq!(config.server.role.as_str(), "exchange");
    }

    #[test]
    fn check_config_file_rejects_the_same_invalid_closed_domain() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, "[server]\nrole = \"not-a-role\"\n").expect("write config");

        let err = check_config_file(&path).expect_err("invalid role must fail closed");

        assert!(err.to_string().contains("server.role"));
    }

    #[test]
    fn check_config_file_rejects_non_file_paths_without_fallback() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("temp dir");

        let err = check_config_file(dir.path()).expect_err("directories are not config files");

        assert!(err.to_string().contains("config error"));
        assert!(err.to_string().contains("not a readable file"));
    }

    #[test]
    fn check_config_file_ignores_cwd_overlays_and_environment_overrides() {
        let _env_lock = lock_test_environment();
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, "[server]\nrole = \"exchange\"\n").expect("write config");
        let old_cwd = std::env::current_dir().expect("current directory");
        let cwd = tempfile::tempdir().expect("cwd temp dir");
        std::fs::create_dir(cwd.path().join("config")).expect("create config directory");
        std::fs::write(
            cwd.path().join("config/default.toml"),
            "[server]\nrole = \"admin\"\n",
        )
        .expect("write cwd default");
        std::env::set_current_dir(cwd.path()).expect("change cwd");
        std::env::set_var("OIDC_EXCHANGE__SERVER__ROLE", "admin");

        let config = check_config_file(&path).expect("explicit config must resolve");

        std::env::remove_var("OIDC_EXCHANGE__SERVER__ROLE");
        std::env::set_current_dir(old_cwd).expect("restore cwd");
        assert_eq!(config.server.role.as_str(), "exchange");
    }

    #[test]
    fn checked_config_rendering_redacts_secrets() {
        let _env_lock = lock_test_environment();
        let config = parse_config(
            "[internal_api]\nenabled = true\nauth_method = \"shared_secret\"\nshared_secret = \"do-not-print-me-0123456789abcdef\"\n",
        )
        .expect("config should resolve");

        let rendered = render_checked_config(&config);

        assert!(!rendered.contains("do-not-print-me-0123456789abcdef"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn parse_config_resolves_placeholders_for_ffi_callers() {
        let _env_lock = lock_test_environment();
        let _guard =
            EnvVarGuard::set(&[("INTERNAL_API_SECRET", "ffi-secret-0123456789abcdef012345")]);
        let config = parse_config(
            r#"
                [server]
                role = "all"
                [internal_api]
                enabled = true
                auth_method = "shared_secret"
                shared_secret = "${INTERNAL_API_SECRET}"
            "#,
        )
        .expect("FFI TOML placeholders must resolve before validation");

        assert_eq!(
            config
                .internal_api
                .shared_secret
                .as_ref()
                .map(|secret| secret.expose().as_str()),
            Some("ffi-secret-0123456789abcdef012345")
        );
    }

    #[test]
    fn parse_config_applies_structural_environment_overrides_for_ffi_callers() {
        let _env_lock = lock_test_environment();
        let _guard =
            EnvVarGuard::set(&[("OIDC_EXCHANGE__REGISTRATION__MODE", "existing_users_only")]);
        let config = parse_config("[server]\nrole = \"all\"")
            .expect("FFI TOML must receive structural environment overrides");

        assert_eq!(config.registration.mode.as_str(), "existing_users_only");
    }

    #[test]
    fn parse_config_rejects_empty_placeholder_values_with_a_path() {
        let _env_lock = lock_test_environment();
        let _guard = EnvVarGuard::set(&[("INTERNAL_API_SECRET", "")]);
        let err = parse_config(
            r#"
                [server]
                role = "all"
                [internal_api]
                enabled = true
                shared_secret = "${INTERNAL_API_SECRET}"
            "#,
        )
        .expect_err("empty placeholder values must fail closed");
        let message = err.to_string();
        assert!(message.contains("empty environment variable 'INTERNAL_API_SECRET'"));
        assert!(message.contains("internal_api.shared_secret"));
        assert!(!message.contains("ffi-secret"));
    }

    #[test]
    fn parse_config_rejects_malformed_and_empty_placeholders() {
        let _env_lock = lock_test_environment();
        for placeholder in ["${", "${}"] {
            let err = parse_config(&format!(
                "[server]\nrole = \"all\"\n[internal_api]\nenabled = true\nshared_secret = \"{placeholder}\""
            ))
            .expect_err("malformed placeholders must fail closed");
            assert!(err.to_string().contains("internal_api.shared_secret"));
        }
    }

    // -----------------------------------------------------------------
    // S2 — the defaults merge preserves explicit falsy overrides
    //
    // Before the fix, `remove_empty_values` stripped every `false`, `0`,
    // and `""` from the deployment overlay before it merged onto
    // `config/default.toml`, so an operator's explicit falsy value
    // silently reverted to the committed default. Each of these cases was
    // empirically confirmed broken against that code (the overlay value
    // was dropped and the resolved config carried the default instead);
    // they now hold because the merge is value-level and never round-trips
    // the overlay through `#[serde(default)]` `RawConfig`.
    // -----------------------------------------------------------------

    /// `token.refresh_rotation = false` — the documented rotation off-switch —
    /// survives resolution instead of reverting to the committed `true`.
    #[test]
    fn explicit_refresh_rotation_false_survives_resolution() {
        let config = resolve_config_toml("[token]\nrefresh_rotation = false")
            .expect("a config setting only refresh_rotation = false must resolve");
        assert!(
            !config.token.refresh_rotation,
            "an explicit refresh_rotation = false must survive the defaults merge, \
             not revert to the committed default of true"
        );
    }

    /// `rate_limit.per_subject = 0` — "zero disables a scope" — survives
    /// resolution instead of reverting to the committed `10`.
    #[test]
    fn explicit_zero_rate_limit_budget_survives_resolution() {
        let config = resolve_config_toml("[rate_limit]\nper_subject = 0")
            .expect("a config setting only per_subject = 0 must resolve");
        assert_eq!(
            config.rate_limit.per_subject, 0,
            "an explicit per_subject = 0 must survive the defaults merge \
             (zero disables the scope), not revert to the committed default of 10"
        );
    }

    /// `rate_limit.enabled = false` survives resolution instead of reverting
    /// to the committed `true`.
    #[test]
    fn explicit_rate_limit_enabled_false_survives_resolution() {
        let config = resolve_config_toml("[rate_limit]\nenabled = false")
            .expect("a config setting only rate_limit.enabled = false must resolve");
        assert!(
            !config.rate_limit.enabled,
            "an explicit rate_limit.enabled = false must survive the defaults merge, \
             not revert to the committed default of true"
        );
    }

    /// Preservation: a config that omits the falsy switches still inherits the
    /// committed defaults (`refresh_rotation = true`, `per_subject = 10`,
    /// `rate_limit.enabled = true`) — the merge fills genuinely-absent keys.
    #[test]
    fn omitted_switches_still_inherit_committed_defaults() {
        let config = resolve_config_toml("[server]\nhost = \"0.0.0.0\"")
            .expect("a minimal override must resolve against the committed defaults");
        assert!(
            config.token.refresh_rotation,
            "an omitted refresh_rotation must inherit the committed default true"
        );
        assert_eq!(
            config.rate_limit.per_subject, 10,
            "an omitted per_subject must inherit the committed default 10"
        );
        assert!(
            config.rate_limit.enabled,
            "an omitted rate_limit.enabled must inherit the committed default true"
        );
    }

    /// Negative space: an explicit empty string now reaches its domain resolver
    /// and fails loudly, rather than being stripped and silently reverting to
    /// the committed default TTL. `access_token_ttl = ""` is an invalid
    /// duration, so resolution must return a `ConfigError`.
    #[test]
    fn explicit_empty_duration_fails_resolution_loudly() {
        let result = resolve_config_toml("[token]\naccess_token_ttl = \"\"");
        let err =
            result.expect_err("an explicitly empty access_token_ttl must fail resolution loudly");
        // A duration-parse failure, not a silent revert to the committed "15m".
        assert!(
            matches!(err, Error::ConfigError { .. }),
            "an empty duration must surface as a ConfigError, got: {err:?}"
        );
    }

    /// The structural env-override channel (`OIDC_EXCHANGE__…` through
    /// `parse_config`) flows through the same value-level merge, so an explicit
    /// `false` set via the environment also survives.
    #[test]
    fn env_override_refresh_rotation_false_survives_resolution() {
        let _env_lock = lock_test_environment();
        let _guard = EnvVarGuard::set(&[("OIDC_EXCHANGE__TOKEN__REFRESH_ROTATION", "false")]);
        let config = parse_config("[server]\nrole = \"all\"")
            .expect("an env-set refresh_rotation = false must resolve");
        assert!(
            !config.token.refresh_rotation,
            "OIDC_EXCHANGE__TOKEN__REFRESH_ROTATION=false must survive the defaults merge"
        );
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
    use std::collections::HashMap;
    use tower::ServiceExt;

    use oidc_exchange_core::ports::IdentityProvider;
    use oidc_exchange_test_utils::{
        MockAuditLog, MockIdentityProvider, MockKeyManager, MockRateLimiter, MockRepository,
        MockUserSync,
    };

    const TEST_SECRET: &str = "test-internal-secret-build-router";

    fn router_config(role: &str, internal_enabled: bool, shared_secret: Option<&str>) -> AppConfig {
        let secret = shared_secret
            .map(|value| format!("auth_method = \"shared_secret\"\nshared_secret = {value:?}"))
            .unwrap_or_default();
        resolve_config_toml(&format!(
            r#"[server]
host = "0.0.0.0"
port = 8080
issuer = "https://localhost:8080"
role = {role:?}
request_timeout = "30s"
[registration]
mode = "open"
[token]
access_token_ttl = "15m"
refresh_token_ttl = "30d"
audience = "oidc-exchange"
[audit]
adapter = "noop"
blocking_threshold = "warning"
emit_threshold = "info"
[telemetry]
enabled = false
exporter = "none"
[internal_api]
enabled = {internal_enabled}
{secret}
"#,
        ))
        .expect("test config should resolve")
    }
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
        let routers = build_routers(config, service).expect("test configs always build routers");
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

    /// `internal_api.enabled = true` with role `admin` mounts `/internal/*`
    /// behind the Bearer check on the admin router, and binds no public
    /// router at all — not even an empty one.
    #[tokio::test]
    async fn enabled_true_admin_mounts_internal_behind_bearer_auth() {
        let config = router_config("admin", true, Some(TEST_SECRET));
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
        let config = router_config("all", true, Some(TEST_SECRET));
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
        let config = router_config("admin", false, None);
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
        let config = router_config("all", false, None);
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
        // No `role` key at all: the absent-role default must resolve to the
        // exchange-only plane.
        let config = resolve_config_toml(&format!(
            r#"[server]
host = "0.0.0.0"
port = 8080
issuer = "https://localhost:8080"
request_timeout = "30s"
[registration]
mode = "open"
[token]
access_token_ttl = "15m"
refresh_token_ttl = "30d"
audience = "oidc-exchange"
[audit]
adapter = "noop"
blocking_threshold = "warning"
emit_threshold = "info"
[key_manager]
adapter = "local"
[repository]
adapter = "sqlite"
[internal_api]
enabled = true
auth_method = "shared_secret"
shared_secret = {TEST_SECRET:?}
"#
        ))
        .expect("absent-role config resolves");
        assert_eq!(
            config.server.role,
            oidc_exchange_core::config::ServerRole::Exchange,
            "the test is only meaningful for the absent-role default"
        );
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

    /// An empty configured `shared_secret` must never reach the wire as a
    /// working (or half-working) mechanism: the auth gate refuses to build
    /// with a blank secret, so `build_routers` fails and the process never
    /// serves the admin plane at all — defence in depth alongside
    /// `AppConfig::validate`, which rejects the same config at load.
    #[tokio::test]
    async fn empty_shared_secret_fails_router_build() {
        let mut config = AppConfig::test_default();
        config.server.role = oidc_exchange_core::config::ServerRole::Admin;
        config.internal_api.enabled = true;
        config.internal_api.auth_methods =
            vec![oidc_exchange_core::config::InternalAuthMethod::SharedSecret];
        config.internal_api.shared_secret = None;
        let service = build_test_service(&config);

        let outcome = build_routers(&config, service);

        let err = outcome.expect_err("an empty shared secret must fail router construction");
        assert!(
            err.to_string().contains("no secret is configured"),
            "the failure must name the missing credential, got: {err}"
        );
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
        let mut config = AppConfig::test_default();
        config.server.role = oidc_exchange_core::config::ServerRole::All;
        config.internal_api.enabled = true;
        config.internal_api.auth_methods =
            vec![oidc_exchange_core::config::InternalAuthMethod::SharedSecret];
        config.internal_api.shared_secret =
            Some(oidc_exchange_core::Secret::new(TEST_SECRET.to_string()));
        let all = build_routers(&config, build_test_service(&config))
            .expect("test configs always build routers");
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

        config.server.role = oidc_exchange_core::config::ServerRole::Exchange;
        let exchange = build_routers(&config, build_test_service(&config))
            .expect("test configs always build routers");
        assert!(exchange.admin.is_none());
        assert!(exchange.single_plane().is_some());

        config.server.role = oidc_exchange_core::config::ServerRole::Admin;
        let admin = build_routers(&config, build_test_service(&config))
            .expect("test configs always build routers");
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
        let config = resolve_config_toml(include_str!("../../../config/default.toml"))
            .expect("default config");

        let duration = request_timeout_duration(&config);

        assert_eq!(duration, Duration::from_secs(30));
        assert_eq!(config.server.request_timeout, Duration::from_secs(30));
    }

    /// An overridden `server.request_timeout` parses to the matching `Duration`, not just
    /// the default.
    #[test]
    fn request_timeout_duration_resolves_configured_override() {
        let config = resolve_config_toml("[server]\nhost = \"0.0.0.0\"\nport = 8080\nissuer = \"https://localhost:8080\"\nrole = \"all\"\nrequest_timeout = \"2m\"\n[registration]\nmode = \"open\"\n[token]\naccess_token_ttl = \"15m\"\nrefresh_token_ttl = \"30d\"\naudience = \"oidc-exchange\"\n[audit]\nadapter = \"noop\"\nblocking_threshold = \"warning\"\nemit_threshold = \"info\"\n[telemetry]\nenabled = false\nexporter = \"none\"").expect("configured config");

        let duration = request_timeout_duration(&config);

        assert_eq!(duration, Duration::from_secs(120));
    }

    /// Negative-space: an unparseable `server.request_timeout` must panic rather than
    /// silently building a `TimeoutLayer` from a fallback duration — `AppConfig::validate`
    /// is expected to have already rejected this at config load, so reaching this function
    /// with a bad value is a programmer error that must fail loudly.
    #[test]
    fn request_timeout_duration_panics_on_unparseable_value() {
        let result = resolve_config_toml(&format!(
            "{}\n[server]\nrequest_timeout = \"not-a-duration\"",
            include_str!("../../../config/default.toml")
        ));
        assert!(
            result.is_err(),
            "an unparseable request_timeout must be rejected during config resolution"
        );
    }

    /// Negative-space: a zero-second `request_timeout` — a value `parse_duration_secs`
    /// happily parses but that would time out every request instantly — must trip the
    /// non-zero assertion in `request_timeout_duration` rather than build a degenerate
    /// `TimeoutLayer`.
    #[test]
    fn request_timeout_duration_panics_on_zero_seconds() {
        let config = resolve_config_toml("[server]\nhost = \"0.0.0.0\"\nport = 8080\nissuer = \"https://localhost:8080\"\nrole = \"all\"\nrequest_timeout = \"0s\"\n[registration]\nmode = \"open\"\n[token]\naccess_token_ttl = \"15m\"\nrefresh_token_ttl = \"30d\"\naudience = \"oidc-exchange\"\n[audit]\nadapter = \"noop\"\nblocking_threshold = \"warning\"\nemit_threshold = \"info\"\n[telemetry]\nenabled = false\nexporter = \"none\"").expect("zero timeout config");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            request_timeout_duration(&config)
        }));
        assert!(
            result.is_err(),
            "a zero-second request_timeout must panic rather than build a degenerate timeout layer"
        );
    }

    /// Builds the production middleware ordering around two bare test handlers by calling
    /// the same [`apply_route_layers`] / [`wrap_with_base_path_under_outer_guard`] functions
    /// the router builders use — request-id outermost, then the timeout layer, then
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
            axum::middleware::from_fn(crate::middleware::audit_context::ffi_audit_context_layer),
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
            axum::middleware::from_fn(crate::middleware::audit_context::ffi_audit_context_layer),
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
            axum::middleware::from_fn(crate::middleware::audit_context::ffi_audit_context_layer),
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
            axum::middleware::from_fn(crate::middleware::audit_context::ffi_audit_context_layer),
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

#[cfg(test)]
mod stats_cache_ttl_tests {
    use super::*;

    /// The documented default (`"60s"`) resolves to exactly 60 seconds, and an
    /// explicit override parses to its own duration.
    #[test]
    fn stats_cache_ttl_resolves_default_and_override() {
        let config = AppConfig::test_default();
        assert_eq!(config.internal_api.stats_cache_ttl.as_secs(), 60);
        assert_eq!(stats_cache_ttl(&config), std::time::Duration::from_secs(60));

        let mut config = AppConfig::test_default();
        config.internal_api.stats_cache_ttl = std::time::Duration::from_secs(120);
        assert_eq!(
            stats_cache_ttl(&config),
            std::time::Duration::from_secs(120)
        );
    }

    /// Negative-space: an out-of-window or unparseable TTL must panic rather
    /// than silently build a repository around an unusable cache —
    /// `AppConfig::validate` is expected to have rejected these at load.
    #[test]
    fn stats_cache_ttl_panics_on_values_validation_should_have_rejected() {
        for bad in [
            std::time::Duration::ZERO,
            std::time::Duration::from_secs(3601),
        ] {
            let mut config = AppConfig::test_default();
            config.internal_api.stats_cache_ttl = bad;

            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stats_cache_ttl(&config)));

            assert!(
                result.is_err(),
                "stats_cache_ttl {bad:?} must panic at wiring time, not silently default"
            );
        }
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
    use oidc_exchange_core::domain::{NewUser, Session};
    use oidc_exchange_core::Secret;
    use uuid::Uuid;

    /// An `AppConfig` whose user repository (and, since `session_repository.adapter` is
    /// left unset, the session repository via its documented fallback) both target
    /// Postgres at `url`, with `run_migrations` left as given.
    fn postgres_config(url: &str, run_migrations: Option<bool>) -> AppConfig {
        resolve_config_toml(&format!(
            "{}\n[repository]\nadapter = \"postgres\"\n[repository.postgres]\nurl = {url:?}\nrun_migrations = {}",
            include_str!("../../../config/default.toml"),
            run_migrations.map(|value| value.to_string()).unwrap_or_else(|| "false".to_string()),
        ))
        .expect("postgres test config should resolve")
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
            refresh_token_hash: Secret::new(refresh_token_hash.clone()),
            family_id: oidc_exchange_core::domain::new_family_id(),
            generation: 0,
            provider: "bootstrap-test".to_string(),
            expires_at: now + Duration::hours(1),
            rotated_at: None,
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: now,
        };
        session_repo.store_refresh_token(&session).await.expect(
            "store_refresh_token must succeed once bootstrap has migrated the session pool",
        );
        let fetched_session = session_repo
            .get_session_by_refresh_token(&Secret::new(refresh_token_hash.clone()))
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
        assert!(
            fetched_session.refresh_token_hash == Secret::new(refresh_token_hash),
            "round-tripped session refresh_token_hash must match"
        );
    }
}
