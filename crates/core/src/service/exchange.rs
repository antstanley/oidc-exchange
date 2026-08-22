use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::domain::{
    is_valid_family_id, new_family_id, AuditEventType, AuditOutcome, AuditSeverity, NewUser,
    Session, TokenResponse, UserStatus,
};
use crate::error::{Error, Result};
use crate::service::{create_audit_event, parse_duration_secs, AppService};

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
///
/// Each entry can be:
/// - An exact domain, e.g. `example.com` -- matches only `example.com`.
/// - A wildcard, e.g. `*.example.com` -- matches any subdomain such as
///   `sub.example.com` or `a.b.example.com`, but NOT `example.com` itself.
fn matches_domain_allowlist(email: &str, allowlist: &[String]) -> bool {
    let domain = match email.rsplit_once('@') {
        Some((_, domain)) => domain,
        None => return false,
    };

    let domain_lower = domain.to_lowercase();

    for entry in allowlist {
        let entry_lower = entry.to_lowercase();
        if let Some(suffix) = entry_lower.strip_prefix('*') {
            // Wildcard entry like "*.example.com" -> suffix is ".example.com"
            // The email domain must end with the suffix AND be strictly longer
            // (i.e., there must be at least one subdomain level).
            if domain_lower.ends_with(&suffix) && domain_lower.len() > suffix.len() {
                return true;
            }
        } else {
            // Exact match
            if domain_lower == entry_lower {
                return true;
            }
        }
    }

    false
}

