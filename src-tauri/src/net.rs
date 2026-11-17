//! Looking up Kryoto's own addresses when this network's DNS will not.
//!
//! The commonest way a network blocks a site is in its DNS: the internet
//! provider's (or a router's, or a filter's) answer for kryo.to is nothing, or
//! an address that goes nowhere (0.0.0.0, 127.0.0.1, one inside the home
//! network). The Store's web view often gets past that by itself, since
//! Chromium asks a secure resolver when it can, while this app's own requests
//! took the system's answer: the Store loaded and downloads "kept dropping",
//! saves could not be checked and chat never connected (KB-0328).
//!
//! So every client this app builds (`builder`) resolves through `Resolver`:
//! the system's answer as always, and only for Kryoto's own names, when that
//! answer is missing or unusable, the same question put to a public resolver
//! over HTTPS (Cloudflare's 1.1.1.1, then Google's 8.8.8.8, by address so no
//! name has to be looked up to ask). Nothing changes on a network whose DNS
//! answers properly, and no other site's lookup ever leaves the system's.
//!
//! This does not get around a network that cuts the connection itself (by the
//! site's name in the TLS handshake); that still needs a VPN.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Kryoto's own names: the site and everything under it.
fn ours(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "kryo.to" || host.ends_with(".kryo.to")
}

/// An address a request can really go to: not "nowhere", this PC, or the
/// local network, which is what a DNS block answers with.
fn reachable(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !(v4.is_unspecified() || v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_broadcast()),
        IpAddr::V6(v6) => !(v6.is_unspecified() || v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80),
    }
}

/// Whether the system's answer for one of our names can be used as it is.
fn usable(system: &Result<Vec<SocketAddr>, String>) -> bool {
    system.as_ref().is_ok_and(|addrs| addrs.iter().any(|a| reachable(&a.ip())))
}

/// Answers from the public resolvers, kept for a few minutes.
type Kept = HashMap<String, (Instant, Vec<IpAddr>)>;
static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
const KEEP: Duration = Duration::from_secs(300);
/// Said once a run, so the log shows this network's DNS is the block.
static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The addresses in a DNS-over-HTTPS JSON answer (`Answer[].data` of A and AAAA records).
fn addresses_in(body: &str) -> Vec<IpAddr> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else { return Vec::new() };
    json["Answer"]
        .as_array()
        .map(|answers| {
            answers
                .iter()
                .filter(|a| matches!(a["type"].as_u64(), Some(1 | 28)))
                .filter_map(|a| a["data"].as_str().and_then(|d| d.parse::<IpAddr>().ok()))
                .filter(reachable)
                .collect()
        })
        .unwrap_or_default()
}

/// Ask the public resolvers over HTTPS. Its own plain client: this lookup
/// must not go through `Resolver` again.
async fn ask_public(host: &str) -> Vec<IpAddr> {
    if let Some((at, kept)) = KEPT.lock().ok().and_then(|k| k.as_ref().and_then(|m| m.get(host).cloned())) {
        if at.elapsed() < KEEP {
            return kept;
        }
    }
    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(6)).build() else { return Vec::new() };
    let mut found = Vec::new();
    for base in ["https://1.1.1.1/dns-query", "https://8.8.8.8/resolve"] {
        for kind in ["A", "AAAA"] {
            let res = client.get(base).query(&[("name", host), ("type", kind)]).header("accept", "application/dns-json").send().await;
            if let Ok(res) = res {
                if res.status().is_success() {
                    found.extend(addresses_in(&res.text().await.unwrap_or_default()));
                }
            }
        }
        if !found.is_empty() {
            break;
        }
    }
    if !found.is_empty() {
        if let Ok(mut kept) = KEPT.lock() {
            kept.get_or_insert_with(HashMap::new).insert(host.to_string(), (Instant::now(), found.clone()));
        }
    }
    found
}

/// The addresses to use for `host`, given what the system's DNS said.
async fn settle(host: &str, system: Result<Vec<SocketAddr>, String>) -> Result<Vec<SocketAddr>, String> {
    if !ours(host) || usable(&system) {
        return system;
    }
    let public = ask_public(host).await;
    if public.is_empty() {
        // Nothing better to offer: what the system said stands.
        return system;
    }
    if !SAID.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let said = match &system {
            Ok(addrs) => format!("answered {}", addrs.iter().map(|a| a.ip().to_string()).collect::<Vec<_>>().join(", ")),
            Err(e) => format!("failed ({e})"),
        };
        crate::logging::warn("net", &format!("this network's DNS {said} for {host}; using a public resolver's answer for Kryoto's addresses instead"));
    }
    Ok(public.into_iter().map(|ip| SocketAddr::new(ip, 0)).collect())
}

