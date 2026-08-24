use axum::extract::{Extension, FromRequest, State};
use axum::response::IntoResponse;
use axum::{Form, Json};
use serde::Deserialize;

use crate::error::ApiError;
use crate::middleware::audit_context::AuditContext;
use crate::state::AppState;
use oidc_exchange_core::error::Error;
use oidc_exchange_core::service::exchange::{ExchangeCredential, ExchangeRequest};
use oidc_exchange_core::service::refresh::RefreshRequest;

/// The untrusted flattened wire shape of the `POST /token` body. It carries
/// every known parameter as an `Option` so the per-grant parser below can
/// classify them; `grant_type` itself stays a required `String` because the
/// extractor (`FromRequest` impl below) classifies its absence as
/// `invalid_request` instead of relaxing the wire field to `Option<String>`
/// — moving a required parameter to `Option` is exactly the shape of the
/// original grant-confusion regression.
#[derive(Debug, Deserialize)]
pub struct TokenForm {
    pub grant_type: String,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub provider: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    /// Provider access token co-issued with a directly-presented ID token.
    /// Bearer credential: bound once by the core's `at_hash` check, never
    /// logged or persisted.
    pub provider_access_token: Option<String>,
}

/// All-optional mirror of `TokenForm` used only inside the extractor: a body
/// without `grant_type` must deserialize successfully here so the extractor
/// can emit the specified `400 invalid_request` envelope instead of axum's
/// default `422` plain-text form rejection.
#[derive(Deserialize)]
struct RawTokenForm {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    provider: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    provider_access_token: Option<String>,
}

impl TryFrom<RawTokenForm> for TokenForm {
    type Error = ApiError;

    fn try_from(raw: RawTokenForm) -> Result<Self, Self::Error> {
        let grant_type = raw.grant_type.ok_or_else(|| {
            ApiError::Domain(Error::InvalidRequest {
                reason: "missing required parameter: grant_type".to_string(),
            })
        })?;
        Ok(TokenForm {
            grant_type,
            code: raw.code,
            redirect_uri: raw.redirect_uri,
            provider: raw.provider,
            refresh_token: raw.refresh_token,
            id_token: raw.id_token,
            provider_access_token: raw.provider_access_token,
        })
    }
}

impl<S> FromRequest<S> for TokenForm
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        // Any rejection from the form extraction itself (missing body, wrong
        // content type, malformed percent-encoding) is reported in the OAuth
        // error envelope too: the endpoint never answers with axum's
        // plain-text rejections. The detail stays server-side-generic — the
        // body is untrusted client input, not an internal failure.
        let Form(raw) = Form::<RawTokenForm>::from_request(req, state)
            .await
            .map_err(|_| {
                ApiError::Domain(Error::InvalidRequest {
                    reason: "malformed request body: expected a urlencoded form".to_string(),
                })
            })?;
        TokenForm::try_from(raw)
    }
}

/// The declared grant with its closed parameter set, produced only when the
/// form matches the per-grant table exactly. This is the type the handler
/// dispatches on — the declared `grant_type` alone selects the flow, and a
/// field belonging to a different grant can never reach the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenGrant {
    AuthorizationCode {
        provider: String,
        code: String,
        redirect_uri: String,
    },
    IdTokenAssertion {
        provider: String,
        id_token: String,
        /// Provider access token co-issued with the presented ID token,
        /// carried only so the core's `at_hash` binding control can verify
        /// it. Bearer credential: never logged, never persisted.
        provider_access_token: Option<String>,
    },
    RefreshToken {
        refresh_token: String,
    },
}

/// Per-grant parameter table, applied exactly as specified:
///
/// | `grant_type`        | required                            | rejected if present                          |
/// |---------------------|-------------------------------------|----------------------------------------------|
/// | `authorization_code`| `provider`, `code`, `redirect_uri`  | `id_token`, `refresh_token`, `provider_access_token` |
/// | `id_token`          | `provider`, `id_token` (+ optional `provider_access_token`) | `code`, `redirect_uri`, `refresh_token` |
/// | `refresh_token`     | `refresh_token`                     | `provider`, `code`, `redirect_uri`, `id_token`, `provider_access_token` |
///
/// Cross-grant fields are rejected (not ignored) before required members are
/// checked, because the closed-set rule is unconditional. Parameters outside
/// the known set are ignored by `TokenForm`'s deserialization (no
/// `deny_unknown_fields`). Present-but-unrecognized `grant_type` values —
/// including the empty string — are `unsupported_grant_type`.
impl TryFrom<TokenForm> for TokenGrant {
    type Error = ApiError;

