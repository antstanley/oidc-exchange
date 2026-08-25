use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

/// A value that must never reach a log line, a span field, or an error string.
///
/// Implements neither [`Debug`](std::fmt::Debug) nor [`Display`](std::fmt::Display), so
/// `tracing`'s value capture — including `#[instrument]`'s default argument recording and
/// any `?value` field — cannot render it; formatting one is a compile error rather than a
/// review miss. `serde` support is deliberately transparent (`#[serde(transparent)]`),
/// because persistence and the log stream are different trust domains: stored records,
/// wire bodies, and schemas keep their exact string shapes while the in-memory value is
/// unprintable.
///
/// Reach for the raw value only at a deliberate boundary — building a store key, feeding a
/// constant-time comparison, posting to a provider — via [`Secret::expose`], or consume
/// it with [`Secret::into_inner`].
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// Wrap a credential-derived value.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Reveal the wrapped value at a deliberate use boundary (store key, HMAC key,
    /// request body). Never pass the result to a formatter or a tracing macro.
    pub fn expose(&self) -> &T {
        &self.0
    }

    /// Consume the secret, unwrapping the raw value. The escape hatch of last resort:
    /// prefer [`Secret::expose`] so the value stays wrapped as long as possible.
    pub fn into_inner(self) -> T {
        self.0
    }
}

/// Constant-time equality for string secrets, via `subtle`.
///
/// Implemented only for `Secret<String>` — the shape every enumerated credential-derived
/// value has after wrapping — so a comparison cannot become a timing oracle on the values
/// that matter (shared-secret bearer checks, token-hash lookups). Length differences are
/// handled by comparing both sides against a zeroed buffer of equal length, mirroring the
/// repository's existing defense-in-depth pattern.
impl PartialEq for Secret<String> {
    fn eq(&self, other: &Self) -> bool {
        // Precondition: `expose` hands over raw bytes precisely so this comparison sees
        // them without any formatting side door.
        let left = self.0.as_bytes();
        let right = other.0.as_bytes();
        if left.len() != right.len() {
            // Still burn a comparison against an equally sized dummy buffer so total
            // length cannot be inferred from timing; the result is false either way.
            let _ = left.ct_eq(&vec![0u8; left.len()]);
            return false;
        }
        left.ct_eq(right).into()
    }
}

impl Eq for Secret<String> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Equal secrets compare equal through the constant-time path. Assertions use bare
    /// `assert!(a == b)` rather than `assert_eq!` because formatting a mismatched pair
    /// would require `Debug` — exactly the trait this type refuses to implement.
    #[test]
    fn equal_secrets_are_equal() {
        let a = Secret::new("same-value-1".to_string());
        let b = Secret::new("same-value-1".to_string());
        assert!(a == b, "identical values must compare equal");
        assert!(a == a.clone(), "clones must compare equal");
    }

    /// Unequal secrets of equal length compare unequal; unequal lengths fail closed too.
    #[test]
    fn unequal_secrets_are_not_equal() {
        let a = Secret::new("value-alpha".to_string());
        let b = Secret::new("value-bravo".to_string());
        assert!(a != b, "differing same-length values must compare unequal");

        let short = Secret::new("short".to_string());
        let long = Secret::new("a-much-longer-value".to_string());
        assert!(short != long);
        // Symmetry of the comparison matters for the length-mismatch path, so assert it
        // from both directions rather than relying on the general equality contract.
        assert!(long != short);
    }

    /// Empty secrets are still valid values and compare equal to each other.
    #[test]
    fn empty_secrets_compare_equal() {
        let a = Secret::<String>::new(String::new());
        let b = Secret::<String>::new(String::new());
        assert!(a == b);
        assert!(a != Secret::new("nonempty".to_string()));
    }

    /// serde transparency: `Secret<String>` serializes as exactly the wrapped string, so
    /// persisted records and wire bodies keep their current shapes with no migration.
    #[test]
    fn serde_round_trip_is_identical_to_plain_string() {
        let plain = "deadbeefcafe0123456789abcdef7890";
        let wrapped = Secret::new(plain.to_string());

        let as_secret = serde_json::to_string(&wrapped).expect("serialize Secret<String>");
        let as_plain = serde_json::to_string(&plain.to_string()).expect("serialize String");
        assert_eq!(
            as_secret, as_plain,
            "serialization must not change the wire shape"
        );

        let back: Secret<String> =
            serde_json::from_str(&as_secret).expect("deserialize into Secret<String>");
        assert_eq!(back.expose(), &plain.to_string());
    }

    /// `expose` and `into_inner` hand over the identical value they were given.
    #[test]
    fn expose_and_into_inner_return_the_wrapped_value() {
        let value = "exposed-once".to_string();
        let secret = Secret::new(value.clone());
        assert_eq!(secret.expose(), &value);
        assert_eq!(secret.into_inner(), value);
    }
}
