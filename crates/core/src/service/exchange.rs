use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::config::{AsciiDomainPattern, RegistrationMode};
use crate::domain::{
    is_valid_family_id, new_family_id, AuditFailure, AuditOutcome, AuthenticationKind, ClientAddr,
    NewUser, RateLimitDecision, RateLimitKey, SecurityEvent, Session, TokenResponse, UserStatus,
};
use crate::error::{Error, Result};
use crate::service::assertion::{AssertionBindError, AssertionContext};
use crate::service::AppService;

/// The typed form of the declared `grant_type`: one variant per exchange
/// grant, each owning that grant's required parameters as non-optional
/// fields. Which credential executes is a property of the type, so an
/// incoherent request — a code plus an ID token, or no credential at all —
/// is unrepresentable instead of a branch the service could take.
///
/// The refresh grant has its own input type, `RefreshRequest`; it is not a
/// variant here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExchangeCredential {
    /// Exchange an authorization `code` (plus its `redirect_uri`) at the
    /// provider, then validate the returned ID token.
    AuthorizationCode { code: String, redirect_uri: String },
    /// Validate a raw ID token assertion directly (e.g. Google Identity
    /// Services posting the credential it already holds).
    IdTokenAssertion { id_token: String },
}

#[derive(Clone)]
pub struct ExchangeRequest {
    pub credential: ExchangeCredential,
    pub provider: String,
    /// Provider access token co-issued with a directly-presented ID token,
    /// carried so the core's `at_hash` binding control can verify it. A
    /// bearer credential: never logged, never persisted, and dropped as soon
    /// as the assertion is bound.
    ///
    /// On the authorization-code path this field is ignored — the access
    /// token from `ProviderTokens` takes the same slot instead.
    pub provider_access_token: Option<String>,
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
fn matches_domain_allowlist(email: &str, allowlist: &[AsciiDomainPattern]) -> bool {
    let domain = match email.rsplit_once('@') {
        Some((_, domain)) => domain,
        None => return false,
    };

    let domain_lower = domain.to_lowercase();

    for entry in allowlist {
        let entry_lower = entry.as_str().to_lowercase();
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

/// Applies the verified-email and optional domain-allowlist policy to the
/// current identity assertion. This predicate is shared by new and existing
/// user paths so a tightened allowlist cannot be bypassed by a prior login.
fn registration_policy_reason(
    email: Option<&str>,
    email_verified: Option<bool>,
    allowlist: Option<&[AsciiDomainPattern]>,
) -> Option<&'static str> {
    let Some(email) = email else {
        return Some("verified email required for registration");
    };
    if email_verified != Some(true) {
        return Some("verified email required for registration");
    }

    if let Some(allowlist) = allowlist {
        if !matches_domain_allowlist(email, allowlist) {
            return Some("email domain not in allowlist");
        }
    }

    None
}

impl AppService {
    /// Exchanges provider credentials for local tokens and emits exactly one
    /// terminal security event for every result that reaches this core flow.
    /// Principal creation remains a separate state-change event emitted only
    /// by the successful creator, and assertion-binding rejections keep their
    /// detailed `ValidationFailed` record alongside the terminal event.
    pub async fn exchange(&self, request: ExchangeRequest) -> Result<TokenResponse> {
        let client_addr = request
            .ip_address
            .clone()
            .and_then(ClientAddr::asserted)
            .unwrap_or(ClientAddr::Unknown);
        let result = match self.exchange_inner(&request, &client_addr).await {
            Ok(success) => Ok(success),
            // A binding rejection's detailed `ValidationFailed` record *is*
            // its terminal security event — same class and severity the
            // generic mapping would produce, plus the failed control's name —
            // emitted here through the mandatory path so exactly one terminal
            // record exists per exchange.
            Err(ExchangeFlowError::Binding(rejection)) => {
                let mut event = SecurityEvent::AuthenticationFailed.into_audit_event(
                    AuditOutcome::Failure(AuditFailure::AuthenticationFailed),
                    None,
                    Some(request.provider.clone()),
                    client_addr,
                    request.user_agent.clone(),
                );
                event.detail.insert(
                    "check".to_string(),
                    serde_json::Value::String(rejection.check.to_string()),
                );
                event.detail.insert(
                    "reason".to_string(),
                    serde_json::Value::String(rejection.reason.clone()),
                );
                let emitted = self.emit_mandatory_audit_event(event).await;
                return match emitted {
                    Ok(()) => Err(Error::InvalidGrant {
                        reason: rejection.reason,
                    }),
                    Err(audit_error) => Err(audit_error),
                };
            }
            // Infrastructure failures are not client-attributable outcomes:
            // they surface as 5xx and are not recorded as authentication
            // failures (the failing store may be the audit dependency
            // itself). Infrastructure ≠ client fault.
            Err(ExchangeFlowError::Other(error @ Error::StoreError { .. })) => return Err(error),
            Err(ExchangeFlowError::Attributed { error, actor }) => {
                let (event, outcome, _) = exchange_terminal_event(&error);
                let emitted = self
                    .emit_security_event(
                        event,
                        outcome,
                        Some(actor),
                        Some(request.provider.clone()),
                        client_addr,
                        request.user_agent.clone(),
                    )
                    .await;
                return match emitted {
                    Ok(()) => Err(error),
                    Err(audit_error) => Err(audit_error),
                };
            }
            Err(ExchangeFlowError::Other(error)) => Err(error),
        };

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
                // The terminal success record is mandatory. Do not leave a
                // session live if the caller receives no tokens because
                // enforcing durability rejected the outcome.
                if self.config.audit.durability.is_enforce() {
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
    ) -> std::result::Result<ExchangeSuccess, ExchangeFlowError> {
        // 1. Resolve provider
        let provider =
            self.providers
                .get(&request.provider)
                .ok_or_else(|| Error::UnknownProvider {
                    provider: request.provider.clone(),
                })?;

        // Bound all outbound code and JWKS-backed validation work by provider.
        self.consume_limit(RateLimitKey::provider(request.provider.clone()), request)
            .await?;

        // 2. Get validated claims — the typed credential names the grant, so
        //    selection is an exhaustive match over variants rather than a
        //    check of optional-field presence. The match has no wildcard arm:
        //    a new credential variant must be handled here to compile. Both
        //    arms also produce the inputs the shared binding controls
        //    consume: the compact JWT as presented (replay-marker fallback
        //    key) and any access token that can anchor an `at_hash` check.
        let is_direct_grant = matches!(
            request.credential,
            ExchangeCredential::IdTokenAssertion { .. }
        );
        let (claims, compact_jwt, binding_access_token) = match &request.credential {
            ExchangeCredential::AuthorizationCode { code, redirect_uri } => {
                // Postcondition on the port contract: every code exchange must
                // return the ID token the flow validates next — an adapter
                // that returned an empty one would be a contract violation,
                // not a user-facing condition.
                let tokens = provider.exchange_code(code, redirect_uri).await?;
                assert!(
                    !tokens.id_token.is_empty(),
                    "exchange: IdentityProvider::exchange_code returned an empty id_token, violating the port contract"
                );
                let claims = provider.validate_id_token(&tokens.id_token).await?;
                // Redeeming the single-use code supplied this access token over
                // an authenticated back channel; it anchors the same `at_hash`
                // slot.
                (
                    claims,
                    tokens.id_token,
                    tokens.access_token.as_deref().map(str::to_string),
                )
            }
            ExchangeCredential::IdTokenAssertion { id_token } => {
                // Direct assertion: no code redemption happens on this path —
                // which is exactly why the grant/field binding is enforced at
                // the HTTP boundary before this type can be constructed.
                let claims = provider.validate_id_token(id_token).await?;
                (
                    claims,
                    id_token.clone(),
                    request.provider_access_token.as_deref().map(str::to_string),
                )
            }
        };
        // Postcondition on the port contract: downstream registration and
        // session storage are keyed on the subject, so an adapter returning
        // an empty one would corrupt identity rather than fail loudly.
        assert!(
            !claims.subject.is_empty(),
            "exchange: IdentityProvider::validate_id_token returned an empty subject, violating the port contract"
        );

        // A subject becomes available only after validated claims; retain
        // only its hash in the limiter key.
        self.consume_limit(
            RateLimitKey::subject(Some(&request.provider), &claims.subject),
            request,
        )
        .await?;

        // 3. Bind the assertion — lifetime ceiling, `azp`, applicable `at_hash`,
        // direct-grant nonce burn, then the single-use marker — exactly once,
        // before any user lookup or registration side effect can run.
        self.enforce_assertion_binding(
            &claims,
            &AssertionContext {
                provider_id: provider.provider_id(),
                client_id: provider.client_id(),
                access_token: binding_access_token.as_deref(),
                compact_jwt: &compact_jwt,
                require_nonce: is_direct_grant,
                max_assertion_secs: self.config.grants.max_assertion_lifetime.as_secs(),
            },
        )
        .await?;

        // 4. Look up user by external ID, applying registration policy for new users
        let user = match self
            .user_repo
            .get_user_by_external_id(&claims.subject, &request.provider)
            .await?
        {
            Some(user) => {
                // Denials return typed errors; the exchange wrapper emits the
                // single terminal security event for them, naming the known
                // principal as `actor`.
                if user.status != UserStatus::Active {
                    return Err(ExchangeFlowError::from(Error::UserSuspended {
                        user_id: user.id,
                    }));
                }
                if let Some(reason) = registration_policy_reason(
                    claims.email.as_deref(),
                    claims.email_verified,
                    self.config.registration.domain_allowlist.as_deref(),
                ) {
                    return Err(ExchangeFlowError::Attributed {
                        error: Error::AccessDenied {
                            reason: reason.to_string(),
                        },
                        actor: user.id,
                    });
                }
                user
            }
            None => {
                // Apply registration policy before creating a new user. The
                // exchange wrapper emits the terminal RegistrationDenied
                // security event for these typed errors.
                if let Some(reason) = registration_policy_reason(
                    claims.email.as_deref(),
                    claims.email_verified,
                    self.config.registration.domain_allowlist.as_deref(),
                ) {
                    return Err(ExchangeFlowError::from(Error::AccessDenied {
                        reason: reason.to_string(),
                    }));
                }

                match self.config.registration.mode {
                    RegistrationMode::Open => {}
                    RegistrationMode::ExistingUsersOnly => {
                        return Err(ExchangeFlowError::from(Error::AccessDenied {
                            reason: "registration is restricted to existing users only".to_string(),
                        }));
                    }
                }

                let new_user = NewUser {
                    external_id: claims.subject.clone(),
                    provider: request.provider.clone(),
                    email: claims.email.clone(),
                    display_name: claims.name.clone(),
                };
                match self.user_repo.create_user(&new_user).await {
                    Ok(created) => {
                        // Principal creation is a state-change security event
                        // emitted only by the successful creator, through the
                        // mandatory (durability-governed) path.
                        self.emit_security_event(
                            SecurityEvent::PrincipalCreated,
                            AuditOutcome::Success,
                            Some(created.id.clone()),
                            Some(request.provider.clone()),
                            client_addr.clone(),
                            request.user_agent.clone(),
                        )
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
                                    return Err(ExchangeFlowError::from(Error::UserSuspended {
                                        user_id: user.id,
                                    }));
                                }
                                if let Some(reason) = registration_policy_reason(
                                    claims.email.as_deref(),
                                    claims.email_verified,
                                    self.config.registration.domain_allowlist.as_deref(),
                                ) {
                                    return Err(ExchangeFlowError::Attributed {
                                        error: Error::AccessDenied {
                                            reason: reason.to_string(),
                                        },
                                        actor: user.id,
                                    });
                                }
                                user
                            }
                            None => {
                                // The winner's row is absent from a re-lookup
                                // immediately after a uniqueness conflict was
                                // reported — an adapter invariant violation.
                                // Surface a distinct error rather than
                                // panicking so the branch stays total.
                                return Err(ExchangeFlowError::from(Error::StoreError {
                                    detail: format!(
                                        "create_user conflicted for provider={} external_id={} but re-lookup found no user",
                                        request.provider, claims.subject
                                    ),
                                }));
                            }
                        }
                    }
                    Err(other) => return Err(ExchangeFlowError::from(other)),
                }
            }
        };

        // 5. Generate refresh token (256-bit random, base64url-encoded). Wrapped
        // immediately at mint so the raw token cannot be formatted anywhere downstream.
        let token_bytes: [u8; 32] = rand::random();
        let refresh_token = crate::secret::Secret::new(URL_SAFE_NO_PAD.encode(token_bytes));

        // 6. Hash refresh token with SHA-256 (hex-encoded); the digest is itself wrapped —
        // it is the session lookup key.
        let token_hash = crate::secret::Secret::new(hex::encode(Sha256::digest(
            refresh_token.expose().as_bytes(),
        )));

        // 7. Compute session expiry from config
        let refresh_ttl_secs = self.config.token.refresh_token_ttl.as_secs();
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
        // Postconditions on the assembled response: the body *is* the
        // credential, so reporting success with an empty access token or a
        // zero expiry would be a silent contract violation toward clients.
        assert!(
            !response.access_token.is_empty(),
            "exchange: success response must carry a non-empty access token"
        );
        assert!(
            response.expires_in > 0,
            "exchange: success response must carry a positive expires_in, got {}",
            response.expires_in
        );

        // 10. The terminal success audit is emitted by the `exchange`
        // wrapper, through the mandatory (durability-governed) path.
        Ok(ExchangeSuccess {
            refresh_token_hash: session.refresh_token_hash.clone(),
            user_id: Some(user.id.clone()),
            response,
        })
    }

