use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    // Auth flow errors (4xx)
    #[error("invalid grant: {reason}")]
    InvalidGrant { reason: String },

    #[error("invalid token: {reason}")]
    InvalidToken { reason: String },

    #[error("invalid request: {reason}")]
    InvalidRequest { reason: String },

    #[error("unknown provider: {provider}")]
    UnknownProvider { provider: String },

    #[error("access denied: {reason}")]
    AccessDenied { reason: String },

    #[error("user suspended: {user_id}")]
    UserSuspended { user_id: String },

    #[error("unauthorized: {reason}")]
    Unauthorized { reason: String },

    #[error("conflict: {detail}")]
    Conflict { detail: String },

    #[error("not found: {detail}")]
    NotFound { detail: String },

    /// A bounded rate-limit budget is exhausted; the caller must back off for
    /// at least `retry_after_secs`.
    ///
    /// VENDORED SEAM (task 03): variant and mapping vendored from sibling PR
    /// #24 (`2026-08-05-audit_and_throttle_authentication_failures`); deleted
    /// in favour of #24's identical variant at merge time.
    #[error("too many requests; retry after {retry_after_secs} seconds")]
    TooManyRequests { retry_after_secs: u64 },

    // Provider errors (upstream)
    #[error("provider error ({provider}): {detail}")]
    ProviderError { provider: String, detail: String },

    #[error("provider timeout: {provider}")]
    ProviderTimeout { provider: String },

    // Infrastructure errors (5xx)
    #[error("store error: {detail}")]
    StoreError { detail: String },

    #[error("key error: {detail}")]
    KeyError { detail: String },

    #[error("audit error: {detail}")]
    AuditError { detail: String },

    #[error("sync error: {detail}")]
    SyncError { detail: String },

    // Internal
    #[error("config error: {detail}")]
    ConfigError { detail: String },
}

pub type Result<T> = std::result::Result<T, Error>;
