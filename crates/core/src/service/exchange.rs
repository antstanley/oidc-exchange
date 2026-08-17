use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::domain::{
    AuditFailure, AuditOutcome, AuthenticationKind, ClientAddr, NewUser, RateLimitDecision,
    RateLimitKey, SecurityEvent, Session, TokenResponse, User, UserStatus,
};
use crate::error::{Error, Result};
use crate::service::{parse_duration_secs, AppService};

#[derive(Default)]
pub struct ExchangeRequest {
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub id_token: Option<String>,
    pub provider: String,
    /// Client IP address extracted by the server's audit-context middleware
    /// (e.g. from `X-Forwarded-For`). Stored on the resulting session.
    pub ip_address: Option<String>,
    /// Client `User-Agent` header, extracted by the server's audit-context
    /// middleware. Stored on the resulting session.
    pub user_agent: Option<String>,
    /// Client-supplied device identifier (`X-Device-Id`), extracted by the
    /// server's audit-context middleware. Stored on the resulting session.
    pub device_id: Option<String>,
}

/// Check whether an email's domain matches any entry in the allowlist.
fn matches_domain_allowlist(email: &str, allowlist: &[String]) -> bool {
    let Some((_, domain)) = email.rsplit_once('@') else {
        return false;
    };
    let domain = domain.to_lowercase();

    allowlist.iter().any(|entry| {
        let entry = entry.to_lowercase();
        match entry.strip_prefix('*') {
            Some(suffix) => domain.ends_with(suffix) && domain.len() > suffix.len(),
            None => domain == entry,
        }
    })
}

impl AppService {
    /// Exchanges provider credentials for local tokens and emits exactly one terminal security
    /// event for every result that reaches this core flow. Principal creation remains a separate
    /// state-change event emitted only by the successful creator.
    pub async fn exchange(&self, request: ExchangeRequest) -> Result<TokenResponse> {
        let client_addr = request
            .ip_address
            .clone()
            .and_then(ClientAddr::asserted)
            .unwrap_or(ClientAddr::Unknown);
        let result = self.exchange_inner(&request, &client_addr).await;

        let (event, outcome, actor) = match &result {
            Ok(success) => (
                SecurityEvent::AuthenticationSucceeded {
                    kind: AuthenticationKind::Exchange,
                },
                AuditOutcome::Success,
                success.user_id.clone(),
            ),
            Err(error) => exchange_terminal_event(error),
        };

        let emitted = self
            .emit_security_event(
                event,
                outcome,
                actor,
                Some(request.provider.clone()),
                client_addr,
                request.user_agent.clone(),
            )
            .await;

        match (result, emitted) {
            (Ok(success), Ok(())) => Ok(success.response),
            (Ok(success), Err(error)) => {
                // The terminal success record is mandatory. Do not leave a session live if the
                // caller receives no tokens because enforcing durability rejected the outcome.
                if self.config.audit.durability.eq_ignore_ascii_case("enforce") {
                    self.session_repo
                        .revoke_session(&success.refresh_token_hash)
                        .await?;
                }
                Err(error)
            }
            (Err(error), Ok(())) => Err(error),
            (Err(_), Err(audit_error)) => Err(audit_error),
        }
    }

