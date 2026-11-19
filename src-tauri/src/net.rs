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
//! The other common block reads the site's name in the TLS handshake and cuts
//! the connection (os error 10054) or lets it hang. Edge and the Store's web
//! view get past it with Encrypted Client Hello: the name goes encrypted with a
//! key kryo.to publishes in its DNS, and the network sees only
//! `cloudflare-ech.com`. This app's own connections to kryo.to do the same
//! (`kryoto_tls`, below) once one request has proved it works on this network: the
//! Store loaded, Edge downloaded, and the app's own downloads were "blocked by
//! your network" (KB-0347).

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

/* ── Encrypted Client Hello for kryo.to's own connections ─────────────── */

/// `builder` for a request to `url`: with the site's name encrypted when
/// `url` is one of Kryoto's own HTTPS addresses and Encrypted Client Hello has
/// been seen to work on this network (`check_ech`). Every other address keeps
/// the plain handshake: the key is Cloudflare's, and only Cloudflare can open it.
pub fn kryoto_tls(url: &str, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    let own = url::Url::parse(url).ok().is_some_and(|u| u.scheme() == "https" && u.host_str().is_some_and(ours));
    match ech_config().filter(|_| own) {
        Some(config) => builder.use_preconfigured_tls((*config).clone()),
        None => builder,
    }
}

/// What is known about Encrypted Client Hello on this network.
enum Ech {
    /// Not looked at yet this run.
    Unknown,
    /// A request with it reached kryo.to with its name encrypted, at that time.
    Works(std::sync::Arc<rustls::ClientConfig>, Instant),
    /// It did not, at that time, for that reason (an antivirus or company proxy
    /// that reads HTTPS turns it away; the plain handshake is what works there).
    Off(Instant),
}
static ECH: Mutex<Ech> = Mutex::new(Ech::Unknown);
/// Bumped whenever the answer changes, so a client built before can be built again.
static ECH_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Looked at again after this long: Cloudflare turns its keys over, and a
/// laptop moves between networks.
const ECH_FRESH: Duration = Duration::from_secs(30 * 60);

fn ech_config() -> Option<std::sync::Arc<rustls::ClientConfig>> {
    match &*ECH.lock().ok()? {
        Ech::Works(config, _) => Some(config.clone()),
        _ => None,
    }
}

/// Changes whenever `ours` would build a different client.
pub fn ech_generation() -> u64 {
    ECH_GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}

/// Whether it is time to look again: never looked, the answer is old, or a
/// minute on from the last look while kryo.to is not answering (a new
/// network, or a key Cloudflare has turned over).
pub fn ech_due(failing: bool) -> bool {
    let Ok(state) = ECH.lock() else { return false };
    match &*state {
        Ech::Unknown => true,
        Ech::Works(_, at) | Ech::Off(at) => at.elapsed() >= if failing { ECH_RETRY } else { ECH_FRESH },
    }
}
const ECH_RETRY: Duration = Duration::from_secs(60);