    fn try_from(form: TokenForm) -> Result<Self, Self::Error> {
        const AUTHORIZATION_CODE: &str = "authorization_code";
        const ID_TOKEN: &str = "id_token";
        const REFRESH_TOKEN: &str = "refresh_token";

        let missing = |name: &str| {
            ApiError::Domain(Error::InvalidRequest {
                reason: format!("missing required parameter: {name}"),
            })
        };
        let cross_grant = |name: &str, grant: &str| {
            ApiError::Domain(Error::InvalidRequest {
                reason: format!("{name} is not a parameter of the {grant} grant"),
            })
        };

        match form.grant_type.as_str() {
            AUTHORIZATION_CODE => {
                if form.id_token.is_some() {
                    return Err(cross_grant("id_token", AUTHORIZATION_CODE));
                }
                if form.refresh_token.is_some() {
                    return Err(cross_grant("refresh_token", AUTHORIZATION_CODE));
                }
                if form.provider_access_token.is_some() {
                    return Err(cross_grant("provider_access_token", AUTHORIZATION_CODE));
                }
                let provider = form.provider.ok_or_else(|| missing("provider"))?;
                let code = form.code.ok_or_else(|| missing("code"))?;
                let redirect_uri = form.redirect_uri.ok_or_else(|| missing("redirect_uri"))?;
                Ok(TokenGrant::AuthorizationCode {
                    provider,
                    code,
                    redirect_uri,
                })
            }
            ID_TOKEN => {
                if form.code.is_some() {
                    return Err(cross_grant("code", ID_TOKEN));
                }
                if form.redirect_uri.is_some() {
                    return Err(cross_grant("redirect_uri", ID_TOKEN));
                }
                if form.refresh_token.is_some() {
                    return Err(cross_grant("refresh_token", ID_TOKEN));
                }
                let provider = form.provider.ok_or_else(|| missing("provider"))?;
                let id_token = form.id_token.ok_or_else(|| missing("id_token"))?;
                Ok(TokenGrant::IdTokenAssertion {
                    provider,
                    id_token,
                    provider_access_token: form.provider_access_token,
                })
            }
            REFRESH_TOKEN => {
                if form.provider.is_some() {
                    return Err(cross_grant("provider", REFRESH_TOKEN));
                }
                if form.code.is_some() {
                    return Err(cross_grant("code", REFRESH_TOKEN));
                }
                if form.redirect_uri.is_some() {
                    return Err(cross_grant("redirect_uri", REFRESH_TOKEN));
                }
                if form.id_token.is_some() {
                    return Err(cross_grant("id_token", REFRESH_TOKEN));
                }
                if form.provider_access_token.is_some() {
                    return Err(cross_grant("provider_access_token", REFRESH_TOKEN));
                }
                let refresh_token = form.refresh_token.ok_or_else(|| missing("refresh_token"))?;
                Ok(TokenGrant::RefreshToken { refresh_token })
            }
            // Present but unrecognized (including empty) — RFC 6749 §5.2
            // classifies this as an unsupported grant, not a missing one.
            _ => Err(ApiError::UnsupportedGrantType),
        }
    }
}

