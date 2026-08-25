//! Endpoint-origin pinning: the set of remote origins a provider's discovery
//! document is permitted to name.
//!
//! The RFC 8414 issuer self-consistency check is a string comparison and
//! constrains nothing about the endpoints the document goes on to name — a
//! compromised or hostile document could otherwise relocate the verification-
//! key source or the destination the client secret is posted to. Pinning the
//! permitted set in config keeps the operator's declared intent authoritative
//! over the provider's runtime assertion: discovery may *confirm* which origins
//! this service talks to, but can never *widen* them.
//!
//! Two strictness levels live here on purpose:
//!
//! - **Declared origins** (`endpoint_origins` config entries) are validated
//!   strictly — `https`, host, optional port, and nothing else — because they
//!   are security input and a sloppy entry must fail config load, not silently
//!   pin less than the operator wrote.
//! - **Observed origins** (issuer, configured endpoint overrides, discovered
//!   endpoints) are extracted leniently — any scheme — because the wiremock
//!   test-suite serves plain-`http` loopback origins (recorded wave-A
//!   deviation) and because the https scheme constraint on configured and
//!   discovered endpoints belongs to the sibling `fail_closed` change, not
//!   here. Membership comparison happens on normalized origin strings either
//!   way (`scheme://host[:port]`, default ports elided, hosts lowercased).

use oidc_exchange_core::error::{Error, Result};

/// Hard bound on how many origins one provider may declare, so a malformed or
/// hostile config cannot grow the pinned set without limit.
///
/// Real providers need two or three (Google needs exactly two beyond its
/// issuer); anything past sixteen is a config mistake worth rejecting.
pub const MAX_ENDPOINT_ORIGINS: usize = 16;

/// Hard bound on the byte length of a single declared origin entry, in bytes.
///
/// A legitimate origin is well under this; the bound exists so an oversized
/// entry is rejected *before* any parse or error formatting, keeping hostile
/// config text out of log and error surfaces entirely.
pub const MAX_ENDPOINT_ORIGIN_LEN_BYTES: usize = 256;

/// Why an operator-declared origin string was rejected at the config boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginParseError {
    /// The entry did not parse as an absolute URL at all.
    NotAnAbsoluteUrl,
    /// The entry parsed but its scheme was not `https`.
    SchemeNotHttps { actual_scheme: String },
    /// The entry carried a path, query, or fragment; an origin is only
    /// `scheme "://" host [":" port]`.
    NotABareOrigin,
}

impl std::fmt::Display for OriginParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Messages describe the violation class, never echo the rejected input:
        // config values are operator-controlled text and must not become a
        // log-injection channel through error strings.
        match self {
            Self::NotAnAbsoluteUrl => write!(f, "endpoint origin is not an absolute URL"),
            Self::SchemeNotHttps { actual_scheme } => {
                write!(
                    f,
                    "endpoint origin scheme must be https, found {actual_scheme:?}"
                )
            }
            Self::NotABareOrigin => write!(
                f,
                "endpoint origin must be scheme://host[:port] with no path, query, or fragment"
            ),
        }
    }
}

impl std::error::Error for OriginParseError {}

/// Validate one operator-declared `endpoint_origins` entry and return its
/// normalized origin string.
///
/// Strict by design: `https` scheme (the sibling change's scheme constraint,
/// applied where the operator declares what discovery may name), and a bare
/// origin with no path, query, or fragment. The returned string is the
/// canonical form used for every membership comparison. The std `Result` is
/// used deliberately: this is a parse result, not a crate error, so callers
/// translate it into their own error type at their boundary.
///
/// Callers must enforce [`MAX_ENDPOINT_ORIGIN_LEN_BYTES`] before invoking this
/// so oversized entries never reach URL parsing at all.
pub fn parse_https_origin(input: &str) -> std::result::Result<String, OriginParseError> {
    assert!(
        !input.is_empty(),
        "declared origin entries are non-empty here"
    );
    assert!(
        input.len() <= MAX_ENDPOINT_ORIGIN_LEN_BYTES,
        "callers must reject over-length entries before parsing"
    );

    let url = reqwest::Url::parse(input).map_err(|_| OriginParseError::NotAnAbsoluteUrl)?;

    // The url crate lowercases schemes during parsing, so comparing against the
    // literal reports the canonical form.
    if url.scheme() != "https" {
        return Err(OriginParseError::SchemeNotHttps {
            actual_scheme: url.scheme().to_string(),
        });
    }

    let has_no_path = url.path() == "/" || url.path().is_empty();
    if !has_no_path || url.query().is_some() || url.fragment().is_some() {
        return Err(OriginParseError::NotABareOrigin);
    }

    // Parsed special-scheme URLs always carry a host (the url crate rejects
    // hostless ones at parse time); pin that invariant instead of trusting it.
    assert!(
        url.host_str().is_some(),
        "the url crate guarantees a host for parsed https URLs"
    );

    Ok(origin_string("https", &url))
}