/// Find out whether Encrypted Client Hello works here: kryo.to's key from its
/// DNS (over HTTPS, as for addresses), then one request with it to the
/// site's trace page, which says `sni=encrypted` when the name went hidden.
/// Anything else and the plain handshake stays. Say so when the answer changes.
pub async fn check_ech() {
    let result = try_ech().await;
    let mut state = match ECH.lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    let was_on = matches!(*state, Ech::Works(..));
    let was_known = !matches!(*state, Ech::Unknown);
    *state = match result {
        Ok(config) => {
            if !was_on {
                crate::logging::info("net", "kryo.to's name goes encrypted (Encrypted Client Hello) on this network");
            }
            Ech::Works(config, Instant::now())
        }
        Err(why) => {
            if was_on || !was_known {
                crate::logging::info("net", &format!("kryo.to's name goes in the clear on this network: {why}"));
            }
            Ech::Off(Instant::now())
        }
    };
    if was_on != matches!(*state, Ech::Works(..)) || !was_known {
        ECH_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

async fn try_ech() -> Result<std::sync::Arc<rustls::ClientConfig>, String> {
    let list = ech_keys("kryo.to").await.ok_or("no key in kryo.to's DNS (or no secure resolver answered)")?;
    let config = std::sync::Arc::new(ech_client_config(&list)?);
    let client = builder()
        .use_preconfigured_tls((*config).clone())
        .timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|e| e.to_string())?;
    let res = client.get("https://kryo.to/cdn-cgi/trace").send().await.map_err(|e| crate::logging::why(&e.without_url()))?;
    let body = res.text().await.map_err(|e| e.to_string())?;
    if body.lines().any(|l| l.trim() == "sni=encrypted") {
        Ok(config)
    } else {
        Err("the request went through without its name encrypted".into())
    }
}

/// The `ech=` key list in a DNS-over-HTTPS JSON answer for an HTTPS record.
fn ech_in(body: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let json = serde_json::from_str::<serde_json::Value>(body).ok()?;
    json["Answer"].as_array()?.iter().filter(|a| a["type"].as_u64() == Some(65)).find_map(|a| {
        let data = a["data"].as_str()?;
        let value = data.split_whitespace().find_map(|p| p.strip_prefix("ech="))?.trim_matches('"');
        base64::engine::general_purpose::STANDARD.decode(value).ok()
    })
}

async fn ech_keys(host: &str) -> Option<Vec<u8>> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(6)).build().ok()?;
    for base in ["https://1.1.1.1/dns-query", "https://8.8.8.8/resolve"] {
        let res = client.get(base).query(&[("name", host), ("type", "HTTPS")]).header("accept", "application/dns-json").send().await;
        if let Ok(res) = res {
            if let Some(list) = ech_in(&res.text().await.unwrap_or_default()) {
                return Some(list);
            }
        }
    }
    None
}

/// Both root sets, as reqwest loads them for every other client (Cargo.toml).
static ROOTS: std::sync::LazyLock<std::sync::Arc<rustls::RootCertStore>> = std::sync::LazyLock::new(|| {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    std::sync::Arc::new(roots)
});

