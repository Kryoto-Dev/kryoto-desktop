//! "Continue with Discord" in the sign-in box, finished in the browser and
//! brought back here.
//!
//! Discord's sign-in never shows inside Kryoto (KB-0312, `discord_web`): it
//! opens in the player's browser. Until 0.3.13 that signed the browser in and
//! left Desktop signed out on the home page. Now kryo.to's sign-in box (inside
//! the Store's web view) opens a device sign-in first, polls it like Sign in
//! with your phone does, and sends only its eight-character code into the
//! browser's address (`/api/auth/discord?desktop=<code>`). Before that address
//! goes to the browser this module listens on a port of this PC's loopback
//! address and adds it (`&port=`).
//!
//! Back from Discord, kryo.to sends the browser to that port with a ticket it
//! signed for the account and the code (kryo.to lib/desktop-handoff.ts). Only
//! the PC the browser runs on can receive it: a link someone else made goes to
//! the clicker's own PC. The ticket goes back to kryo.to with the code
//! (`POST /api/auth/desktop/handoff`), which approves the sign-in; the sign-in
//! box's next poll gets the session, as a cookie in the Store's own web view.
//! The browser is sent on to kryo.to's "You are signed in. Go back to Kryoto
//! Desktop" page. Nothing that signs anybody in ever travels in an address, and
//! nothing needs a `kryoto://` link registered (Linux installs may have none).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime};
use url::Url;

/// As long as a device sign-in lives on kryo.to (lib/device-auth.ts).
const WAIT: Duration = Duration::from_secs(10 * 60);
/// The path kryo.to sends the browser to.
const PATH: &str = "/kryoto/signed-in";
/// Bumped by every new sign-in, so an older one still listening stops.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The device sign-in code kryo.to's sign-in box put in the address, when it
/// is one (the site's own alphabet, eight characters).
pub fn code_in(url: &Url) -> Option<String> {
    let raw = url.query_pairs().find(|(k, _)| k == "desktop")?.1.to_ascii_uppercase();
    let code: String = raw.chars().filter(char::is_ascii_alphanumeric).collect();
    (code.len() == 8 && code.chars().all(|c| "ABCDEFGHJKMNPQRSTVWXYZ23456789".contains(c))).then_some(code)
}

/// `url` with `port` added (any `port` already there replaced).
fn with_port(url: &Url, port: u16) -> Url {
    let mut out = url.clone();
    let pairs: Vec<(String, String)> = url.query_pairs().filter(|(k, _)| k != "port").map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    {
        let mut q = out.query_pairs_mut();
        q.clear();
        for (k, v) in &pairs {
            q.append_pair(k, v);
        }
        q.append_pair("port", &port.to_string());
    }
    out
}

/// Listen for the browser's return and give the address to open: `url` with
/// the port in it, or `url` as it was when there is no code in it or no port
/// could be had (the browser then shows kryo.to's approval page instead).
pub fn start<R: Runtime>(app: &AppHandle<R>, url: &Url) -> Url {
    let Some(code) = code_in(url) else { return url.clone() };
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l,
        Err(e) => {
            crate::logging::warn("sign-in", &format!("could not listen for the browser's sign-in on this PC: {e}"));
            return url.clone();
        }
    };
    let Ok(port) = listener.local_addr().map(|a| a.port()) else { return url.clone() };
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(app));
    let app = app.clone();
    std::thread::spawn(move || {
        let done = format!("{endpoint}/desktop/signed-in?done=1");
        let failed = format!("{endpoint}/desktop/signed-in?failed=1");
        let current = || GENERATION.load(Ordering::SeqCst) == generation;
        let signed_in = serve(&listener, current, WAIT, |ticket| redeem(&endpoint, &code, ticket), &done, &failed);
        if signed_in {
            crate::logging::info("sign-in", "the browser's Discord sign-in reached Kryoto Desktop");
            // The sign-in box polls and has it within two seconds; this only
            // brings the window back to the front, where the player is going.
            if let Some(main) = app.get_window("main") {
                let _ = main.unminimize();
                let _ = main.set_focus();
            }
            if let Some(store) = app.get_webview(crate::STORE) {
                let _ = store.eval("window.__kryoDesktopRefresh&&window.__kryoDesktopRefresh()");
            }
        }
    });
    with_port(url, port)
}

/// kryo.to approves the sign-in for the ticket's account.
fn redeem(endpoint: &str, code: &str, ticket: &str) -> bool {
    let url = format!("{endpoint}/api/auth/desktop/handoff");
    let Ok(client) = crate::net::kryoto_tls(&url, crate::net::builder())
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()
    else {
        return false;
    };
    let body = serde_json::json!({ "ticket": ticket, "code": code });
    match tauri::async_runtime::block_on(async { client.post(&url).json(&body).send().await }) {
        Ok(res) if res.status().is_success() => true,
        Ok(res) => {
            crate::logging::warn("sign-in", &format!("kryo.to did not take the browser's sign-in: {}", res.status()));
            false
        }
        Err(e) => {
            crate::logging::warn("sign-in", &format!("could not reach kryo.to to finish the browser's sign-in: {}", crate::logging::why(&e.without_url())));
            false
        }
    }
}