/// Extract the normalized origin (`scheme://host[:port]`) of an already-trusted
/// config-supplied URL.
///
/// Lenient on purpose: any scheme passes, so plain-http loopback test origins
/// keep working. Returns `None` when the string does not parse as an absolute
/// URL with a host — such a string pins nothing and admits nothing.
pub fn origin_of(url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }

    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    if host.is_empty() {
        return None;
    }

    Some(origin_string(parsed.scheme(), &parsed))
}

/// Compose the normalized origin string for a parsed URL.
fn origin_string(scheme: &str, url: &reqwest::Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        // `url.port()` is None when the port equals the scheme default, so an
        // explicit `:443` normalizes to the same origin string as its absence.
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    }
}

/// The permitted endpoint-origin set for one provider, fixed at config load.
///
/// Built from three sources — the issuer's own origin, the origins of any
/// endpoints the operator configured explicitly, and every declared
/// `endpoint_origins` entry — and never mutated again afterwards. A discovery
/// document may confirm membership but cannot add to the set.
#[derive(Debug, Clone)]
pub struct EndpointOrigins {
    /// Normalized origin strings; may be empty (issuer-only providers whose
    /// endpoints all share the issuer's origin still admit those via the
    /// issuer entry).
    origins: Vec<String>,
}

impl EndpointOrigins {
    /// Build the set from an issuer URL, the provider's explicitly configured
    /// endpoint overrides, and the declared `endpoint_origins` extras.
    ///
    /// Pieces that do not parse as absolute URLs contribute nothing: they pin
    /// no origin, so under enforcement they can never admit an endpoint. The
    /// declared extras were already strict-validated at the config boundary;
    /// they are re-read leniently here as a paired check, and the total is
    /// capped at [`MAX_ENDPOINT_ORIGINS`] (config load rejects overflow before
    /// this constructor runs).
    pub fn from_parts(
        issuer: &str,
        configured_endpoints: &[&str],
        declared_extras: &[String],
    ) -> Self {
        assert!(
            !issuer.is_empty(),
            "an issuer URL is required to pin origins"
        );

        let mut origins = Vec::new();
        let push = |candidate: Option<String>, origins: &mut Vec<String>| {
            let Some(origin) = candidate else { return };
            if origins.len() >= MAX_ENDPOINT_ORIGINS {
                // Config-load validation rejects over-long lists; this is the
                // paired defensive stop so the set can never silently outgrow
                // its documented bound even if a future caller skips that gate.
                return;
            }
            if !origins.contains(&origin) {
                origins.push(origin);
            }
        };

        push(origin_of(issuer), &mut origins);
        for endpoint in configured_endpoints {
            push(origin_of(endpoint), &mut origins);
        }
        for extra in declared_extras {
            push(origin_of(extra), &mut origins);
        }

        debug_assert!(origins.len() <= MAX_ENDPOINT_ORIGINS);
        Self { origins }
    }

    /// Whether `candidate_url`'s origin is in the pinned set.
    ///
    /// An unparseable candidate is never admitted: pinning compares origins,
    /// and a string with no derivable origin has none to compare.
    pub fn admits(&self, candidate_url: &str) -> bool {
        match origin_of(candidate_url) {
            Some(observed) => self.contains_normalized(&observed),
            None => false,
        }
    }

    /// Membership test on an already-normalized origin string.
    pub fn contains_normalized(&self, observed_origin: &str) -> bool {
        assert!(
            !observed_origin.is_empty(),
            "observed origins are non-empty"
        );
        self.origins.iter().any(|o| o == observed_origin)
    }

    /// The pinned origins, for structured warning output.
    pub fn as_list(&self) -> &[String] {
        &self.origins
    }
}

/// How a discovered endpoint outside the pinned set is treated.
///
/// This is the release boundary between the two stages of the rollout: the
/// service ships in [`OriginCheckMode::Warn`] for one release so deployments
/// relying on an undeclared cross-origin endpoint learn about it from a log
/// line rather than an outage, and enforcement lands only as an explicit,
/// separately reviewed flip of [`ENDPOINT_ORIGIN_CHECK_MODE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginCheckMode {
    /// Log a structured warning naming the endpoint, its observed origin, and
    /// the permitted set, then accept the endpoint.
    Warn,
    /// Reject the endpoint with a `ProviderError` naming all three.
    Enforce,
}