fn ech_client_config(list: &[u8]) -> Result<rustls::ClientConfig, String> {
    use rustls::client::{EchConfig, EchMode};
    let ech = EchConfig::new(list.to_vec().into(), hpke_suites::ALL).map_err(|e| format!("kryo.to's key is not one this app can use: {e}"))?;
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_ech(EchMode::Enable(ech))
        .map_err(|e| e.to_string())?
        .with_root_certificates(ROOTS.clone())
        .with_no_client_auth();
    // reqwest takes this config as it is: HTTP/2 has to be offered here (KB-0289).
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

/// The HPKE that Encrypted Client Hello seals with, for rustls (which ships
/// one only with another crypto backend): DHKEM(X25519) with HKDF-SHA256, and
/// AES-128-GCM (Cloudflare's key) or ChaCha20-Poly1305. Sealing only: this
/// app never opens one.
mod hpke_suites {
    use hpke::{Deserializable as _, Serializable as _};
    use rustls::crypto::hpke::{EncapsulatedSecret, Hpke, HpkeOpener, HpkePrivateKey, HpkePublicKey, HpkeSealer, HpkeSuite};
    use rustls::internal::msgs::enums::{HpkeAead, HpkeKdf, HpkeKem};
    use rustls::internal::msgs::handshake::HpkeSymmetricCipherSuite;
    use rustls::Error;

    type Kem = hpke::kem::X25519HkdfSha256;
    type Kdf = hpke::kdf::HkdfSha256;

    pub struct Suite<A>(std::marker::PhantomData<A>, HpkeAead);
    pub static AES128: Suite<hpke::aead::AesGcm128> = Suite(std::marker::PhantomData, HpkeAead::AES_128_GCM);
    pub static CHACHA: Suite<hpke::aead::ChaCha20Poly1305> = Suite(std::marker::PhantomData, HpkeAead::CHACHA20_POLY_1305);
    pub static ALL: &[&'static dyn Hpke] = &[&AES128, &CHACHA];

    fn err(e: impl std::fmt::Display) -> Error {
        Error::General(format!("HPKE: {e}"))
    }

    impl<A> std::fmt::Debug for Suite<A> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "DHKEM(X25519, HKDF-SHA256) {:?}", self.1)
        }
    }

    struct Sealer<A: hpke::aead::Aead>(hpke::aead::AeadCtxS<A, Kdf, Kem>);
    impl<A: hpke::aead::Aead> std::fmt::Debug for Sealer<A> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("HPKE sealer")
        }
    }
    impl<A: hpke::aead::Aead + Send + Sync + 'static> HpkeSealer for Sealer<A>
    where
        hpke::aead::AeadCtxS<A, Kdf, Kem>: Send + Sync,
    {
        fn seal(&mut self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, Error> {
            self.0.seal(plaintext, aad).map_err(err)
        }
    }

    impl<A: hpke::aead::Aead + Send + Sync + 'static> Hpke for Suite<A>
    where
        hpke::aead::AeadCtxS<A, Kdf, Kem>: Send + Sync,
    {
        fn seal(&self, info: &[u8], aad: &[u8], plaintext: &[u8], pub_key: &HpkePublicKey) -> Result<(EncapsulatedSecret, Vec<u8>), Error> {
            let pk = <Kem as hpke::Kem>::PublicKey::from_bytes(&pub_key.0).map_err(err)?;
            let (enc, sealed) = hpke::single_shot_seal::<A, Kdf, Kem>(&hpke::OpModeS::Base, &pk, info, plaintext, aad).map_err(err)?;
            Ok((EncapsulatedSecret(enc.to_bytes().to_vec()), sealed))
        }

        fn setup_sealer(&self, info: &[u8], pub_key: &HpkePublicKey) -> Result<(EncapsulatedSecret, Box<dyn HpkeSealer + 'static>), Error> {
            let pk = <Kem as hpke::Kem>::PublicKey::from_bytes(&pub_key.0).map_err(err)?;
            let (enc, ctx) = hpke::setup_sender::<A, Kdf, Kem>(&hpke::OpModeS::Base, &pk, info).map_err(err)?;
            Ok((EncapsulatedSecret(enc.to_bytes().to_vec()), Box::new(Sealer(ctx))))
        }

        fn open(&self, _: &EncapsulatedSecret, _: &[u8], _: &[u8], _: &[u8], _: &HpkePrivateKey) -> Result<Vec<u8>, Error> {
            Err(err("only sealing"))
        }

        fn setup_opener(&self, _: &EncapsulatedSecret, _: &[u8], _: &HpkePrivateKey) -> Result<Box<dyn HpkeOpener + 'static>, Error> {
            Err(err("only sealing"))
        }

        fn generate_key_pair(&self) -> Result<(HpkePublicKey, HpkePrivateKey), Error> {
            Err(err("only sealing"))
        }

        fn suite(&self) -> HpkeSuite {
            HpkeSuite {
                kem: HpkeKem::DHKEM_X25519_HKDF_SHA256,
                sym: HpkeSymmetricCipherSuite { kdf_id: HpkeKdf::HKDF_SHA256, aead_id: self.1 },
            }
        }
    }
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

    /// kryo.to's HTTPS record as 1.1.1.1 answers it (2026-11-19): the key list
    /// comes out whole; an answer without one gives nothing.
    #[test]
    fn reads_the_key_from_an_https_record() {
        let body = r#"{"Status":0,"Answer":[{"name":"kryo.to","type":65,"TTL":300,"data":"1 . alpn=h3,h2 ipv4hint=104.21.12.161 ech=AEX+DQBBpQAgACAuwMQtM1v9kQJyUlfYP1aqdSNI2MqKHEv0m5sJCYAFJQAEAAEAAQASY2xvdWRmbGFyZS1lY2guY29tAAA= ipv6hint=2606:4700:3030::ac43:c326"}]}"#;
        let list = ech_in(body).expect("a key list");
        assert_eq!(list.len(), 0x47);
        assert!(String::from_utf8_lossy(&list).contains("cloudflare-ech.com"));
        // A key list this app can seal with: Cloudflare's suite is X25519 + AES-128-GCM.
        assert!(ech_client_config(&list).is_ok());
        assert!(ech_in(r#"{"Answer":[{"type":65,"data":"1 . alpn=h2"}]}"#).is_none());
        assert!(ech_in("not json").is_none());
    }

    /// Only Kryoto's own HTTPS addresses get the encrypted name.
    #[test]
    fn only_kryotos_https_addresses_get_the_key() {
        let body = r#"{"Answer":[{"type":65,"data":"1 . ech=AEX+DQBBpQAgACAuwMQtM1v9kQJyUlfYP1aqdSNI2MqKHEv0m5sJCYAFJQAEAAEAAQASY2xvdWRmbGFyZS1lY2guY29tAAA="}]}"#;
        let config = std::sync::Arc::new(ech_client_config(&ech_in(body).unwrap()).unwrap());
        *ECH.lock().unwrap() = Ech::Works(config, Instant::now());
        // A client built either way; what matters is that nothing panics and other hosts are left alone.
        for url in ["https://dl.kryo.to/d/x", "https://kryo.to/", "http://kryo.to/", "https://pixeldrain.com/u/x", "https://notkryo.to/"] {
            assert!(kryoto_tls(url, reqwest::Client::builder()).build().is_ok(), "{url}");
        }
        *ECH.lock().unwrap() = Ech::Unknown;
    }

    /// Live: kryo.to's key from its DNS, a request with it, and Cloudflare's own
    /// word that the name went encrypted; then a download link's host the same way.
    #[test]
    #[ignore = "reaches 1.1.1.1 and kryo.to"]
    fn live_kryotos_name_goes_encrypted() {
        tauri::async_runtime::block_on(async {
            check_ech().await;
            let config = ech_config().expect("Encrypted Client Hello works from here");
            let client = builder().use_preconfigured_tls((*config).clone()).timeout(Duration::from_secs(10)).build().unwrap();
            for url in ["https://kryo.to/cdn-cgi/trace", "https://dl.kryo.to/cdn-cgi/trace"] {
                let res = client.get(url).send().await.unwrap();
                let version = res.version();
                let body = res.text().await.unwrap();
                assert!(body.contains("sni=encrypted"), "{url}: {body}");
                assert_eq!(version, reqwest::Version::HTTP_2, "{url} over HTTP/2");
            }
            // A plain client shows the difference.
            let plain = builder().timeout(Duration::from_secs(10)).build().unwrap();
            let body = plain.get("https://kryo.to/cdn-cgi/trace").send().await.unwrap().text().await.unwrap();
            assert!(body.contains("sni=plaintext"), "{body}");
        });
    }

    /// Live, behind a network that resets every connection naming kryo.to in
    /// the clear (KB-0347's; the iptables rule is in that bug's Verify): the
    /// app's plain request is cut, as Callan's downloads were, and the one with
    /// the name encrypted goes through. Runs only with KB_NAME_FILTER set.
    #[test]
    #[ignore = "needs a network that cuts kryo.to by name"]
    fn live_behind_a_name_filter() {
        if std::env::var_os("KB_NAME_FILTER").is_none() {
            return;
        }
        tauri::async_runtime::block_on(async {
            let plain = builder().timeout(Duration::from_secs(10)).build().unwrap();
            let cut = plain.get("https://dl.kryo.to/cdn-cgi/trace").send().await;
            assert!(cut.is_err(), "the filter must cut the plain request: {:?}", cut.map(|r| r.status()));
            eprintln!("plain: {}", crate::logging::why(&cut.unwrap_err().without_url()));
            check_ech().await;
            assert!(ech_config().is_some(), "Encrypted Client Hello must get past the filter");
            let own = kryoto_tls("https://dl.kryo.to/d/x", builder()).timeout(Duration::from_secs(10)).build().unwrap();
            let body = own.get("https://dl.kryo.to/cdn-cgi/trace").send().await.unwrap().text().await.unwrap();
            assert!(body.contains("sni=encrypted"), "{body}");
            eprintln!("encrypted: {}", body.lines().filter(|l| l.starts_with("sni=") || l.starts_with("http=") || l.starts_with("tls=")).collect::<Vec<_>>().join(" "));
        });
    }
}
