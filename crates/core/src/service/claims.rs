use std::collections::HashMap;

use serde_json::Value;

use crate::domain::User;

/// Number of names in [`RESERVED_CLAIMS`]. Kept beside the set so the array
/// length is checked against a named bound, and so tests can fail loudly if
/// the two ever drift apart.
pub const RESERVED_CLAIM_COUNT: usize = 24;

/// The closed set of reserved protocol claim names — the names a conformant
/// verifier or relying party reads as protocol-defined, so a caller-supplied
/// claim of the same name could override or forge them in a signed token.
///
/// Mirrors the registry-backed enumeration in `03-service-flows.md`:
/// - the RFC 7519 §4.1 registered names,
/// - the OpenID Connect, RFC 9068, and RFC 7800 names, including `sid` and
///   `nbf` (reserved by the revoke-claims work: `sid` collides with the
///   flattened session binding `/revoke` resolves),
/// - the de-facto authorization names.
///
/// It is *closed*, not a maintained denylist: the set changes only when the
/// registry it mirrors does, never per-incident. It is enforced at the write
/// boundaries (`admin_set_claims`/`admin_merge_claims`/`admin_update_user`),
/// at configuration acceptance (`token.custom_claims` in
/// `AppConfig::validate`), and defensively at token build and template
/// resolution, so a record written before the write-path rule existed still
/// cannot leak a reserved name into a signed token.
pub const RESERVED_CLAIMS: [&str; RESERVED_CLAIM_COUNT] = [
    // RFC 7519 §4.1 registered names.
    "iss",
    "sub",
    "aud",
    "exp",
    "nbf",
    "iat",
    "jti",
    // OpenID Connect, RFC 9068, and RFC 7800 names.
    "acr",
    "amr",
    "at_hash",
    "auth_time",
    "azp",
    "c_hash",
    "cnf",
    "nonce",
    "sid",
    "typ",
    "client_id",
    // De-facto authorization names.
    "scope",
    "scp",
    "roles",
    "groups",
    "entitlements",
    "permissions",
];

/// Whether `key` collides with a protocol-owned claim name. Claim names are
/// case-sensitive per JWT: `Sub` is not reserved, `sub` is.
pub fn is_reserved_claim(key: &str) -> bool {
    RESERVED_CLAIMS.contains(&key)
}

/// The lowest (sorted) reserved key in a caller-supplied claim map, if any.
///
/// Sorted so the reported offender is deterministic when a payload carries
/// several reserved names; callers turn the name into an `InvalidRequest`
/// reason at the write/config boundary.
pub(crate) fn find_reserved_claim_key<V>(claims: &HashMap<String, V>) -> Option<String> {
    let mut keys: Vec<&String> = claims.keys().collect();
    keys.sort();
    keys.into_iter().find(|key| is_reserved_claim(key)).cloned()
}

/// Resolve custom claims by merging config template claims with per-user claims.
///
/// Per-user claims (`user.claims`) take precedence over config template claims.
/// Reserved protocol claim names ([`RESERVED_CLAIMS`]) are silently ignored
/// from both sources — the write path and config validation reject them before
/// they can be persisted or configured; this filter is the defensive last line
/// for records written before that rule existed.
pub fn resolve_custom_claims(
    config_claims: &Option<HashMap<String, String>>,
    user: &User,
) -> HashMap<String, Value> {
    let mut result = HashMap::new();

    // 1. Resolve config template claims
    if let Some(templates) = config_claims {
        for (key, template) in templates {
            if is_reserved_claim(key) {
                continue;
            }
            if let Some(value) = resolve_template(template, user) {
                result.insert(key.clone(), value);
            }
        }
    }

    // 2. Merge per-user claims on top (they take precedence)
    for (key, value) in &user.claims {
        if is_reserved_claim(key) {
            continue;
        }
        result.insert(key.clone(), value.clone());
    }

    result
}

/// Resolve a single template string against the user model.
///
/// - If the string is wrapped in `{{ }}`, it's treated as a field reference
///   (optionally with a `| default: 'value'` filter).
/// - Otherwise, it's a static string value.
fn resolve_template(template: &str, user: &User) -> Option<Value> {
    let trimmed = template.trim();
    if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
        let inner = trimmed[2..trimmed.len() - 2].trim();
        if let Some((path, default)) = parse_default_filter(inner) {
            resolve_field(path.trim(), user).or(Some(Value::String(default)))
        } else {
            resolve_field(inner, user)
        }
    } else {
        // Static string
        Some(Value::String(template.to_string()))
    }
}

/// Parse a `| default: 'value'` filter from a template expression.
///
/// Returns `Some((field_path, default_value))` if the filter is present.
fn parse_default_filter(inner: &str) -> Option<(&str, String)> {
    // Look for `| default:` pattern
    let pipe_pos = inner.find('|')?;
    let path = &inner[..pipe_pos];
    let filter_part = inner[pipe_pos + 1..].trim();

    // Must start with "default:"
    let rest = filter_part.strip_prefix("default:")?;
    let rest = rest.trim();

    // Extract the quoted default value (single quotes)
    let default_value = if rest.starts_with('\'') && rest.ends_with('\'') && rest.len() >= 2 {
        rest[1..rest.len() - 1].to_string()
    } else {
        // Unquoted value — take as-is
        rest.to_string()
    };

    Some((path, default_value))
}

