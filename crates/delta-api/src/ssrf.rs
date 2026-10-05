//! Guards for outbound requests to user-supplied URLs (webhooks, federation).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Whether an address must not be reached on behalf of users: anything not
/// publicly routable (loopback, private, link-local, unique-local, CGNAT,
/// unspecified, multicast, reserved, ...), including IPv4 addresses embedded
/// in IPv6 (`::ffff:a.b.c.d`, `::a.b.c.d`, NAT64 `64:ff9b::a.b.c.d`).
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4() {
                // IPv4-mapped or IPv4-compatible (this also covers `::` and `::1`).
                return is_private_v4(v4);
            }
            let s = v6.segments();
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [a, b] = s[6].to_be_bytes();
                let [c, d] = s[7].to_be_bytes();
                return is_private_v4(Ipv4Addr::new(a, b, c, d));
            }
            v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (s[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (s[0] & 0xffc0) == 0xfec0 // site-local fec0::/10
                || s[0] == 0x2001 && s[1] == 0x0db8 // documentation
        }
    }
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0 // "this network" 0.0.0.0/8
        || (a == 100 && (b & 0xc0) == 64) // shared address space 100.64.0.0/10
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments 192.0.0.0/24
        || (a == 198 && (b & 0xfe) == 18) // benchmarking 198.18.0.0/15
        || a >= 240 // reserved 240.0.0.0/4
}

/// Syntactic check: rejects unparseable URLs, schemes other than HTTP(S),
/// local host names, and literal private addresses. Host names are not
/// resolved here; use [`guarded_client`] before connecting.
pub fn is_private_url(url_str: &str) -> bool {
    let Ok(url) = url::Url::parse(url_str) else {
        return true; // Reject unparseable URLs
    };
    if !matches!(url.scheme(), "http" | "https") {
        return true;
    }
    match url.host() {
        None => true,
        Some(url::Host::Ipv4(ip)) => is_private_v4(ip),
        Some(url::Host::Ipv6(ip)) => is_private_ip(IpAddr::V6(ip)),
        Some(url::Host::Domain(domain)) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            domain == "localhost"
                || domain.ends_with(".localhost")
                || domain.ends_with(".local")
                || domain.ends_with(".internal")
        }
    }
}

/// Resolve the URL's host and vet every address it resolves to.
pub async fn resolve_public(url_str: &str) -> Result<Vec<SocketAddr>, String> {
    if is_private_url(url_str) {
        return Err("URL targets a private or local address".into());
    }
    let url = url::Url::parse(url_str).map_err(|e| e.to_string())?;
    let port = url.port_or_known_default().ok_or("URL has no port")?;
    let addrs: Vec<SocketAddr> = match url.host() {
        Some(url::Host::Domain(domain)) => tokio::net::lookup_host((domain, port))
            .await
            .map_err(|e| format!("DNS lookup failed: {e}"))?
            .collect(),
        Some(url::Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        Some(url::Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        None => return Err("URL has no host".into()),
    };
    if addrs.is_empty() {
        return Err("host did not resolve".into());
    }
    if addrs.iter().any(|a| is_private_ip(a.ip())) {
        return Err("URL resolves to a private or local address".into());
    }
    Ok(addrs)
}

/// An HTTP client for one user-supplied URL: the host must resolve only to
/// public addresses, connections are pinned to those vetted addresses (no
/// DNS rebinding between check and connect), and redirects are not followed
/// (each hop would need vetting).
pub async fn guarded_client(url_str: &str, timeout: Duration) -> Result<reqwest::Client, String> {
    let addrs = resolve_public(url_str).await?;
    let url = url::Url::parse(url_str).map_err(|e| e.to_string())?;
    let mut builder = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none());
    if let Some(url::Host::Domain(domain)) = url.host() {
        builder = builder.resolve_to_addrs(domain, &addrs);
    }
    builder.build().map_err(|e| e.to_string())
}

/// Read at most `limit` bytes of a response body.
pub async fn read_capped(mut response: reqwest::Response, limit: usize) -> Vec<u8> {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        let room = limit - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if body.len() >= limit {
            break;
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_private_literals() {
        for url in [
            "http://localhost/hook",
            "http://foo.localhost/hook",
            "http://127.0.0.1:3000/hook",
            "http://127.255.255.255/hook",
            "http://0.0.0.0/hook",
            "http://0x7f.1/hook", // WHATWG parsing normalizes to 127.0.0.1
            "http://10.0.0.1/hook",
            "http://172.16.0.1/hook",
            "http://192.168.1.1/hook",
            "http://169.254.169.254/latest/meta-data",
            "http://100.64.0.1/hook",
            "http://[::1]/hook",
            "http://[::]/hook",
            "http://[::ffff:127.0.0.1]/hook",
            "http://[::ffff:7f00:1]/hook",
            "http://[::ffff:a9fe:a9fe]/hook",
            "http://[64:ff9b::a9fe:a9fe]/hook",
            "http://[fe80::1]/hook",
            "http://[fc00::1]/hook",
            "http://[fd12::1]/hook",
            "http://myhost.local/hook",
            "http://service.internal./hook",
            "ftp://example.com/hook",
            "ext::sh -c id",
            "not-a-url",
            "",
        ] {
            assert!(is_private_url(url), "{url} should be rejected");
        }
    }

    #[test]
    fn test_public_urls() {
        for url in [
            "https://example.com/hook",
            "https://api.github.com/webhook",
            "http://8.8.8.8/hook",
            "http://172.15.0.1/hook",
            "http://172.32.0.1/hook",
            "http://[2606:4700::1111]/hook",
        ] {
            assert!(!is_private_url(url), "{url} should be allowed");
        }
    }

    #[tokio::test]
    async fn test_resolve_public_rejects_names_resolving_to_loopback() {
        // "localhost" is rejected syntactically; resolution catches any other
        // name pointing at a private address, e.g. via /etc/hosts.
        assert!(resolve_public("http://localhost/x").await.is_err());
        assert!(resolve_public("http://127.0.0.1/x").await.is_err());
        assert!(resolve_public("http://8.8.8.8/x").await.is_ok());
    }
}
