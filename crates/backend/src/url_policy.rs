//! Shared outbound URL policy for service configuration and untrusted provider endpoints.

use std::net::{Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

/// Service credentials may cross plaintext only to an explicitly local development endpoint.
pub(crate) fn is_https_or_loopback_http(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => host_is_loopback(url),
        _ => false,
    }
}

fn host_is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

/// Provider response URLs are untrusted. Refuse literal addresses that can target
/// this process, its host, or an attached private network. Reject reserved
/// loopback names too. Other DNS names require deployment-level egress controls
/// because Browser Use does not publish a stable endpoint-host allowlist.
pub(crate) fn has_forbidden_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(address)) => forbidden_v4(address),
        Some(Host::Ipv6(address)) => forbidden_v6(address),
        Some(Host::Domain(host)) => {
            let host = host.trim_end_matches('.');
            host == "localhost" || host.ends_with(".localhost")
        }
        None => false,
    }
}

fn forbidden_v4(address: Ipv4Addr) -> bool {
    address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_unspecified()
        || address.is_multicast()
        || address.octets()[0] == 0
        || address.octets()[0] >= 240
        || (address.octets()[0] == 100 && address.octets()[1] & 0xc0 == 64)
}

fn forbidden_v6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return forbidden_v4(mapped);
    }
    let first = address.segments()[0];
    let unique_local = first & 0xfe00 == 0xfc00;
    let link_local = first & 0xffc0 == 0xfe80;
    address.is_loopback() || address.is_unspecified() || address.is_multicast() || unique_local || link_local
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_service_urls_are_local_only() {
        for allowed in ["https://service.example", "http://localhost:8080", "http://127.0.0.1", "http://[::1]"] {
            assert!(is_https_or_loopback_http(&Url::parse(allowed).unwrap()), "{allowed}");
        }
        for rejected in ["http://service.example", "http://10.0.0.1", "http://[fd00::1]"] {
            assert!(!is_https_or_loopback_http(&Url::parse(rejected).unwrap()), "{rejected}");
        }
    }

    #[test]
    fn provider_urls_reject_special_literal_addresses() {
        for rejected in [
            "https://127.0.0.1/path",
            "https://localhost/path",
            "https://LOCALHOST./path",
            "https://browser.localhost/path",
            "https://0.1.2.3/path",
            "https://100.64.0.1/path",
            "https://100.127.255.254/path",
            "https://255.255.255.255/path",
            "https://10.0.0.1/path",
            "https://169.254.1.1/path",
            "https://0.0.0.0/path",
            "https://224.0.0.1/path",
            "https://[::1]/path",
            "https://[fd00::1]/path",
            "https://[fe80::1]/path",
            "https://[::]/path",
            "https://[ff02::1]/path",
            "https://[::ffff:127.0.0.1]/path",
        ] {
            assert!(has_forbidden_host(&Url::parse(rejected).unwrap()), "{rejected}");
        }
        for allowed in [
            "https://provider.example/path",
            "https://localhost.example/path",
            "https://100.63.255.254/path",
            "https://100.128.0.1/path",
            "https://8.8.8.8/path",
            "https://[2606:4700:4700::1111]/path",
        ] {
            assert!(!has_forbidden_host(&Url::parse(allowed).unwrap()), "{allowed}");
        }
    }

    #[test]
    fn host_parser_canonicalization_cannot_disguise_loopback() {
        for disguised in ["https://127.1/path", "https://2130706433/path", "https://0x7f000001/path"] {
            assert!(has_forbidden_host(&Url::parse(disguised).unwrap()), "{disguised}");
        }
    }
}
