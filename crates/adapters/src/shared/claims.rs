//! Shared claim-value coercion helpers used by both the generic OIDC adapter and the
//! Apple provider when mapping JSON claim values onto `IdentityClaims` fields.

/// Coerce a JSON claim value to `Option<bool>`.
///
/// Some providers (notably Apple) send boolean-valued claims such as `email_verified`
/// and `is_private_email` as the JSON strings `"true"`/`"false"` rather than JSON
/// booleans. This helper accepts either representation:
///
/// - a JSON `true`/`false` passes through as `Some(true)`/`Some(false)`,
/// - the strings `"true"`/`"false"` coerce to `Some(true)`/`Some(false)`,
/// - everything else (numbers, other strings, `null`, or an absent value) yields
///   `None` — the coercion never guesses.
pub fn coerce_bool(value: &serde_json::Value) -> Option<bool> {
    if let Some(b) = value.as_bool() {
        return Some(b);
    }
    match value.as_str() {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coerces_json_bool_true() {
        let value = json!(true);
        let result = coerce_bool(&value);
        assert_eq!(result, Some(true));
        assert!(result.is_some());
    }

    #[test]
    fn coerces_json_bool_false() {
        let value = json!(false);
        let result = coerce_bool(&value);
        assert_eq!(result, Some(false));
        assert!(!result.unwrap());
    }

    #[test]
    fn coerces_string_true() {
        let value = json!("true");
        let result = coerce_bool(&value);
        assert_eq!(result, Some(true));
        assert!(result.is_some());
    }

    #[test]
    fn coerces_string_false() {
        let value = json!("false");
        let result = coerce_bool(&value);
        assert_eq!(result, Some(false));
        assert!(!result.unwrap());
    }

    #[test]
    fn non_coercible_number_yields_none() {
        let value = json!(1);
        let result = coerce_bool(&value);
        assert_eq!(result, None);
        assert!(result.is_none());
    }

    #[test]
    fn non_coercible_string_yields_none() {
        let value = json!("yes");
        let result = coerce_bool(&value);
        assert_eq!(result, None);
        assert!(result.is_none());
    }

    #[test]
    fn null_yields_none() {
        let value = serde_json::Value::Null;
        let result = coerce_bool(&value);
        assert_eq!(result, None);
        assert!(result.is_none());
    }

    #[test]
    fn absent_key_yields_none() {
        let claims = json!({ "other": "field" });
        let value = &claims["email_verified"];
        assert!(value.is_null());
        let result = coerce_bool(value);
        assert_eq!(result, None);
    }
}