/// The ticket in a request's first line, when it is for our path.
fn ticket_in(head: &str) -> Option<String> {
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let target = Url::parse(&format!("http://127.0.0.1{}", parts.next()?)).ok()?;
    if target.path() != PATH {
        return None;
    }
    let ticket = target.query_pairs().find(|(k, _)| k == "t")?.1.into_owned();
    (!ticket.is_empty() && ticket.len() <= 600 && ticket.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))).then_some(ticket)
}

/// Read up to the end of the request's head (or a few kilobytes of it).
fn read_head(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while buf.len() < 8 * 1024 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// On to kryo.to's page. No referrer: the address it leaves carries the ticket.
fn send_on(stream: &mut TcpStream, to: &str) {
    let body = format!("<!doctype html><meta charset=utf-8><title>Kryoto Desktop</title><a href=\"{to}\">Continue</a>");
    let _ = write!(
        stream,
        "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn not_found(stream: &mut TcpStream) {
    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
}

/// Wait for the browser, up to `wait` or until a newer sign-in starts: each
/// ticket goes to `redeem`, and the browser on to `done` or `failed`. True
/// once one was taken.
fn serve(listener: &TcpListener, current: impl Fn() -> bool, wait: Duration, mut redeem: impl FnMut(&str) -> bool, done: &str, failed: &str) -> bool {
    if listener.set_nonblocking(true).is_err() {
        return false;
    }
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline && current() {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let head = read_head(&mut stream);
                match ticket_in(&head) {
                    Some(ticket) => {
                        let ok = redeem(&ticket);
                        send_on(&mut stream, if ok { done } else { failed });
                        if ok {
                            return true;
                        }
                    }
                    // A favicon, a probe: not ours, keep waiting.
                    None => not_found(&mut stream),
                }
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        s.parse().unwrap()
    }

    #[test]
    fn reads_only_a_real_sign_in_code() {
        assert_eq!(code_in(&u("https://kryo.to/api/auth/discord?next=%2F&desktop=ABCD-EFGH")).as_deref(), Some("ABCDEFGH"));
        assert_eq!(code_in(&u("https://kryo.to/api/auth/discord?desktop=abcdefgh")).as_deref(), Some("ABCDEFGH"));
        for bad in ["https://kryo.to/api/auth/discord", "https://kryo.to/api/auth/discord?desktop=ABCD", "https://kryo.to/api/auth/discord?desktop=ABCDEFG0"] {
            assert_eq!(code_in(&u(bad)), None, "{bad}");
        }
    }

    #[test]
    fn the_port_goes_into_the_address_once() {
        let url = with_port(&u("https://kryo.to/api/auth/discord?next=%2Flibrary&desktop=ABCDEFGH&port=1"), 50123);
        let pairs: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        assert_eq!(pairs, vec![("next".into(), "/library".into()), ("desktop".into(), "ABCDEFGH".into()), ("port".into(), "50123".into())]);
    }

    #[test]
    fn takes_a_ticket_only_from_its_own_path() {
        assert_eq!(ticket_in("GET /kryoto/signed-in?t=eyJ1Ijo.abc-_ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_deref(), Some("eyJ1Ijo.abc-_"));
        for bad in [
            "GET /favicon.ico HTTP/1.1\r\n\r\n",
            "POST /kryoto/signed-in?t=abc HTTP/1.1\r\n\r\n",
            "GET /kryoto/signed-in HTTP/1.1\r\n\r\n",
            "GET /kryoto/signed-in?t=%3Cscript%3E HTTP/1.1\r\n\r\n",
            "",
        ] {
            assert_eq!(ticket_in(bad), None, "{bad:?}");
        }
    }

    /// The whole round on this PC's loopback address, as the browser makes it:
    /// a stray request first, then the ticket, then on to kryo.to's page.
    #[test]
    fn the_browser_is_sent_on_once_the_ticket_is_taken() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut seen = Vec::new();
            let ok = serve(&listener, || true, Duration::from_secs(20), |t| {
                seen.push(t.to_string());
                t == "good.ticket"
            }, "https://kryo.to/desktop/signed-in?done=1", "https://kryo.to/desktop/signed-in?failed=1");
            (ok, seen)
        });
        let ask = |path: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        assert!(ask("/favicon.ico").starts_with("HTTP/1.1 404"));
        let refused = ask("/kryoto/signed-in?t=bad.ticket");
        assert!(refused.contains("Location: https://kryo.to/desktop/signed-in?failed=1"), "{refused}");
        let taken = ask("/kryoto/signed-in?t=good.ticket");
        assert!(taken.starts_with("HTTP/1.1 302"), "{taken}");
        assert!(taken.contains("Location: https://kryo.to/desktop/signed-in?done=1"));
        assert!(taken.contains("Referrer-Policy: no-referrer"));
        let (ok, seen) = server.join().unwrap();
        assert!(ok);
        assert_eq!(seen, vec!["bad.ticket", "good.ticket"]);
    }

    /// A newer sign-in stops an older one from listening.
    #[test]
    fn a_newer_sign_in_ends_the_older_wait() {
        static NEWER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let server = std::thread::spawn(move || serve(&listener, || !NEWER.load(Ordering::SeqCst), Duration::from_secs(30), |_| true, "x", "y"));
        std::thread::sleep(Duration::from_millis(150));
        NEWER.store(true, Ordering::SeqCst);
        let started = Instant::now();
        assert!(!server.join().unwrap());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