/// reqwest's resolver: see the module's words.
pub struct Resolver;

impl reqwest::dns::Resolve for Resolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let system = tokio::net::lookup_host((host.as_str(), 0)).await.map(|a| a.collect::<Vec<_>>()).map_err(|e| e.to_string());
            let addrs = settle(&host, system).await.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { format!("dns error: {e}").into() })?;
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Every HTTP client in the app starts here.
pub fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().dns_resolver(std::sync::Arc::new(Resolver))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ip: &str) -> SocketAddr {
        SocketAddr::new(ip.parse().unwrap(), 0)
    }

    #[test]
    fn only_kryotos_own_names_are_looked_up_elsewhere() {
        for host in ["kryo.to", "dl.kryo.to", "chat.kryo.to", "KRYO.TO."] {
            assert!(ours(host), "{host}");
        }
        for host in ["notkryo.to", "kryo.to.example.com", "github.com", "1.1.1.1"] {
            assert!(!ours(host), "{host}");
        }
    }

    #[test]
    fn a_blocked_answer_is_told_from_a_real_one() {
        assert!(usable(&Ok(vec![at("104.21.5.9")])));
        assert!(usable(&Ok(vec![at("0.0.0.0"), at("172.67.195.38")])));
        for blocked in ["0.0.0.0", "127.0.0.1", "192.168.1.1", "10.0.0.1", "::", "::1", "fd00::1"] {
            assert!(!usable(&Ok(vec![at(blocked)])), "{blocked}");
        }
        assert!(!usable(&Ok(Vec::new())));
        assert!(!usable(&Err("No such host is known".into())));
    }

    #[test]
    fn reads_a_resolvers_answer() {
        let body = r#"{"Status":0,"Answer":[{"name":"kryo.to","type":1,"TTL":300,"data":"104.21.5.9"},{"name":"kryo.to","type":5,"data":"x.example."},{"name":"kryo.to","type":28,"data":"2606:4700:3030::6815:509"},{"name":"kryo.to","type":1,"data":"0.0.0.0"}]}"#;
        assert_eq!(addresses_in(body), vec!["104.21.5.9".parse::<IpAddr>().unwrap(), "2606:4700:3030::6815:509".parse().unwrap()]);
        assert!(addresses_in("<html>blocked</html>").is_empty());
    }

    /// Another site's lookup is never touched, whatever the system said.
    #[test]
    fn other_sites_keep_the_systems_answer() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let blocked: Result<Vec<SocketAddr>, String> = Ok(vec![at("0.0.0.0")]);
        assert_eq!(rt.block_on(settle("github.com", blocked.clone())), blocked);
    }

    /// Against the real internet: `cargo test --lib net::tests::live -- --ignored`.
    /// A network whose DNS answers 0.0.0.0 for kryo.to (the way a DNS block
    /// does), and a real request to the site through it.
    #[test]
    #[ignore]
    fn live_a_blocked_dns_answer_still_reaches_kryoto() {
        struct Blocked;
        impl reqwest::dns::Resolve for Blocked {
            fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
                let host = name.as_str().to_string();
                Box::pin(async move {
                    let addrs = settle(&host, Ok(vec![SocketAddr::new("0.0.0.0".parse().unwrap(), 0)])).await.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
                    Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
                })
            }
        }
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let through = reqwest::Client::builder().dns_resolver(std::sync::Arc::new(Blocked)).timeout(Duration::from_secs(15)).build().unwrap();
            let res = through.get("https://kryo.to/api/health").send().await.expect("kryo.to reached through the public resolver's answer");
            assert!(res.status().as_u16() < 500, "{}", res.status());
            let dl = through.get("https://dl.kryo.to/").send().await.expect("dl.kryo.to reached too");
            assert!(dl.status().as_u16() < 500, "{}", dl.status());
            // The control: the same blocked answer, taken as it is, reaches nothing.
            struct Raw;
            impl reqwest::dns::Resolve for Raw {
                fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
                    Box::pin(async { Ok(Box::new(vec![SocketAddr::new("0.0.0.0".parse().unwrap(), 0)].into_iter()) as reqwest::dns::Addrs) })
                }
            }
            let raw = reqwest::Client::builder().dns_resolver(std::sync::Arc::new(Raw)).timeout(Duration::from_secs(6)).build().unwrap();
            assert!(raw.get("https://kryo.to/api/health").send().await.is_err(), "the blocked answer alone must fail");
        });
    }
}