    /// Consume one budgeted unit of the given limiter scope. A limiter
    /// infrastructure failure is logged and allowed through (fail-open for
    /// availability); a `Deny` maps to `TooManyRequests`, which the wrapper
    /// records as a `ThrottleExceeded` terminal event.
    async fn consume_limit(
        &self,
        key: Option<RateLimitKey>,
        request: &ExchangeRequest,
    ) -> Result<()> {
        let Some(key) = key else {
            tracing::error!(
                provider_length = request.provider.len(),
                "provider identifier exceeded the bounded rate-limit key size"
            );
            return Err(Error::InvalidRequest {
                reason: "invalid provider identifier".to_string(),
            });
        };
        match self.rate_limiter.check_and_consume(&key).await {
            Ok(RateLimitDecision::Allow) => Ok(()),
            Ok(RateLimitDecision::Deny { retry_after_secs }) => {
                Err(Error::TooManyRequests { retry_after_secs })
            }
            Err(error) => {
                tracing::warn!(error = %error, provider = %request.provider, "rate limiter unavailable; allowing exchange");
                Ok(())
            }
        }
    }

    /// Run the shared assertion-binding controls and translate their outcome:
    /// a control rejection surfaces to the `exchange` wrapper, which emits the
    /// single detail-enriched terminal `ValidationFailed` record and answers
    /// `InvalidGrant` (the OAuth error class for a bad assertion); a
    /// single-use store failure propagates untouched so it maps to `5xx`,
    /// never to a client fault.
    async fn enforce_assertion_binding(
        &self,
        claims: &crate::domain::IdentityClaims,
        ctx: &AssertionContext<'_>,
    ) -> std::result::Result<(), ExchangeFlowError> {
        match crate::service::assertion::bind(self.session_repo.as_ref(), claims, ctx).await {
            Ok(()) => Ok(()),
            Err(AssertionBindError::Store(err)) => Err(ExchangeFlowError::Other(err)),
            Err(AssertionBindError::Rejected(rejection)) => {
                Err(ExchangeFlowError::Binding(rejection))
            }
        }
    }
}

