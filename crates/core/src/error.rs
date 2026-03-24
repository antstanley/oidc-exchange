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