pub async fn token_handler(
    State(state): State<AppState>,
    Extension(audit_ctx): Extension<AuditContext>,
    form: TokenForm,
) -> Result<impl IntoResponse, ApiError> {
    // The grants switch gates exposure, and it gates it up front: when the
    // direct ID-token grant is disabled, a request carrying an `id_token`
    // field is rejected as `unsupported_grant_type` whatever `grant_type`
    // declares, so field-presence branch selection cannot evade the switch.
    // The gate lives in this handler (not the core) because
    // `unsupported_grant_type` is a server-layer error class, and this handler
    // is shared by the server, Lambda, and FFI runtimes via `build_router`.
    if form.id_token.is_some() && !state.config.grants.id_token {
        return Err(ApiError::UnsupportedGrantType);
    }

    // Parse/validate at the boundary: everything below this line sees a
    // coherent declared grant, so neither provider port is reachable for a
    // request whose fields disagree with its declared grant.
    let grant = TokenGrant::try_from(form)?;
    match grant {
        TokenGrant::AuthorizationCode {
            provider,
            code,
            redirect_uri,
        } => {
            let result = state
                .service
                .exchange(ExchangeRequest {
                    credential: ExchangeCredential::AuthorizationCode { code, redirect_uri },
                    provider,
                    provider_access_token: None,
                    ip_address: audit_ctx.ip_address.clone(),
                    user_agent: audit_ctx.user_agent.clone(),
                    device_id: audit_ctx.device_id.clone(),
                })
                .await?;
            Ok(Json(result))
        }
        TokenGrant::IdTokenAssertion {
            provider,
            id_token,
            provider_access_token,
        } => {
            let result = state
                .service
                .exchange(ExchangeRequest {
                    credential: ExchangeCredential::IdTokenAssertion { id_token },
                    provider,
                    provider_access_token,
                    ip_address: audit_ctx.ip_address.clone(),
                    user_agent: audit_ctx.user_agent.clone(),
                    device_id: audit_ctx.device_id.clone(),
                })
                .await?;
            Ok(Json(result))
        }
        TokenGrant::RefreshToken { refresh_token } => {
            let result = state
                .service
                .refresh(RefreshRequest {
                    refresh_token,
                    ip_address: audit_ctx.ip_address.clone(),
                    user_agent: audit_ctx.user_agent.clone(),
                    device_id: audit_ctx.device_id.clone(),
                })
                .await?;
            Ok(Json(result))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `TokenForm` carrying only `grant_type`, with every known parameter
    /// absent — the base for per-case builder calls in the table tests.
    fn form_with_grant(grant_type: &str) -> TokenForm {
        TokenForm {
            grant_type: grant_type.to_string(),
            code: None,
            redirect_uri: None,
            provider: None,
            refresh_token: None,
            id_token: None,
            provider_access_token: None,
        }
    }

    fn expect_missing(form: TokenForm, name: &str) {
        match TokenGrant::try_from(form) {
            Err(ApiError::Domain(Error::InvalidRequest { reason })) => assert_eq!(
                reason,
                format!("missing required parameter: {name}"),
                "error_description must name the missing parameter exactly"
            ),
            other => panic!("expected missing-{name} invalid_request, got: {other:?}"),
        }
    }

    fn expect_cross_grant(form: TokenForm, name: &str, grant: &str) {
        match TokenGrant::try_from(form) {
            Err(ApiError::Domain(Error::InvalidRequest { reason })) => assert_eq!(
                reason,
                format!("{name} is not a parameter of the {grant} grant"),
                "error_description must name the offending parameter and the declared grant"
            ),
            other => panic!("expected cross-grant {name}/{grant} invalid_request, got: {other:?}"),
        }
    }

    #[test]
    fn authorization_code_parses_with_closed_parameter_set() {
        let mut form = form_with_grant("authorization_code");
        form.provider = Some("google".to_string());
        form.code = Some("abc".to_string());
        form.redirect_uri = Some("https://app.example.com/cb".to_string());
        // Unknown parameters outside the known set are ignored by design.
        let grant = TokenGrant::try_from(form).expect("complete authorization_code form parses");
        assert_eq!(
            grant,
            TokenGrant::AuthorizationCode {
                provider: "google".to_string(),
                code: "abc".to_string(),
                redirect_uri: "https://app.example.com/cb".to_string(),
            }
        );
    }

    #[test]
    fn authorization_code_rejects_each_cross_grant_field() {
        for (field, value) in [("id_token", "fake.id.token"), ("refresh_token", "rt-1")] {
            let mut form = form_with_grant("authorization_code");
            form.provider = Some("google".to_string());
            form.code = Some("abc".to_string());
            form.redirect_uri = Some("https://app.example.com/cb".to_string());
            match field {
                "id_token" => form.id_token = Some(value.to_string()),
                "refresh_token" => form.refresh_token = Some(value.to_string()),
                other => panic!("unexpected field {other}"),
            }
            expect_cross_grant(form, field, "authorization_code");
        }
    }

    #[test]
    fn authorization_code_requires_provider_code_and_redirect_uri() {
        expect_missing(form_with_grant("authorization_code"), "provider");

        let mut form = form_with_grant("authorization_code");
        form.provider = Some("google".to_string());
        expect_missing(form, "code");

        let mut form = form_with_grant("authorization_code");
        form.provider = Some("google".to_string());
        form.code = Some("abc".to_string());
        expect_missing(form, "redirect_uri");
    }

    #[test]
    fn id_token_grant_parses_and_rejects_cross_grant_fields() {
        let mut form = form_with_grant("id_token");
        form.provider = Some("google".to_string());
        form.id_token = Some("fake.id.token".to_string());
        let grant = TokenGrant::try_from(form).expect("complete id_token form parses");
        assert_eq!(
            grant,
            TokenGrant::IdTokenAssertion {
                provider: "google".to_string(),
                id_token: "fake.id.token".to_string(),
                provider_access_token: None,
            }
        );

        for (field, value) in [
            ("code", "abc"),
            ("redirect_uri", "https://app.example.com/cb"),
            ("refresh_token", "rt-1"),
        ] {
            let mut form = form_with_grant("id_token");
            form.provider = Some("google".to_string());
            form.id_token = Some("fake.id.token".to_string());
            match field {
                "code" => form.code = Some(value.to_string()),
                "redirect_uri" => form.redirect_uri = Some(value.to_string()),
                "refresh_token" => form.refresh_token = Some(value.to_string()),
                other => panic!("unexpected field {other}"),
            }
            expect_cross_grant(form, field, "id_token");
        }
    }

    #[test]
    fn id_token_grant_requires_provider_and_id_token() {
        expect_missing(form_with_grant("id_token"), "provider");

        let mut form = form_with_grant("id_token");
        form.provider = Some("google".to_string());
        expect_missing(form, "id_token");
    }

    #[test]
    fn refresh_grant_parses_and_rejects_cross_grant_fields() {
        let mut form = form_with_grant("refresh_token");
        form.refresh_token = Some("rt-1".to_string());
        let grant = TokenGrant::try_from(form).expect("complete refresh_token form parses");
        assert_eq!(
            grant,
            TokenGrant::RefreshToken {
                refresh_token: "rt-1".to_string(),
            }
        );

        for (field, value) in [
            ("provider", "google"),
            ("code", "abc"),
            ("redirect_uri", "https://app.example.com/cb"),
            ("id_token", "fake.id.token"),
        ] {
            let mut form = form_with_grant("refresh_token");
            form.refresh_token = Some("rt-1".to_string());
            match field {
                "provider" => form.provider = Some(value.to_string()),
                "code" => form.code = Some(value.to_string()),
                "redirect_uri" => form.redirect_uri = Some(value.to_string()),
                "id_token" => form.id_token = Some(value.to_string()),
                other => panic!("unexpected field {other}"),
            }
            expect_cross_grant(form, field, "refresh_token");
        }

        expect_missing(form_with_grant("refresh_token"), "refresh_token");
    }

    #[test]
    fn unknown_or_empty_grant_type_is_unsupported_grant_type() {
        for grant_type in ["client_credentials", "password", ""] {
            match TokenGrant::try_from(form_with_grant(grant_type)) {
                Err(ApiError::UnsupportedGrantType) => {}
                other => panic!(
                    "grant_type {grant_type:?} must be unsupported_grant_type, got {other:?}"
                ),
            }
        }
    }

    #[test]
    fn missing_grant_type_is_invalid_request_naming_the_parameter() {
        let raw = RawTokenForm {
            grant_type: None,
            code: Some("abc".to_string()),
            redirect_uri: None,
            provider: Some("google".to_string()),
            refresh_token: None,
            id_token: None,
            provider_access_token: None,
        };
        match TokenForm::try_from(raw) {
            Err(ApiError::Domain(Error::InvalidRequest { reason })) => assert_eq!(
                reason, "missing required parameter: grant_type",
                "absent grant_type must classify as invalid_request, not 422 or unsupported"
            ),
            other => panic!("expected missing grant_type invalid_request, got {other:?}"),
        }
    }
}