impl AppService {
    pub async fn exchange(&self, request: ExchangeRequest) -> Result<TokenResponse> {
        // 1. Resolve provider
        let provider =
            self.providers
                .get(&request.provider)
                .ok_or_else(|| Error::UnknownProvider {
                    provider: request.provider.clone(),
                })?;

        // 2. Get validated claims — either via code exchange or direct ID token
        let claims = if let Some(ref id_token) = request.id_token {
            // Direct ID token exchange (e.g., Google Sign-In SDK)
            provider.validate_id_token(id_token).await?
        } else {
            // Authorization code exchange
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

        // 4. Look up user by external ID, applying registration policy for new users
        let user = match self
            .user_repo
            .get_user_by_external_id(&claims.subject, &request.provider)
            .await?
        {
            Some(user) => {
                if user.status != UserStatus::Active {
                    self.emit_audit(create_audit_event(
                        AuditEventType::UserSuspended,
                        AuditSeverity::Warning,
                        AuditOutcome::Failure {
                            reason: format!("user status is {:?}, not active", user.status),
                        },
                        Some(user.id.clone()),
                        Some(request.provider.clone()),
                        request.ip_address.clone(),
                        request.user_agent.clone(),
                    ))
                    .await?;
                    return Err(Error::UserSuspended { user_id: user.id });
                }
                user
            }
            None => {
                // Apply registration policy before creating a new user

                // Check domain allowlist if configured
                if let Some(ref allowlist) = self.config.registration.domain_allowlist {
                    match claims.email {
                        Some(ref email) => {
                            // Reject unverified emails for allowlist matching
                            if claims.email_verified != Some(true) {
                                let reason =
                                    "verified email required when domain allowlist is configured"
                                        .to_string();
                                self.emit_audit(create_audit_event(
                                    AuditEventType::RegistrationDenied,
                                    AuditSeverity::Warning,
                                    AuditOutcome::Failure {
                                        reason: reason.clone(),
                                    },
                                    None,
                                    Some(request.provider.clone()),
                                    request.ip_address.clone(),
                                    request.user_agent.clone(),
                                ))
                                .await?;
                                return Err(Error::AccessDenied { reason });
                            }
                            if !matches_domain_allowlist(email, allowlist) {
                                let reason = "email domain not in allowlist".to_string();
                                self.emit_audit(create_audit_event(
                                    AuditEventType::RegistrationDenied,
                                    AuditSeverity::Warning,
                                    AuditOutcome::Failure {
                                        reason: reason.clone(),
                                    },
                                    None,
                                    Some(request.provider.clone()),
                                    request.ip_address.clone(),
                                    request.user_agent.clone(),
                                ))
                                .await?;
                                return Err(Error::AccessDenied { reason });
                            }
                        }
                        None => {
                            let reason =
                                "email required when domain allowlist is configured".to_string();
                            self.emit_audit(create_audit_event(
                                AuditEventType::RegistrationDenied,
                                AuditSeverity::Warning,
                                AuditOutcome::Failure {
                                    reason: reason.clone(),
                                },
                                None,
                                Some(request.provider.clone()),
                                request.ip_address.clone(),
                                request.user_agent.clone(),
                            ))
                            .await?;
                            return Err(Error::AccessDenied { reason });
                        }
                    }
                }

                // Check registration mode
                if self.config.registration.mode == "existing_users_only" {
                    let reason = "registration is restricted to existing users only".to_string();
                    self.emit_audit(create_audit_event(
                        AuditEventType::RegistrationDenied,
                        AuditSeverity::Warning,
                        AuditOutcome::Failure {
                            reason: reason.clone(),
                        },
                        None,
                        Some(request.provider.clone()),
                        request.ip_address.clone(),
                        request.user_agent.clone(),
                    ))
                    .await?;
                    return Err(Error::AccessDenied { reason });
                }

                let new_user = NewUser {
                    external_id: claims.subject.clone(),
                    provider: request.provider.clone(),
                    email: claims.email.clone(),
                    display_name: claims.name.clone(),
                };
                match self.user_repo.create_user(&new_user).await {
                    Ok(created) => {
                        self.emit_audit(create_audit_event(
                            AuditEventType::UserCreated,
                            AuditSeverity::Notice,
                            AuditOutcome::Success,
                            Some(created.id.clone()),
                            Some(request.provider.clone()),
                            request.ip_address.clone(),
                            request.user_agent.clone(),
                        ))
                        .await?;

                        // Best-effort JIT user-sync notify, mirroring
                        // `admin_create_user`: awaited (not spawned) so a
                        // fast follow-up `user.updated` cannot overtake this
                        // `user.created`, its result discarded, and a
                        // failure logged rather than failing the exchange.
                        if let Err(e) = self.user_sync.notify_user_created(&created).await {
                            tracing::warn!(error = %e, user_id = %created.id, "user sync notify_user_created failed");
                        }

                        created
                    }
                    Err(Error::Conflict { .. }) => {
                        // A concurrent first login for the same subject won the
                        // race and created the row first (JIT-registration
                        // race). Re-run the lookup and continue on the
                        // found-user branch instead of surfacing a 500; do not
                        // emit a second create or `UserCreated` audit event.
                        let winner = self
                            .user_repo
                            .get_user_by_external_id(&claims.subject, &request.provider)
                            .await?;
                        match winner {
                            Some(user) => {
                                // Postcondition: the re-lookup is keyed on
                                // the exact identity we just tried to
                                // create, so the row it returns must match —
                                // an adapter that returned a different
                                // identity's row would be a port-contract
                                // violation.
                                debug_assert_eq!(user.provider, request.provider);
                                debug_assert_eq!(user.external_id, claims.subject);
                                if user.status != UserStatus::Active {
                                    self.emit_audit(create_audit_event(
                                        AuditEventType::UserSuspended,
                                        AuditSeverity::Warning,
                                        AuditOutcome::Failure {
                                            reason: format!(
                                                "user status is {:?}, not active",
                                                user.status
                                            ),
                                        },
                                        Some(user.id.clone()),
                                        Some(request.provider.clone()),
                                        request.ip_address.clone(),
                                        request.user_agent.clone(),
                                    ))
                                    .await?;
                                    return Err(Error::UserSuspended { user_id: user.id });
                                }
                                user
                            }
                            None => {
                                // The winner's row is absent from a re-lookup
                                // immediately after a uniqueness conflict was
                                // reported — an adapter invariant violation.
                                // Surface a distinct error rather than
                                // panicking so the branch stays total.
                                return Err(Error::StoreError {
                                    detail: format!(
                                        "create_user conflicted for provider={} external_id={} but re-lookup found no user",
                                        request.provider, claims.subject
                                    ),
                                });
                            }
                        }
                    }
                    Err(other) => return Err(other),
                }
            }
        };

        // 5. Generate refresh token (256-bit random, base64url-encoded)
        let token_bytes: [u8; 32] = rand::random();
        let refresh_token = URL_SAFE_NO_PAD.encode(token_bytes);

        // 6. Hash refresh token with SHA-256 (hex-encoded)
        let token_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));

        // 7. Compute session expiry from config
        let refresh_ttl_secs = parse_duration_secs(&self.config.token.refresh_token_ttl)?;
        let expires_at = Utc::now() + chrono::Duration::seconds(refresh_ttl_secs as i64);

