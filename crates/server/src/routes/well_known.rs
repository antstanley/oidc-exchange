use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::state::AppState;

/// The grant types the process actually serves. `authorization_code` and
/// `refresh_token` are always served; the direct `id_token` grant appears
/// only when `grants.id_token` is enabled, so the discovery document stays
/// true in both switch states.
fn grant_types_supported(id_token_grant_enabled: bool) -> Vec<&'static str> {
    let mut grants = vec!["authorization_code", "refresh_token"];
    if id_token_grant_enabled {
        grants.push("id_token");
    }
    grants
}

pub async fn openid_config_handler(State(state): State<AppState>) -> impl IntoResponse {
    let issuer = &state.config.server.issuer;
    let alg = state.service.signing_algorithm();
    let grant_types = grant_types_supported(state.config.grants.id_token);
    assert!(
        grant_types.contains(&"authorization_code") && grant_types.contains(&"refresh_token"),
        "the always-served grants must never drop off discovery"
    );
    assert_eq!(
        grant_types.contains(&"id_token"),
        state.config.grants.id_token,
        "id_token advertisement must mirror the grants switch exactly"
    );
    Json(json!({
        "issuer": issuer,
        "jwks_uri": format!("{}/keys", issuer),
        "token_endpoint": format!("{}/token", issuer),
        "revocation_endpoint": format!("{}/revoke", issuer),
        "grant_types_supported": grant_types,
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": [alg]
    }))
}

#[cfg(test)]
mod tests {
    use super::grant_types_supported;

    /// Discovery advertises the direct grant exactly when the switch is on —
    /// and never drops the two always-served grants in either state.
    #[test]
    fn grant_types_track_the_switch_in_both_states() {
        let disabled = grant_types_supported(false);
        assert!(disabled.contains(&"authorization_code"));
        assert!(disabled.contains(&"refresh_token"));
        assert!(
            !disabled.contains(&"id_token"),
            "disabled deployments must not advertise id_token"
        );
        assert_eq!(disabled.len(), 2, "disabled advertisement stays minimal");

        let enabled = grant_types_supported(true);
        assert!(enabled.contains(&"authorization_code"));
        assert!(enabled.contains(&"refresh_token"));
        assert!(
            enabled.contains(&"id_token"),
            "enabled deployments advertise id_token"
        );
        assert_eq!(enabled.len(), 3);
    }
}
