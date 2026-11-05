use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, State};
use axum::http::Request;
use axum::middleware::Next;
use axum::response::IntoResponse;
use ipnet::IpNet;
use lambda_http::request::RequestContext;

use crate::state::AppState;
use oidc_exchange_core::config::{SessionLocationConfig, MAX_TRUSTED_PROXY_HOPS};
use oidc_exchange_core::domain::{ClientAddr, SessionLocation};

/// Maximum byte length copied from a client-authored forwarding header.
pub const MAX_FORWARDED_HEADER_LEN: usize = 1_024;
/// Maximum byte length copied from a client-authored User-Agent header.
pub const MAX_USER_AGENT_LEN: usize = 256;
/// Maximum byte length copied from a client-authored device identifier.
pub const MAX_DEVICE_ID_LEN: usize = 256;
/// Maximum byte length read from an operator-named location or address header
/// before percent-decoding. `SessionLocation::new` bounds each decoded part.
pub const MAX_LOCATION_HEADER_LEN: usize = 1_024;
/// Maximum forwarding-chain entries inspected for a trusted proxy request.
pub const MAX_FORWARDED_CHAIN_ENTRIES: usize = MAX_TRUSTED_PROXY_HOPS;

/// A session location supplied directly by an in-process embedder (the FFI and
/// language bindings), carried as a request extension. HTTP clients cannot set
/// request extensions, so this is trusted exactly as far as the embedder is; it
/// takes precedence over any header-derived location.
#[derive(Clone, Debug)]
pub struct SuppliedLocation(pub SessionLocation);

/// Request-scoped audit metadata with server-established address provenance.
#[derive(Clone, Debug)]
pub struct AuditContext {
    pub client_addr: ClientAddr,
    pub user_agent: Option<String>,
    pub device_id: Option<String>,
    /// Session location read from the operator-named `[session.location]`
    /// headers; `None` unless configured and the request's peer is trusted.
    pub location: Option<SessionLocation>,
}

impl AuditContext {
    pub fn ip_address(&self) -> Option<String> {
        self.client_addr.audit_address()
    }
}

/// Builds a context from an optional observed peer. Lambda can provide its platform
/// request-context address and FFI deliberately passes `None`.
pub fn audit_context_from_request<B>(
    request: &Request<B>,
    peer: Option<SocketAddr>,
    trusted: &[IpNet],
    hops: usize,
) -> AuditContext {
    audit_context_with_location(
        request,
        peer,
        trusted,
        hops,
        &SessionLocationConfig::default(),
    )
}

/// As [`audit_context_from_request`], additionally honouring the
/// `[session.location]` header mapping. With no header named (the default)
/// this is identical to the plain form.
pub fn audit_context_with_location<B>(
    request: &Request<B>,
    peer: Option<SocketAddr>,
    trusted: &[IpNet],
    hops: usize,
    location_config: &SessionLocationConfig,
) -> AuditContext {
    let forwarded = bounded_header(request, "x-forwarded-for", MAX_FORWARDED_HEADER_LEN);
    let peer = lambda_platform_source_ip(request).or(peer);
    let peer_ip = peer.map(|peer| peer.ip());
    let mut client_addr = resolve_client_addr(peer_ip, forwarded.as_deref(), trusted, hops);
    let mut location = None;

    if location_config.is_enabled() {
        let peer_trusted = peer_ip.is_some_and(|ip| trusted.iter().any(|net| net.contains(&ip)));
        if peer_trusted || location_config.trust_headers_from_any_peer {
            location = SessionLocation::new(
                decoded_header(request, location_config.city_header.as_deref()),
                decoded_header(request, location_config.region_header.as_deref()),
                decoded_header(request, location_config.country_header.as_deref()),
            );
            if let Some(reported) = decoded_header(request, location_config.ip_header.as_deref())
                .as_deref()
                .and_then(parse_header_ip)
            {
                // A trusted-CIDR peer is a verifiable fact, so its report may
                // key a rate limit; a header accepted only through the escape
                // hatch is an operator assertion and may not.
                client_addr = if peer_trusted {
                    ClientAddr::Reported(reported)
                } else {
                    ClientAddr::asserted(reported.to_string()).unwrap_or(client_addr)
                };
            }
        }
    }

    if let Some(SuppliedLocation(supplied)) = request.extensions().get::<SuppliedLocation>() {
        location = Some(supplied.clone());
    }

    AuditContext {
        client_addr,
        user_agent: bounded_header(request, "user-agent", MAX_USER_AGENT_LEN),
        device_id: bounded_header(request, "x-device-id", MAX_DEVICE_ID_LEN),
        location,
    }
}