        // 8. Store session. Exchange mints the family: one `fam_` id shared by
        // every generation this sign-in ever rotates through, generation 0,
        // and no rotation timestamp yet.
        let family_id = new_family_id();
        assert!(
            is_valid_family_id(&family_id),
            "exchange: minted family id must be well-formed"
        );
        let session = Session {
            user_id: user.id.clone(),
            refresh_token_hash: token_hash,
            family_id,
            generation: 0,
            provider: request.provider.clone(),
            expires_at,
            rotated_at: None,
            device_id: request.device_id.clone(),
            user_agent: request.user_agent.clone(),
            ip_address: request.ip_address.clone(),
            created_at: Utc::now(),
        };
        self.session_repo.store_refresh_token(&session).await?;

        // 9. Build access token JWT (shared logic). The token's `sid` names
        // the family just minted, so it stays revocation-stable for the
        // token's whole validity however often the refresh token rotates.
        let (access_token, access_ttl_secs) =
            self.build_access_token(&user, &session.family_id).await?;

        let response = TokenResponse {
            access_token,
            refresh_token: Some(refresh_token),
            token_type: "Bearer".to_string(),
            expires_in: access_ttl_secs,
        };

        // 10. Audit the successful exchange, after the token response is
        // fully assembled.
        self.emit_audit(create_audit_event(
            AuditEventType::TokenExchange,
            AuditSeverity::Info,
            AuditOutcome::Success,
            Some(user.id.clone()),
            Some(request.provider.clone()),
            request.ip_address.clone(),
            request.user_agent.clone(),
        ))
        .await?;

        Ok(response)
    }
}

/// Parse a duration string like "15m", "1h", "30d" into seconds. Exposed for
/// unit testing via integration tests.
#[cfg(test)]
mod tests {
    use super::matches_domain_allowlist;
    use crate::service::parse_duration_secs;

    #[test]
    fn parse_duration_secs_works() {
        assert_eq!(parse_duration_secs("15m").unwrap(), 900);
        assert_eq!(parse_duration_secs("1h").unwrap(), 3600);
        assert_eq!(parse_duration_secs("30d").unwrap(), 2592000);
        assert_eq!(parse_duration_secs("60s").unwrap(), 60);
        assert!(parse_duration_secs("").is_err());
        assert!(parse_duration_secs("abc").is_err());
        assert!(parse_duration_secs("15x").is_err());
    }

    #[test]
    fn domain_allowlist_exact_match() {
        let allowlist = vec!["example.com".to_string()];
        assert!(matches_domain_allowlist("user@example.com", &allowlist));
        assert!(!matches_domain_allowlist("user@other.com", &allowlist));
        assert!(!matches_domain_allowlist(
            "user@sub.example.com",
            &allowlist
        ));
    }

    #[test]
    fn domain_allowlist_wildcard_match() {
        let allowlist = vec!["*.example.com".to_string()];
        assert!(matches_domain_allowlist("user@sub.example.com", &allowlist));
        assert!(matches_domain_allowlist("user@a.b.example.com", &allowlist));
        assert!(
            !matches_domain_allowlist("user@example.com", &allowlist),
            "wildcard requires at least one subdomain"
        );
        assert!(!matches_domain_allowlist("user@notexample.com", &allowlist));
    }

    #[test]
    fn domain_allowlist_case_insensitive() {
        let allowlist = vec!["Example.COM".to_string()];
        assert!(matches_domain_allowlist("user@example.com", &allowlist));
        assert!(matches_domain_allowlist("user@EXAMPLE.COM", &allowlist));
    }

    #[test]
    fn domain_allowlist_no_at_sign() {
        let allowlist = vec!["example.com".to_string()];
        assert!(!matches_domain_allowlist("noemailformat", &allowlist));
    }

    #[test]
    fn domain_allowlist_empty_list() {
        let allowlist: Vec<String> = vec![];
        assert!(!matches_domain_allowlist("user@example.com", &allowlist));
    }

    #[test]
    fn domain_allowlist_multiple_entries() {
        let allowlist = vec!["example.com".to_string(), "*.acme.corp".to_string()];
        assert!(matches_domain_allowlist("user@example.com", &allowlist));
        assert!(matches_domain_allowlist("user@dev.acme.corp", &allowlist));
        assert!(!matches_domain_allowlist("user@other.org", &allowlist));
    }
}
