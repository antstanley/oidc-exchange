use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::domain::{NewUser, Session, TokenResponse, UserStatus};
use crate::error::{Error, Result};
use crate::service::{parse_duration_secs, AppService};

pub struct ExchangeRequest {
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub id_token: Option<String>,
    pub provider: String,
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
            let code = request.code.as_deref().ok_or_else(|| Error::InvalidRequest {
                reason: "either 'code' or 'id_token' is required".to_string(),
            })?;
            let redirect_uri = request.redirect_uri.as_deref().ok_or_else(|| Error::InvalidRequest {
                reason: "redirect_uri is required for authorization_code grant".to_string(),
            })?;
            let tokens = provider.exchange_code(code, redirect_uri).await?;
            provider.validate_id_token(&tokens.id_token).await?
        };

        // 4. Look up user by external ID, applying registration policy for new users
        let user = match self.repo.get_user_by_external_id(&claims.subject).await? {
            Some(user) => {
                if user.status != UserStatus::Active {
                    return Err(Error::UserSuspended {
                        user_id: user.id,
                    });
                }
                user
            }
            None => {
                // Apply registration policy before creating a new user

                // Check domain allowlist if configured
                if let Some(ref allowlist) = self.config.registration.domain_allowlist {
                    match claims.email {
                        Some(ref email) => {
                            if !matches_domain_allowlist(email, allowlist) {
                                return Err(Error::AccessDenied {
                                    reason: "email domain not in allowlist".to_string(),
                                });
                            }
                        }
                        None => {
                            return Err(Error::AccessDenied {
                                reason: "email required when domain allowlist is configured"
                                    .to_string(),
                            });
                        }
                    }
                }

                // Check registration mode
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
                self.repo.create_user(&new_user).await?
            }
        };

        // 5. Generate refresh token (256-bit random, base64url-encoded)
        let token_bytes: [u8; 32] = rand::random();
        let refresh_token = URL_SAFE_NO_PAD.encode(token_bytes);

        // 6. Hash refresh token with SHA-256 (hex-encoded)
        let token_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));

        // 7. Compute session expiry from config
        let refresh_ttl_secs =
            parse_duration_secs(&self.config.token.refresh_token_ttl)?;
        let expires_at =
            Utc::now() + chrono::Duration::seconds(refresh_ttl_secs as i64);

        // 8. Store session
        let session = Session {
            user_id: user.id.clone(),
            refresh_token_hash: token_hash,
            provider: request.provider.clone(),
            expires_at,
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: Utc::now(),
        };
        self.repo.store_refresh_token(&session).await?;

        // 9. Build access token JWT (shared logic)
        let (access_token, access_ttl_secs) = self.build_access_token(&user).await?;

        Ok(TokenResponse {
            access_token,
            refresh_token: Some(refresh_token),
            token_type: "Bearer".to_string(),
            expires_in: access_ttl_secs,
        })
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
        assert!(!matches_domain_allowlist("user@sub.example.com", &allowlist));
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
        let allowlist = vec![
            "example.com".to_string(),
            "*.acme.corp".to_string(),
        ];
        assert!(matches_domain_allowlist("user@example.com", &allowlist));
        assert!(matches_domain_allowlist("user@dev.acme.corp", &allowlist));
        assert!(!matches_domain_allowlist("user@other.org", &allowlist));
    }
}