/// Resolve a dot-notation field path against the User model.
///
/// Supported paths:
/// - `user.id`, `user.email`, `user.display_name`, `user.provider`, `user.external_id`
/// - `user.metadata.KEY`
/// - `user.claims.KEY`
fn resolve_field(path: &str, user: &User) -> Option<Value> {
    let segments: Vec<&str> = path.split('.').collect();

    // First segment must be "user"
    if segments.first() != Some(&"user") || segments.len() < 2 {
        return None;
    }

    match segments[1] {
        "id" => Some(Value::String(user.id.clone())),
        "email" => user.email.as_ref().map(|e| Value::String(e.clone())),
        "display_name" => user.display_name.as_ref().map(|d| Value::String(d.clone())),
        "provider" => Some(Value::String(user.provider.clone())),
        "external_id" => Some(Value::String(user.external_id.clone())),
        "metadata" => {
            let key = segments.get(2)?;
            user.metadata.get(*key).cloned()
        }
        "claims" => {
            let key = segments.get(2)?;
            // A template may not read a reserved claim back out of the
            // persisted map: records written before the write-path rule
            // existed could otherwise re-export a protocol-colliding value
            // into a signed token through this second route. Refusing yields
            // the same behaviour as a missing field (a `default:` filter, if
            // any, applies).
            if is_reserved_claim(key) {
                return None;
            }
            user.claims.get(*key).cloned()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact 24 names the closed set must contain, spelled out
    /// independently of [`RESERVED_CLAIMS`] so a typo or an accidental
    /// addition/removal on either side fails these tests rather than silently
    /// widening or narrowing the enforced set.
    const EXPECTED_RESERVED_CLAIMS: [&str; RESERVED_CLAIM_COUNT] = [
        "iss",
        "sub",
        "aud",
        "exp",
        "nbf",
        "iat",
        "jti",
        "acr",
        "amr",
        "at_hash",
        "auth_time",
        "azp",
        "c_hash",
        "cnf",
        "nonce",
        "sid",
        "typ",
        "client_id",
        "scope",
        "scp",
        "roles",
        "groups",
        "entitlements",
        "permissions",
    ];

    fn make_user() -> User {
        User {
            id: "usr_123".to_string(),
            external_id: "ext_456".to_string(),
            provider: "google".to_string(),
            email: Some("alice@example.com".to_string()),
            display_name: Some("Alice".to_string()),
            metadata: HashMap::new(),
            claims: HashMap::new(),
            status: crate::domain::UserStatus::Active,
            version: crate::domain::INITIAL_USER_VERSION,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn reserved_claim_set_is_exactly_the_closed_24_name_protocol_set() {
        assert_eq!(
            RESERVED_CLAIMS.len(),
            RESERVED_CLAIM_COUNT,
            "the set's length must equal its named count"
        );

        let mut actual: Vec<&str> = RESERVED_CLAIMS.to_vec();
        actual.sort();
        let mut expected = EXPECTED_RESERVED_CLAIMS.to_vec();
        expected.sort();

        assert_eq!(
            actual, expected,
            "RESERVED_CLAIMS must be exactly the closed protocol set, no more, no fewer"
        );

        // Every name must also be individually detectable.
        for name in EXPECTED_RESERVED_CLAIMS {
            assert!(
                is_reserved_claim(name),
                "{name:?} is in the closed set and must be flagged as reserved"
            );
        }
    }

    /// Negative space: near-misses, case variants, and ordinary custom claim
    /// names are not reserved — claim names are case-sensitive per JWT.
    #[test]
    fn non_reserved_and_case_variant_names_are_not_flagged() {
        for name in [
            "role",
            "tenant",
            "email",
            "custom",
            "",
            "Sub",
            "SUB",
            "scopes",
            "subject",
            "iatx",
            "client_ids",
        ] {
            assert!(
                !is_reserved_claim(name),
                "{name:?} is not a protocol claim name and must stay writable"
            );
        }
    }

    /// Every one of the 24 reserved names is dropped from *both* sources at
    /// token build — including from a per-user map that models a record
    /// persisted before the write-path rule existed — while non-reserved keys
    /// from each source survive.
    #[test]
    fn resolve_custom_claims_drops_every_reserved_name_from_both_sources() {
        let mut user = make_user();
        let mut config_claims = HashMap::new();

        for name in EXPECTED_RESERVED_CLAIMS {
            config_claims.insert(name.to_string(), "from-config".to_string());
            user.claims
                .insert(name.to_string(), Value::String("persisted".to_string()));
        }
        config_claims.insert("kept_config".to_string(), "visible".to_string());
        user.claims.insert(
            "kept_user".to_string(),
            Value::String("visible".to_string()),
        );

        let result = resolve_custom_claims(&Some(config_claims), &user);

        for name in EXPECTED_RESERVED_CLAIMS {
            assert!(
                !result.contains_key(name),
                "reserved name {name:?} must never reach the signed token"
            );
        }
        assert_eq!(
            result.get("kept_config"),
            Some(&Value::String("visible".to_string()))
        );
        assert_eq!(
            result.get("kept_user"),
            Some(&Value::String("visible".to_string()))
        );
    }

    /// The template route is closed too: `{{ user.claims.<reserved> }}`
    /// resolves to nothing even when the persisted record carries the name,
    /// and a `default:` filter yields the operator's static value rather than
    /// leaking the stored claim.
    #[test]
    fn resolve_field_refuses_persisted_reserved_claims() {
        let mut user = make_user();

        for name in EXPECTED_RESERVED_CLAIMS {
            user.claims.insert(
                name.to_string(),
                Value::String("secret-override".to_string()),
            );
        }

        for name in EXPECTED_RESERVED_CLAIMS {
            assert_eq!(
                resolve_field(&format!("user.claims.{name}"), &user),
                None,
                "{{ user.claims.{name} }} must not resolve even when persisted"
            );
        }

        // Paired positive: non-reserved claims still resolve through templates.
        user.claims
            .insert("tier".to_string(), Value::String("gold".to_string()));
        assert_eq!(
            resolve_field("user.claims.tier", &user),
            Some(Value::String("gold".to_string()))
        );
    }

    #[test]
    fn reserved_template_reference_falls_back_to_default_filter_not_stored_value() {
        let mut user = make_user();
        user.claims.insert(
            "sid".to_string(),
            Value::String("forged-session".to_string()),
        );

        let result = resolve_template("{{ user.claims.sid | default: 'fallback' }}", &user);

        assert_eq!(
            result,
            Some(Value::String("fallback".to_string())),
            "a refused reserved reference must behave like a missing field"
        );
    }

    #[test]
    fn resolve_template_static_string() {
        let user = make_user();
        let result = resolve_template("example", &user);
        assert_eq!(result, Some(Value::String("example".to_string())));
    }

    #[test]
    fn resolve_template_field_reference() {
        let user = make_user();
        let result = resolve_template("{{ user.email }}", &user);
        assert_eq!(result, Some(Value::String("alice@example.com".to_string())));
    }

    #[test]
    fn resolve_template_with_default_when_missing() {
        let user = make_user();
        let result = resolve_template("{{ user.metadata.role | default: 'user' }}", &user);
        assert_eq!(result, Some(Value::String("user".to_string())));
    }

    #[test]
    fn resolve_template_with_default_when_present() {
        let mut user = make_user();
        user.metadata
            .insert("role".to_string(), Value::String("admin".to_string()));
        let result = resolve_template("{{ user.metadata.role | default: 'user' }}", &user);
        assert_eq!(result, Some(Value::String("admin".to_string())));
    }

    #[test]
    fn resolve_template_missing_field_no_default() {
        let user = make_user();
        let result = resolve_template("{{ user.metadata.missing }}", &user);
        assert_eq!(result, None);
    }

    #[test]
    fn parse_default_filter_valid() {
        let (path, default) = parse_default_filter("user.metadata.role | default: 'user'").unwrap();
        assert_eq!(path.trim(), "user.metadata.role");
        assert_eq!(default, "user");
    }

    #[test]
    fn parse_default_filter_none() {
        assert!(parse_default_filter("user.email").is_none());
    }

    #[test]
    fn resolve_field_direct_fields() {
        let user = make_user();
        assert_eq!(
            resolve_field("user.id", &user),
            Some(Value::String("usr_123".to_string()))
        );
        assert_eq!(
            resolve_field("user.email", &user),
            Some(Value::String("alice@example.com".to_string()))
        );
        assert_eq!(
            resolve_field("user.display_name", &user),
            Some(Value::String("Alice".to_string()))
        );
        assert_eq!(
            resolve_field("user.provider", &user),
            Some(Value::String("google".to_string()))
        );
        assert_eq!(
            resolve_field("user.external_id", &user),
            Some(Value::String("ext_456".to_string()))
        );
    }

    #[test]
    fn resolve_field_optional_none() {
        let mut user = make_user();
        user.email = None;
        assert_eq!(resolve_field("user.email", &user), None);
    }

    #[test]
    fn resolve_field_metadata() {
        let mut user = make_user();
        user.metadata
            .insert("org".to_string(), Value::String("acme".to_string()));
        assert_eq!(
            resolve_field("user.metadata.org", &user),
            Some(Value::String("acme".to_string()))
        );
    }

    #[test]
    fn resolve_field_claims() {
        let mut user = make_user();
        user.claims
            .insert("tier".to_string(), Value::String("premium".to_string()));
        assert_eq!(
            resolve_field("user.claims.tier", &user),
            Some(Value::String("premium".to_string()))
        );
    }

    #[test]
    fn resolve_field_invalid_root() {
        let user = make_user();
        assert_eq!(resolve_field("foo.email", &user), None);
    }

    #[test]
    fn resolve_field_invalid_segment() {
        let user = make_user();
        assert_eq!(resolve_field("user.nonexistent", &user), None);
    }
}
