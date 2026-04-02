use std::collections::HashMap;

use serde_json::Value;

use crate::domain::User;

/// Reserved JWT claim names that must not be overridden by custom claims.
const RESERVED_CLAIMS: &[&str] = &["sub", "iss", "aud", "iat", "exp"];

fn is_reserved(key: &str) -> bool {
    RESERVED_CLAIMS.contains(&key)
}

/// Resolve custom claims by merging config template claims with per-user claims.
///
/// Per-user claims (`user.claims`) take precedence over config template claims.
/// Reserved JWT claim names (`sub`, `iss`, `aud`, `iat`, `exp`) are silently ignored
/// from both sources.
pub fn resolve_custom_claims(
    config_claims: &Option<HashMap<String, String>>,
    user: &User,
) -> HashMap<String, Value> {
    let mut result = HashMap::new();

    // 1. Resolve config template claims
    if let Some(templates) = config_claims {
        for (key, template) in templates {
            if is_reserved(key) {
                continue;
            }
            if let Some(value) = resolve_template(template, user) {
                result.insert(key.clone(), value);
            }
        }
    }

    // 2. Merge per-user claims on top (they take precedence)
    for (key, value) in &user.claims {
        if is_reserved(key) {
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
            user.claims.get(*key).cloned()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
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
