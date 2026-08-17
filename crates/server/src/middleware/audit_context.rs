use std::net::{IpAddr, SocketAddr};

use axum::http::Request;
use axum::middleware::Next;
use axum::response::IntoResponse;
use ipnet::IpNet;
use oidc_exchange_core::config::MAX_TRUSTED_PROXY_HOPS;
use oidc_exchange_core::domain::ClientAddr;

/// Maximum byte length copied from a client-authored forwarding header.
pub const MAX_FORWARDED_HEADER_LEN: usize = 1_024;
/// Maximum byte length copied from a client-authored User-Agent header.
pub const MAX_USER_AGENT_LEN: usize = 512;
/// Maximum byte length copied from a client-authored device identifier.
pub const MAX_DEVICE_ID_LEN: usize = 256;
/// Maximum forwarding-chain entries inspected for a trusted proxy request.
pub const MAX_FORWARDED_CHAIN_ENTRIES: usize = MAX_TRUSTED_PROXY_HOPS;

/// Request-scoped audit metadata with server-established address provenance.
#[derive(Clone, Debug)]
pub struct AuditContext {
    pub client_addr: ClientAddr,
    pub user_agent: Option<String>,
    pub device_id: Option<String>,
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
    let forwarded = bounded_header(request, "x-forwarded-for", MAX_FORWARDED_HEADER_LEN);
    let client_addr = resolve_client_addr(
        peer.map(|peer| peer.ip()),
        forwarded.as_deref(),
        trusted,
        hops,
    );
    AuditContext {
        client_addr,
        user_agent: bounded_header(request, "user-agent", MAX_USER_AGENT_LEN),
        device_id: bounded_header(request, "x-device-id", MAX_DEVICE_ID_LEN),
    }
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
    (value.len() <= max_len).then(|| value.to_owned())
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

/// Default middleware for in-process and FFI callers: no peer is available, so forwarding
/// headers are audit-only asserted data and never form an address rate key.
pub async fn audit_context_layer(
    mut request: Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    let client_addr = bounded_header(&request, "x-forwarded-for", MAX_FORWARDED_HEADER_LEN)
        .and_then(ClientAddr::asserted)
        .unwrap_or(ClientAddr::Unknown);
    let context = AuditContext {
        client_addr,
        user_agent: bounded_header(&request, "user-agent", MAX_USER_AGENT_LEN),
        device_id: bounded_header(&request, "x-device-id", MAX_DEVICE_ID_LEN),
    };
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
        assert!(bounded_header(&request, "user-agent", MAX_USER_AGENT_LEN).is_none());
    }

    #[test]
    fn no_peer_is_unknown_and_never_a_rate_key() {
        let address = resolve_client_addr(None, Some("192.0.2.1"), &[], 1);
        assert_eq!(address, ClientAddr::Unknown);
        assert_eq!(address.rate_limit_key(), None);
    }
}