    async fn exchange_inner(
        &self,
        request: &ExchangeRequest,
        client_addr: &ClientAddr,
    ) -> Result<ExchangeSuccess> {
        let provider =
            self.providers
                .get(&request.provider)
                .ok_or_else(|| Error::UnknownProvider {
                    provider: request.provider.clone(),
                })?;

        // Bound all outbound code and JWKS-backed validation work by provider.
        self.consume_limit(
            RateLimitKey::provider(request.provider.clone()),
            request,
            client_addr,
        )
        .await?;

        let claims = if let Some(id_token) = request.id_token.as_deref() {
            provider.validate_id_token(id_token).await?
        } else {
            let code = request
                .code
                .as_deref()
                .ok_or_else(|| Error::InvalidRequest {
                    reason: "either 'code' or 'id_token' is required".to_string(),
                })?;
            let redirect_uri =
                request
                    .redirect_uri
                    .as_deref()
                    .ok_or_else(|| Error::InvalidRequest {
                        reason: "redirect_uri is required for authorization_code grant".to_string(),
                    })?;
            let tokens = provider.exchange_code(code, redirect_uri).await?;
            provider.validate_id_token(&tokens.id_token).await?
        };

        // A subject becomes available only after validated claims; retain only its hash in the
        // limiter key.
        self.consume_limit(
            RateLimitKey::subject(Some(&request.provider), &claims.subject),
            request,
            client_addr,
        )
        .await?;

        let user = match self
            .user_repo
            .get_user_by_external_id(&claims.subject, &request.provider)
            .await?
        {
            Some(user) if user.status == UserStatus::Active => user,
            Some(user) => return Err(Error::UserSuspended { user_id: user.id }),
            None => self.register_user(request, client_addr, &claims).await?,
        };

        let token_bytes: [u8; 32] = rand::random();
        let refresh_token = URL_SAFE_NO_PAD.encode(token_bytes);
        let refresh_token_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));
        let refresh_ttl_secs = parse_duration_secs(&self.config.token.refresh_token_ttl)?;
        let session = Session {
            user_id: user.id.clone(),
            refresh_token_hash: refresh_token_hash.clone(),
            provider: request.provider.clone(),
            expires_at: Utc::now() + chrono::Duration::seconds(refresh_ttl_secs as i64),
            device_id: request.device_id.clone(),
            user_agent: request.user_agent.clone(),
            ip_address: request.ip_address.clone(),
            created_at: Utc::now(),
        };
        self.session_repo.store_refresh_token(&session).await?;

        let (access_token, expires_in) = self.build_access_token(&user).await?;
        Ok(ExchangeSuccess {
            response: TokenResponse {
                access_token,
                refresh_token: Some(refresh_token),
                token_type: "Bearer".to_string(),
                expires_in,
            },
            user_id: Some(user.id),
            refresh_token_hash,
        })
    }

    async fn consume_limit(
        &self,
        key: Option<RateLimitKey>,
        request: &ExchangeRequest,
        client_addr: &ClientAddr,
    ) -> Result<()> {
        let Some(key) = key else {
            return Ok(());
        };
        match self.rate_limiter.check_and_consume(&key).await {
            Ok(RateLimitDecision::Allow) => Ok(()),
            Ok(RateLimitDecision::Deny { retry_after_secs }) => {
                Err(Error::TooManyRequests { retry_after_secs })
            }
            Err(error) => {
                tracing::warn!(error = %error, provider = %request.provider, client_addr = ?client_addr, "rate limiter unavailable; allowing exchange");
                Ok(())
            }
        }
    }

    async fn register_user(
        &self,
        request: &ExchangeRequest,
        client_addr: &ClientAddr,
        claims: &crate::domain::IdentityClaims,
    ) -> Result<User> {
        if let Some(allowlist) = &self.config.registration.domain_allowlist {
            let allowed = claims.email_verified == Some(true)
                && claims
                    .email
                    .as_deref()
                    .is_some_and(|email| matches_domain_allowlist(email, allowlist));
            if !allowed {
                return Err(Error::AccessDenied {
                    reason: "registration policy denied identity".to_string(),
                });
            }
        }
        if self.config.registration.mode == "existing_users_only" {
            return Err(Error::AccessDenied {
                reason: "registration is restricted to existing users only".to_string(),
            });
        }

        let new_user = NewUser {
            external_id: claims.subject.clone(),
            provider: request.provider.clone(),
            email: claims.email.clone(),
            display_name: claims.name.clone(),
        };
        match self.user_repo.create_user(&new_user).await {
            Ok(created) => {
                self.emit_security_event(
                    SecurityEvent::PrincipalCreated,
                    AuditOutcome::Success,
                    Some(created.id.clone()),
                    Some(request.provider.clone()),
                    client_addr.clone(),
                    request.user_agent.clone(),
                )
                .await?;
                if let Err(error) = self.user_sync.notify_user_created(&created).await {
                    tracing::warn!(error = %error, user_id = %created.id, "user sync notify_user_created failed");
                }
                Ok(created)
            }
            Err(Error::Conflict { .. }) => {
                let user = self
                    .user_repo
                    .get_user_by_external_id(&claims.subject, &request.provider)
                    .await?
                    .ok_or_else(|| Error::StoreError {
                        detail: "create_user conflicted but re-lookup found no user".to_string(),
                    })?;
                if user.status != UserStatus::Active {
                    return Err(Error::UserSuspended { user_id: user.id });
                }
                Ok(user)
            }
            Err(error) => Err(error),
        }
    }
}

struct ExchangeSuccess {
    response: TokenResponse,
    user_id: Option<String>,
    refresh_token_hash: String,
}

/// Maps every exchange error to a fixed, safe terminal event classification. It intentionally
/// never consumes `Display`, as provider errors may include upstream response bodies.
fn exchange_terminal_event(error: &Error) -> (SecurityEvent, AuditOutcome, Option<String>) {
    let (event, failure) = match error {
        Error::AccessDenied { .. } => (
            SecurityEvent::RegistrationDenied,
            AuditFailure::RegistrationDenied,
        ),
        Error::UserSuspended { .. } => (
            SecurityEvent::PrincipalSuspended,
            AuditFailure::PrincipalSuspended,
        ),
        Error::ProviderError { .. } | Error::ProviderTimeout { .. } => (
            SecurityEvent::ProviderRejected,
            AuditFailure::ProviderRejected,
        ),
        Error::TooManyRequests { .. } => (
            SecurityEvent::ThrottleExceeded,
            AuditFailure::ThrottleExceeded,
        ),
        _ => (
            SecurityEvent::AuthenticationFailed,
            AuditFailure::AuthenticationFailed,
        ),
    };
    let actor = match error {
        Error::UserSuspended { user_id } => Some(user_id.clone()),
        _ => None,
    };
    (event, AuditOutcome::Failure(failure), actor)
}

#[cfg(test)]
mod tests {
    use super::matches_domain_allowlist;

    #[test]
    fn domain_allowlist_matches_exact_and_wildcard_domains() {
        assert!(matches_domain_allowlist(
            "user@example.com",
            &["example.com".into()]
        ));
        assert!(matches_domain_allowlist(
            "user@sub.example.com",
            &["*.example.com".into()]
        ));
        assert!(!matches_domain_allowlist(
            "user@example.com",
            &["*.example.com".into()]
        ));
        assert!(!matches_domain_allowlist(
            "invalid",
            &["example.com".into()]
        ));
    }
}