/// Reads a named header, bounds it, and percent-decodes it (invalid UTF-8 is
/// replaced). `None` when the header is unnamed, absent, or empty.
fn decoded_header<B>(request: &Request<B>, name: Option<&str>) -> Option<String> {
    let raw = request.headers().get(name?)?.as_bytes();
    let raw = &raw[..raw.len().min(MAX_LOCATION_HEADER_LEN)];
    let decoded = percent_encoding::percent_decode(raw)
        .decode_utf8_lossy()
        .into_owned();
    (!decoded.trim().is_empty()).then_some(decoded)
}

/// Parses a reported client address, stripping a trailing `:port` and the
/// brackets around an IPv6 literal.
fn parse_header_ip(value: &str) -> Option<IpAddr> {
    let value = value.trim();
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(ip);
    }
    if let Ok(socket) = value.parse::<SocketAddr>() {
        return Some(socket.ip());
    }
    value
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .and_then(|inner| inner.parse::<IpAddr>().ok())
}

/// Resolves only server-observed peers or forwarding chains sent by configured trusted CIDRs.
/// Selection is right-to-left: hop one is the rightmost forwarded address.
pub fn resolve_client_addr(
    peer: Option<IpAddr>,
    forwarded: Option<&str>,
    trusted: &[IpNet],
    hops: usize,
) -> ClientAddr {
    let Some(peer) = peer else {
        return ClientAddr::Unknown;
    };
    if trusted.iter().any(|network| network.contains(&peer)) {
        if let Some(forwarded) = forwarded {
            if let Some(address) = select_forwarded(forwarded, hops) {
                return ClientAddr::Forwarded(address);
            }
        }
    }
    ClientAddr::Peer(peer)
}

fn bounded_header<B>(request: &Request<B>, name: &'static str, max_len: usize) -> Option<String> {
    let value = request.headers().get(name)?.to_str().ok()?;
    Some(value.chars().take(max_len).collect())
}

fn select_forwarded(forwarded: &str, hops: usize) -> Option<IpAddr> {
    if !(1..=MAX_FORWARDED_CHAIN_ENTRIES).contains(&hops) {
        return None;
    }
    let entries: Vec<_> = forwarded.split(',').map(str::trim).collect();
    if entries.is_empty() || entries.len() > MAX_FORWARDED_CHAIN_ENTRIES {
        return None;
    }
    entries
        .iter()
        .rev()
        .nth(hops - 1)
        .and_then(|entry| entry.parse::<IpAddr>().ok())
}

/// HTTP middleware that resolves the observed connection peer using configured proxy CIDRs.
pub async fn audit_context_layer(
    State(state): State<AppState>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    let peer = lambda_platform_source_ip(&request).or_else(|| {
        request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(address)| *address)
    });
    // Parsed CIDRs are a load-time product of the typed config; borrow them.
    let trusted = state.config.server.trusted_proxies.clone();
    if request.extensions().get::<AuditContext>().is_none() {
        let context = audit_context_with_location(
            &request,
            peer,
            &trusted,
            state.config.server.trusted_proxy_hops,
            &state.config.session.location,
        );
        request.extensions_mut().insert(context);
    }
    next.run(request).await
}

/// Extracts the source address provided by the Lambda platform request context, never a
/// client-controlled forwarding header. The synthesized port is immaterial because only the
/// IP component participates in provenance and rate limiting.
fn lambda_platform_source_ip<B>(request: &Request<B>) -> Option<SocketAddr> {
    let source_ip = match request.extensions().get::<RequestContext>()? {
        RequestContext::ApiGatewayV2(context) => context.http.source_ip.as_deref(),
        RequestContext::ApiGatewayV1(context) => context.identity.source_ip.as_deref(),
        // ALB request contexts do not contain a platform source IP. Do not substitute a
        // forwarding header: absent a platform source, the request remains unknown.
        RequestContext::Alb(_) => None,
        RequestContext::WebSocket(context) => context.identity.source_ip.as_deref(),
        _ => None,
    }?;
    source_ip
        .parse::<IpAddr>()
        .ok()
        .map(|ip| SocketAddr::new(ip, 0))
}