/// The shipped endpoint-origin check mode: **warning**.
///
/// Flipping this to [`OriginCheckMode::Enforce`] is the explicit release-owner
/// decision that closes the one-release warning window specified by the source
/// change — it must land as its own reviewed commit after operators have had a
/// release to declare their cross-origin endpoints, never folded silently into
/// another change.
pub const ENDPOINT_ORIGIN_CHECK_MODE: OriginCheckMode = OriginCheckMode::Warn;

/// Check one discovery-supplied endpoint against the provider's pinned
/// endpoint-origin set, under the given mode.
///
/// Production call sites pass [`ENDPOINT_ORIGIN_CHECK_MODE`]; tests pass both
/// modes explicitly so the enforcement behaviour is fully implemented and
/// covered ahead of the release flip rather than written blind.
pub fn check_pinned_origin(
    provider: &str,
    endpoint_kind: &str,
    endpoint_url: &str,
    allowed: &EndpointOrigins,
    mode: OriginCheckMode,
) -> Result<()> {
    assert!(!provider.is_empty(), "provider label is required");
    assert!(!endpoint_kind.is_empty(), "endpoint kind label is required");

    let observed = origin_of(endpoint_url);
    let admitted = observed
        .as_deref()
        .map(|origin| allowed.contains_normalized(origin))
        .unwrap_or(false);

    if admitted {
        return Ok(());
    }

    // Endpoints and origins are configuration-grade facts (they name hosts, not
    // credentials or personal data), so naming them here is what makes the
    // warning actionable; bodies, tokens, and subjects stay out of every path
    // in this module.
    let observed_display = observed.as_deref().unwrap_or("<unparseable>");
    match mode {
        OriginCheckMode::Warn => {
            tracing::warn!(
                provider = %provider,
                endpoint_kind = %endpoint_kind,
                observed_origin = %observed_display,
                permitted_origins = ?allowed.as_list(),
                "provider discovery supplied an endpoint outside the pinned \
                 endpoint-origin set; declare it via endpoint_origins before \
                 enforcement lands"
            );
            Ok(())
        }
        OriginCheckMode::Enforce => Err(Error::ProviderError {
            provider: provider.to_string(),
            detail: format!(
                "discovered {endpoint_kind} '{endpoint_url}' has origin \
                 '{observed_display}' outside the permitted endpoint origins {:?}",
                allowed.as_list()
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_https_origin_accepts_bare_https_origins_and_normalizes_them() {
        let plain = parse_https_origin("https://oauth2.googleapis.com")
            .expect("a bare https origin must parse");
        assert_eq!(plain, "https://oauth2.googleapis.com");

        // Explicit default port normalizes away, so both spellings pin the
        // same origin.
        let explicit_default =
            parse_https_origin("https://www.googleapis.com:443").expect("default port parses");
        assert_eq!(explicit_default, "https://www.googleapis.com");

        let non_default = parse_https_origin("https://example.test:8443")
            .expect("a non-default port is part of the origin");
        assert_eq!(non_default, "https://example.test:8443");

        // Host case normalizes to lowercase.
        let cased = parse_https_origin("https://Accounts.Example.COM").expect("cased host parses");
        assert_eq!(cased, "https://accounts.example.com");
    }

    #[test]
    fn parse_https_origin_rejects_paths_queries_fragments_and_other_schemes() {
        assert_eq!(
            parse_https_origin("https://example.com/token"),
            Err(OriginParseError::NotABareOrigin)
        );
        assert_eq!(
            parse_https_origin("https://example.com/?x=1"),
            Err(OriginParseError::NotABareOrigin)
        );
        assert_eq!(
            parse_https_origin("https://example.com#frag"),
            Err(OriginParseError::NotABareOrigin)
        );
        assert_eq!(
            parse_https_origin("http://example.com"),
            Err(OriginParseError::SchemeNotHttps {
                actual_scheme: "http".to_string()
            })
        );
        assert_eq!(
            parse_https_origin("ftp://example.com"),
            Err(OriginParseError::SchemeNotHttps {
                actual_scheme: "ftp".to_string()
            })
        );
        assert_eq!(
            parse_https_origin("not a url"),
            Err(OriginParseError::NotAnAbsoluteUrl)
        );
        assert!(parse_https_origin("https://").is_err());
    }

    #[test]
    fn origin_of_normalizes_leniently_including_plain_http_loopback() {
        // The wiremock suites serve plain-http loopback origins; lenient
        // extraction keeps them representable (recorded deviation).
        assert_eq!(
            origin_of("http://127.0.0.1:41235/jwks"),
            Some("http://127.0.0.1:41235".to_string())
        );
        assert_eq!(
            origin_of("http://127.0.0.1:41235"),
            Some("http://127.0.0.1:41235".to_string())
        );
        assert_eq!(
            origin_of("https://Example.com:443/a/b?c#d"),
            Some("https://example.com".to_string())
        );

        // Nothing parseable, nothing pinned.
        assert_eq!(origin_of(""), None);
        assert_eq!(origin_of("not a url"), None);
        assert_eq!(origin_of("mailto:someone@example.com"), None);
    }

    #[test]
    fn endpoint_origin_set_admits_issuer_configured_and_declared_members_only() {
        let set = EndpointOrigins::from_parts(
            "https://accounts.google.com",
            &["https://oauth2.googleapis.com/token"],
            &["https://www.googleapis.com".to_string()],
        );

        assert!(set.admits("https://accounts.google.com/anything"));
        assert!(set.admits("https://oauth2.googleapis.com/other/path"));
        assert!(set.admits("https://www.googleapis.com/jwks"));
        // Negative space: a lookalike host is not admitted.
        assert!(!set.admits("https://evil-googleapis.com/jwks"));
        assert!(!set.admits("http://accounts.google.com")); // scheme differs
        assert!(!set.admits("https://unrelated.example.net/x"));

        let listed = set.as_list();
        assert_eq!(listed.len(), 3, "issuer + configured + declared, deduped");
    }

    #[test]
    fn unparseable_candidates_are_never_admitted_by_a_pinned_set() {
        let set = EndpointOrigins::from_parts("https://issuer.example", &[], &[]);
        assert!(!set.admits("not a url"));
        assert!(!set.admits(""));
        // But the set itself is intact and admits its own issuer.
        assert!(set.admits("https://issuer.example/deep/path"));
        assert_eq!(set.as_list(), ["https://issuer.example"]);
    }

    #[test]
    fn duplicate_origins_collapse_and_the_cap_stops_growth() {
        let extras: Vec<String> =
            std::iter::repeat_n("https://same.example".to_string(), MAX_ENDPOINT_ORIGINS * 3)
                .collect();
        let set = EndpointOrigins::from_parts("https://issuer.example", &[], &extras);

        // Deduped to one, and the cap prevented any runaway growth.
        assert_eq!(
            set.as_list().len(),
            2,
            "issuer plus the single unique extra"
        );
        assert!(set.contains_normalized("https://same.example"));
        debug_assert!(set.as_list().len() <= MAX_ENDPOINT_ORIGINS + 1);
    }

    #[test]
    fn warn_mode_accepts_an_undeclared_cross_origin_endpoint_without_error() {
        let set = EndpointOrigins::from_parts("https://issuer.example", &[], &[]);

        // Warning mode returns Ok for an endpoint outside the set: the
        // deployment keeps working while the structured warning fires.
        check_pinned_origin(
            "test-provider",
            "jwks_uri",
            "https://undeclared.example/jwks.json",
            &set,
            OriginCheckMode::Warn,
        )
        .expect("warning mode must accept the deployment unchanged");
    }

    #[test]
    fn enforce_mode_rejects_an_undeclared_endpoint_naming_endpoint_origin_and_set() {
        let set = EndpointOrigins::from_parts(
            "https://issuer.example",
            &[],
            &["https://allowed.example".to_string()],
        );

        let err = check_pinned_origin(
            "test-provider",
            "token_endpoint",
            "https://hostile.example/token",
            &set,
            OriginCheckMode::Enforce,
        )
        .expect_err("enforcement must reject an undeclared cross-origin endpoint");

        let message = err.to_string();
        assert!(
            matches!(err, Error::ProviderError { .. }),
            "enforcement failures are provider faults: {err:?}"
        );
        assert!(
            message.contains("token_endpoint") && message.contains("hostile.example"),
            "the error names the endpoint kind and observed origin: {message}"
        );
        assert!(
            message.contains("issuer.example") && message.contains("allowed.example"),
            "the error names the full permitted set: {message}"
        );
    }

    #[test]
    fn enforce_mode_also_rejects_endpoints_with_no_derivable_origin() {
        let set = EndpointOrigins::from_parts("https://issuer.example", &[], &[]);

        let result = check_pinned_origin(
            "test-provider",
            "revocation_endpoint",
            "garbage",
            &set,
            OriginCheckMode::Enforce,
        );

        assert!(result.is_err(), "nothing unparseable may pass enforcement");
        assert!(
            result.unwrap_err().to_string().contains("<unparseable>"),
            "the failure says the origin could not be derived"
        );
    }

    #[test]
    fn declared_members_pass_under_enforcement_too() {
        let set = EndpointOrigins::from_parts(
            "https://issuer.example",
            &[],
            &["https://declared.example".to_string()],
        );

        check_pinned_origin(
            "test-provider",
            "jwks_uri",
            "https://declared.example/keys",
            &set,
            OriginCheckMode::Enforce,
        )
        .expect("a declared origin must survive enforcement");
    }
}
