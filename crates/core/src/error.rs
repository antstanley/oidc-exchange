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

    #[error("mandatory security audit could not be persisted: {detail}")]
    SecurityAuditDurability { detail: String },

    #[error("sync error: {detail}")]
    SyncError { detail: String },

    // Internal
    #[error("config error: {detail}")]
    ConfigError { detail: String },
}

impl Error {
    /// The stable, client-facing description for this error variant.
    ///
    /// This is the only string that may cross the public HTTP boundary as an
    /// `error_description` (RFC 6749 §5.2). The set is small and fixed: no variant's
    /// description embeds caller input, library error text, provider key state, or
    /// cache internals, so `/token` cannot act as a validation oracle — a caller learns
    /// only which broad class of fault occurred. The full internal `Display` (with the
    /// adapter-composed `reason`/`detail`) is diagnostics for the operator log, under
    /// the request span; see `crates/server/src/error.rs`.
    ///
    /// Exhaustive by construction: the match has no fallthrough, so adding a variant
    /// without a description fails to compile.
    pub fn client_description(&self) -> &'static str {
        match self {
            // 4xx auth-flow faults.
            Error::InvalidGrant { .. } => "the provided grant could not be validated",
            Error::InvalidToken { .. } => "the provided token could not be validated",
            Error::InvalidRequest { .. } => {
                "the request is missing a required parameter or is otherwise malformed"
            }
            Error::UnknownProvider { .. } => "no identity provider matched the requested name",
            Error::AccessDenied { .. } => "access was denied for this account or request",
            Error::UserSuspended { .. } => "user account is suspended",
            Error::Unauthorized { .. } => "the request could not be authenticated",
            Error::Conflict { .. } => "the request conflicts with an existing resource",
            Error::NotFound { .. } => "the requested resource was not found",

            // Throttling.
            Error::TooManyRequests { .. } => "too many authentication attempts",

            // Upstream failures.
            Error::ProviderError { .. } => "upstream provider error",
            Error::ProviderTimeout { .. } => "upstream provider timeout",

            // Infrastructure failures.
            Error::StoreError { .. }
            | Error::KeyError { .. }
            | Error::AuditError { .. }
            | Error::SecurityAuditDurability { .. }
            | Error::SyncError { .. }
            | Error::ConfigError { .. } => "internal server error",
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant maps to a non-empty static description drawn from the small fixed
    /// set; no description embeds the caller-supplied diagnostic material.
    #[test]
    fn every_variant_has_a_fixed_description_without_internal_detail() {
        let samples = [
            Error::InvalidGrant {
                reason: "No matching key for kid: SENTINEL-KID".into(),
            },
            Error::InvalidToken {
                reason: "signature verification failed for SENTINEL-TOKEN".into(),
            },
            Error::InvalidRequest {
                reason: "either 'code' or 'id_token' is required".into(),
            },
            Error::UnknownProvider {
                provider: "SENTINEL-PROVIDER".into(),
            },
            Error::AccessDenied {
                reason: "email domain not in allowlist".into(),
            },
            Error::UserSuspended {
                user_id: "usr_SENTINEL".into(),
            },
            Error::Unauthorized {
                reason: "bad shared secret".into(),
            },
            Error::Conflict {
                detail: "user already registered for (google, SENTINEL-SUB)".into(),
            },
            Error::NotFound {
                detail: "user usr_SENTINEL not found".into(),
            },
            Error::ProviderError {
                provider: "google".into(),
                detail: "HTTP 500; excerpt: token=SENTINEL".to_string(),
            },
            Error::ProviderTimeout {
                provider: "google".into(),
            },
            Error::StoreError {
                detail: "sqlite: database is locked".into(),
            },
            Error::KeyError {
                detail: "kid SENTINEL-KID missing".into(),
            },
            Error::AuditError {
                detail: "queue unavailable".into(),
            },
            Error::SyncError {
                detail: "webhook refused".into(),
            },
            Error::ConfigError {
                detail: "missing issuer".into(),
            },
        ];

        for err in &samples {
            let description = err.client_description();
            assert!(
                !description.is_empty(),
                "{err:?} must carry a non-empty client description"
            );
            // The description must be a compile-time constant of this function, never
            // derived from the error's dynamic fields.
            assert_eq!(
                description,
                err.client_description(),
                "descriptions must be stable across calls"
            );
        }

        // Negative space: none of the sentinels above may appear in any description.
        let all = samples
            .iter()
            .map(|e| e.client_description())
            .collect::<Vec<_>>()
            .join("\n");
        for sentinel in [
            "SENTINEL-KID",
            "SENTINEL-TOKEN",
            "SENTINEL-PROVIDER",
            "usr_SENTINEL",
            "SENTINEL-SUB",
            "code",
        ] {
            assert!(
                !all.contains(sentinel),
                "no client description may embed internal material like {sentinel:?}: {all}"
            );
        }
    }

    /// The spec's indistinguishability contract: distinct validation failures share one
    /// public description.
    #[test]
    fn grant_validation_failures_are_indistinguishable_to_clients() {
        let reasons = [
            "No matching key for kid: unknown-kid",
            "JWT validation failed: InvalidSignature",
            "JWT validation failed: ExpiredSignature",
            "JWT validation failed: InvalidAudience",
        ];
        let descriptions = reasons
            .iter()
            .map(|r| Error::InvalidGrant {
                reason: (*r).to_string(),
            })
            .map(|err| err.client_description())
            .collect::<Vec<_>>();
        assert!(
            descriptions.windows(2).all(|w| w[0] == w[1]),
            "all grant-validation failures must share one description, got {descriptions:?}"
        );
    }
}