struct ExchangeSuccess {
    response: TokenResponse,
    user_id: Option<String>,
    refresh_token_hash: crate::secret::Secret<String>,
}

/// Internal error split for [`AppService::exchange_inner`]: a binding
/// rejection carries its control name so the wrapper can emit the one
/// detail-enriched terminal event; everything else flows to the generic
/// terminal mapping.
enum ExchangeFlowError {
    Binding(crate::service::assertion::AssertionRejection),
    /// A denial attributable to a known principal — the terminal event names
    /// them as `actor` (e.g. an existing user denied by registration policy).
    Attributed {
        error: Error,
        actor: String,
    },
    Other(Error),
}

impl From<Error> for ExchangeFlowError {
    fn from(error: Error) -> Self {
        Self::Other(error)
    }
}

/// Maps every exchange error to a fixed, safe terminal event classification.
/// It intentionally never consumes `Display`, as provider errors may include
/// upstream response bodies.
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

/// Parse a duration string like "15m", "1h", "30d" into seconds. Exposed for
/// unit testing via integration tests.
#[cfg(test)]
mod tests {
    use super::matches_domain_allowlist;
    use crate::config::AsciiDomainPattern;
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
        let allowlist = vec!["example.com".to_string()]
            .into_iter()
            .map(|entry| AsciiDomainPattern::parse(entry).unwrap())
            .collect::<Vec<_>>();
        assert!(matches_domain_allowlist("user@example.com", &allowlist));
        assert!(!matches_domain_allowlist("user@other.com", &allowlist));
        assert!(!matches_domain_allowlist(
            "user@sub.example.com",
            &allowlist
        ));
    }

    #[test]
    fn domain_allowlist_wildcard_match() {
        let allowlist = vec!["*.example.com".to_string()]
            .into_iter()
            .map(|entry| AsciiDomainPattern::parse(entry).unwrap())
            .collect::<Vec<_>>();
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
        let allowlist = vec!["Example.COM".to_string()]
            .into_iter()
            .map(|entry| AsciiDomainPattern::parse(entry).unwrap())
            .collect::<Vec<_>>();
        assert!(matches_domain_allowlist("user@example.com", &allowlist));
        assert!(matches_domain_allowlist("user@EXAMPLE.COM", &allowlist));
    }

    #[test]
    fn domain_allowlist_no_at_sign() {
        let allowlist = vec!["example.com".to_string()]
            .into_iter()
            .map(|entry| AsciiDomainPattern::parse(entry).unwrap())
            .collect::<Vec<_>>();
        assert!(!matches_domain_allowlist("noemailformat", &allowlist));
    }

    #[test]
    fn domain_allowlist_empty_list() {
        let allowlist: Vec<AsciiDomainPattern> = vec![];
        assert!(!matches_domain_allowlist("user@example.com", &allowlist));
    }

    #[test]
    fn domain_allowlist_multiple_entries() {
        let allowlist = vec!["example.com".to_string(), "*.acme.corp".to_string()]
            .into_iter()
            .map(|entry| AsciiDomainPattern::parse(entry).unwrap())
            .collect::<Vec<_>>();
        assert!(matches_domain_allowlist("user@example.com", &allowlist));
        assert!(matches_domain_allowlist("user@dev.acme.corp", &allowlist));
        assert!(!matches_domain_allowlist("user@other.org", &allowlist));
    }
}