/// FFI middleware: no transport peer exists, so never trust client forwarding headers.
pub async fn ffi_audit_context_layer(
    mut request: Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    let context = audit_context_from_request(&request, lambda_platform_source_ip(&request), &[], 1);
    request.extensions_mut().insert(context);
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::*;
    use axum::body::Body;

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[test]
    fn untrusted_peer_cannot_forge_forwarded_address() {
        let address = resolve_client_addr(
            Some(ip("198.51.100.9")),
            Some("203.0.113.4"),
            &["10.0.0.0/8".parse().unwrap()],
            1,
        );
        assert_eq!(address, ClientAddr::Peer(ip("198.51.100.9")));
    }

    #[test]
    fn trusted_peer_selects_hops_from_right() {
        let address = resolve_client_addr(
            Some(ip("10.0.0.9")),
            Some("192.0.2.10, 198.51.100.7"),
            &["10.0.0.0/8".parse().unwrap()],
            2,
        );
        assert_eq!(address, ClientAddr::Forwarded(ip("192.0.2.10")));
    }

    #[test]
    fn missing_forwarded_header_uses_trusted_peer() {
        assert_eq!(
            resolve_client_addr(
                Some(ip("10.0.0.9")),
                None,
                &["10.0.0.0/8".parse().unwrap()],
                1
            ),
            ClientAddr::Peer(ip("10.0.0.9"))
        );
    }

    #[test]
    fn chain_bounds_are_inclusive_and_fail_closed() {
        let below = (0..MAX_FORWARDED_CHAIN_ENTRIES - 1)
            .map(|_| "192.0.2.1")
            .collect::<Vec<_>>()
            .join(",");
        let at = (0..MAX_FORWARDED_CHAIN_ENTRIES)
            .map(|_| "192.0.2.1")
            .collect::<Vec<_>>()
            .join(",");
        let above = (0..MAX_FORWARDED_CHAIN_ENTRIES + 1)
            .map(|_| "192.0.2.1")
            .collect::<Vec<_>>()
            .join(",");
        assert!(select_forwarded(&below, 1).is_some());
        assert!(select_forwarded(&at, 1).is_some());
        assert!(select_forwarded(&above, 1).is_none());
    }

    #[test]
    fn header_bounds_are_inclusive() {
        let request = Request::get("/")
            .header("user-agent", "x".repeat(MAX_USER_AGENT_LEN))
            .body(Body::empty())
            .unwrap();
        assert!(bounded_header(&request, "user-agent", MAX_USER_AGENT_LEN).is_some());
        let request = Request::get("/")
            .header("user-agent", "x".repeat(MAX_USER_AGENT_LEN + 1))
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            bounded_header(&request, "user-agent", MAX_USER_AGENT_LEN)
                .expect("oversize user agent is truncated")
                .len(),
            MAX_USER_AGENT_LEN
        );
    }

    fn location_config(trust_any: bool) -> SessionLocationConfig {
        SessionLocationConfig {
            city_header: Some("x-city".into()),
            region_header: Some("x-region".into()),
            country_header: Some("x-country".into()),
            ip_header: Some("x-viewer-ip".into()),
            trust_headers_from_any_peer: trust_any,
        }
    }

    fn located_request(headers: &[(&str, &str)]) -> Request<Body> {
        let mut builder = Request::get("/");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(Body::empty()).unwrap()
    }

    fn peer(value: &str) -> Option<SocketAddr> {
        Some(SocketAddr::new(ip(value), 443))
    }

    fn trusted() -> Vec<IpNet> {
        vec!["10.0.0.0/8".parse().unwrap()]
    }

    #[test]
    fn embedder_supplied_location_applies_without_any_header_config() {
        let mut request = located_request(&[("x-city", "Ignored")]);
        let supplied = SessionLocation::new(Some("Oslo".into()), None, Some("NO".into())).unwrap();
        request
            .extensions_mut()
            .insert(SuppliedLocation(supplied.clone()));
        let context =
            audit_context_with_location(&request, None, &[], 1, &SessionLocationConfig::default());
        assert_eq!(context.location, Some(supplied));
    }

    #[test]
    fn no_header_named_changes_nothing() {
        let request = located_request(&[("x-city", "Paris"), ("x-viewer-ip", "192.0.2.1")]);
        let context = audit_context_with_location(
            &request,
            peer("10.0.0.9"),
            &trusted(),
            1,
            &SessionLocationConfig::default(),
        );
        assert_eq!(context.location, None);
        assert_eq!(context.client_addr, ClientAddr::Peer(ip("10.0.0.9")));
    }

    #[test]
    fn trusted_peer_location_is_percent_decoded_and_sanitised() {
        let long = "z".repeat(300);
        let request = located_request(&[
            ("x-city", "S%C3%A3o%20Paulo"),
            ("x-region", "  %00%07SP%0A "),
            ("x-country", &long),
        ]);
        let context = audit_context_with_location(
            &request,
            peer("10.0.0.9"),
            &trusted(),
            1,
            &location_config(false),
        );
        let location = context.location.expect("location");
        assert_eq!(location.city.as_deref(), Some("São Paulo"));
        assert_eq!(location.region.as_deref(), Some("SP"));
        assert_eq!(location.country.map(|c| c.chars().count()), Some(80));
    }

    #[test]
    fn invalid_utf8_is_replaced_not_rejected() {
        let request = located_request(&[("x-city", "Bad%FFCity")]);
        let context = audit_context_with_location(
            &request,
            peer("10.0.0.9"),
            &trusted(),
            1,
            &location_config(false),
        );
        assert_eq!(
            context.location.expect("location").city.as_deref(),
            Some("Bad\u{FFFD}City")
        );
    }

    #[test]
    fn untrusted_peer_headers_are_ignored() {
        let request = located_request(&[("x-city", "Paris"), ("x-viewer-ip", "192.0.2.1")]);
        let context = audit_context_with_location(
            &request,
            peer("198.51.100.9"),
            &trusted(),
            1,
            &location_config(false),
        );
        assert_eq!(context.location, None);
        assert_eq!(context.client_addr, ClientAddr::Peer(ip("198.51.100.9")));
    }

    #[test]
    fn trusted_peer_ip_header_is_reported_and_wins_over_forwarded() {
        let request = located_request(&[
            ("x-viewer-ip", "192.0.2.1"),
            ("x-forwarded-for", "198.51.100.7"),
        ]);
        let context = audit_context_with_location(
            &request,
            peer("10.0.0.9"),
            &trusted(),
            1,
            &location_config(false),
        );
        assert_eq!(context.client_addr, ClientAddr::Reported(ip("192.0.2.1")));
        assert_eq!(context.ip_address().as_deref(), Some("192.0.2.1"));
        assert_eq!(context.client_addr.rate_limit_key(), Some(ip("192.0.2.1")));
    }

    #[test]
    fn escape_hatch_honours_headers_but_records_the_address_as_asserted() {
        let request = located_request(&[("x-city", "Paris"), ("x-viewer-ip", "192.0.2.1")]);
        let context = audit_context_with_location(
            &request,
            peer("198.51.100.9"),
            &trusted(),
            1,
            &location_config(true),
        );
        assert_eq!(
            context.location.as_ref().expect("location").city.as_deref(),
            Some("Paris")
        );
        assert!(matches!(context.client_addr, ClientAddr::Asserted(_)));
        assert_eq!(context.ip_address().as_deref(), Some("192.0.2.1"));
        assert_eq!(context.client_addr.rate_limit_key(), None);
    }

    #[test]
    fn escape_hatch_with_a_trusted_peer_still_reports() {
        let request = located_request(&[("x-viewer-ip", "192.0.2.1")]);
        let context = audit_context_with_location(
            &request,
            peer("10.0.0.9"),
            &trusted(),
            1,
            &location_config(true),
        );
        assert_eq!(context.client_addr, ClientAddr::Reported(ip("192.0.2.1")));
    }

    #[test]
    fn ip_header_strips_port_and_brackets() {
        for (value, expected) in [
            ("192.0.2.1:8443", "192.0.2.1"),
            ("[2001:db8::1]:443", "2001:db8::1"),
            ("[2001:db8::1]", "2001:db8::1"),
            ("2001:db8::1", "2001:db8::1"),
        ] {
            let request = located_request(&[("x-viewer-ip", value)]);
            let context = audit_context_with_location(
                &request,
                peer("10.0.0.9"),
                &trusted(),
                1,
                &location_config(false),
            );
            assert_eq!(
                context.client_addr,
                ClientAddr::Reported(ip(expected)),
                "{value}"
            );
        }
    }

    #[test]
    fn absent_empty_or_unparseable_headers_leave_resolution_unchanged() {
        for headers in [
            vec![],
            vec![("x-city", ""), ("x-viewer-ip", "")],
            vec![("x-city", "%20%20"), ("x-viewer-ip", "not-an-ip")],
        ] {
            let request = located_request(&headers);
            let context = audit_context_with_location(
                &request,
                peer("10.0.0.9"),
                &trusted(),
                1,
                &location_config(false),
            );
            assert_eq!(context.location, None, "{headers:?}");
            assert_eq!(context.client_addr, ClientAddr::Peer(ip("10.0.0.9")));
        }
    }

    #[test]
    fn no_peer_is_unknown_and_never_a_rate_key() {
        let address = resolve_client_addr(None, Some("192.0.2.1"), &[], 1);
        assert_eq!(address, ClientAddr::Unknown);
        assert_eq!(address.rate_limit_key(), None);
    }
}
