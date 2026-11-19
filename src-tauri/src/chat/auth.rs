//! Chat's own sign-in for this PC.
//!
//! The Store web view is signed in with a cookie the native side never sees,
//! and that is how it should stay. Chat needs a token the Rust side holds, so
//! it uses kryo.to's device sign-in (RFC 8628, `lib/device-auth.ts`, client
//! `desktop`): this app asks for a code, the signed-in Store page approves that
//! exact code, and the app collects a session token of its own, kept in the OS
//! secure store. The person confirms in the shell first ("Turn on chat on this
//! PC?"); the code never comes from a page, so it cannot be a phishing code.

use std::time::{Duration, Instant};

use serde::Deserialize;
use zeroize::Zeroizing;

pub struct Started {
    pub device_code: Zeroizing<String>,
    pub user_code: String,
    interval: u64,
    expires_in: u64,
}

#[derive(Deserialize)]
struct StartResponse {
    device_code: String,
    user_code: String,
    interval: u64,
    expires_in: u64,
}

/// For kryo.to (`base`), with its name encrypted where that works (net.rs `kryoto_tls`, KB-0347).
fn client(base: &str) -> reqwest::Client {
    crate::net::kryoto_tls(base, crate::net::builder())
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION"), " chat"))
        .build()
        .expect("HTTP client")
}

async fn error_of(r: reqwest::Response) -> String {
    #[derive(Deserialize)]
    struct E {
        error: Option<String>,
    }
    let status = r.status();
    r.json::<E>().await.ok().and_then(|e| e.error).unwrap_or_else(|| format!("kryo.to answered {status}."))
}

pub async fn start(base: &str, device_name: &str) -> Result<Started, String> {
    let r = client(base)
        .post(format!("{base}/api/auth/device"))
        .json(&serde_json::json!({ "client": "desktop", "device_name": device_name }))
        .send()
        .await
        .map_err(|e| format!("Could not reach kryo.to: {e}"))?;
    if !r.status().is_success() {
        return Err(error_of(r).await);
    }
    let s: StartResponse = r.json().await.map_err(|_| "kryo.to sent an unexpected answer.".to_string())?;
    Ok(Started {
        device_code: Zeroizing::new(s.device_code),
        user_code: s.user_code,
        interval: s.interval.clamp(1, 10),
        expires_in: s.expires_in.min(900),
    })
}

/// The script the signed-in Store page runs to approve our code.
pub fn approve_script(user_code: &str) -> Result<String, String> {
    // The code goes into JavaScript: allow only what codes are made of.
    if user_code.is_empty() || user_code.len() > 24 || !user_code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("kryo.to sent an unexpected sign-in code.".into());
    }
    Ok(format!(
        "fetch('/api/auth/device/approve',{{method:'POST',credentials:'include',headers:{{'content-type':'application/json'}},body:JSON.stringify({{code:'{user_code}'}})}}).catch(()=>{{}})"
    ))
}

/// Poll until the code is approved; returns the new session token.
pub async fn wait_for_token(base: &str, started: &Started) -> Result<Zeroizing<String>, String> {
    #[derive(Deserialize)]
    struct Poll {
        status: String,
        token: Option<String>,
    }
    let http = client(base);
    let deadline = Instant::now() + Duration::from_secs(started.expires_in.min(90));
    let mut interval = started.interval;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        let r = http
            .post(format!("{base}/api/auth/device/token"))
            .json(&serde_json::json!({ "device_code": started.device_code.as_str() }))
            .send()
            .await;
        let Ok(r) = r else { continue };
        if !r.status().is_success() {
            continue;
        }
        let Ok(p) = r.json::<Poll>().await else { continue };
        match p.status.as_str() {
            "ok" => return p.token.map(Zeroizing::new).ok_or_else(|| "kryo.to sent no token.".to_string()),
            "authorization_pending" => {}
            "slow_down" => interval = (interval + 1).min(10),
            "access_denied" => return Err("The sign-in was refused.".into()),
            _ => return Err("The sign-in expired. Try again.".into()),
        }
    }
    Err("The Store did not confirm the sign-in. Make sure you are signed in to kryo.to in the Store, then try again.".into())
}

pub struct Me {
    pub id: u64,
    pub anonymous: bool,
}

/// Who a token belongs to; `None` when it is no longer valid.
pub async fn me(base: &str, token: &str) -> Result<Option<Me>, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct User {
        id: String,
        #[serde(default)]
        is_anonymous: bool,
    }
    #[derive(Deserialize)]
    struct Body {
        user: Option<User>,
    }
    let r = client(base)
        .get(format!("{base}/api/auth/me"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Could not reach kryo.to: {e}"))?;
    if !r.status().is_success() {
        return Err(error_of(r).await);
    }
    let body: Body = r.json().await.map_err(|_| "kryo.to sent an unexpected answer.".to_string())?;
    Ok(match body.user {
        None => None,
        Some(u) => Some(Me {
            id: u.id.parse().map_err(|_| "kryo.to sent an unexpected account id.".to_string())?,
            anonymous: u.is_anonymous,
        }),
    })
}

/// GIF search through kryo.to's proxy (supporters only, decided there).
pub async fn gif_search(base: &str, token: &str, query: &str) -> Result<serde_json::Value, String> {
    let q: String = query.trim().chars().take(80).collect();
    let r = client(base)
        .get(format!("{base}/api/gifs/search"))
        .query(&[("q", q.as_str())])
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Could not reach kryo.to: {e}"))?;
    if !r.status().is_success() {
        return Err(error_of(r).await);
    }
    r.json().await.map_err(|_| "kryo.to sent an unexpected answer.".to_string())
}

/// Ask to message someone who is not a friend (kryo.to decides from their
/// settings). Answers `{ status, userId, username, name }`.
pub async fn message_request(base: &str, token: &str, username: &str) -> Result<serde_json::Value, String> {
    let username: String = username.trim().trim_start_matches('@').chars().take(64).collect();
    if username.is_empty() {
        return Err("Whose username?".into());
    }
    let r = client(base)
        .post(format!("{base}/api/chat/message-requests"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "username": username }))
        .send()
        .await
        .map_err(|e| format!("Could not reach kryo.to: {e}"))?;
    if !r.status().is_success() {
        return Err(error_of(r).await);
    }
    r.json().await.map_err(|_| "kryo.to sent an unexpected answer.".to_string())
}

/// Accept or decline a message request someone sent this account.
pub async fn message_request_respond(base: &str, token: &str, user_id: u64, accept: bool) -> Result<(), String> {
    let r = client(base)
        .post(format!("{base}/api/chat/message-requests/{user_id}"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "action": if accept { "accept" } else { "decline" } }))
        .send()
        .await
        .map_err(|e| format!("Could not reach kryo.to: {e}"))?;
    if !r.status().is_success() {
        return Err(error_of(r).await);
    }
    Ok(())
}

/// Names and supporter status for people in your chats (friends, and members
/// of groups you share). Anyone else is left out by kryo.to.
pub async fn people(base: &str, token: &str, ids: &[u64]) -> Result<serde_json::Value, String> {
    let list = ids.iter().take(64).map(u64::to_string).collect::<Vec<_>>().join(",");
    let r = client(base)
        .get(format!("{base}/api/chat/people"))
        .query(&[("ids", list.as_str())])
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Could not reach kryo.to: {e}"))?;
    if !r.status().is_success() {
        return Err(error_of(r).await);
    }
    r.json().await.map_err(|_| "kryo.to sent an unexpected answer.".to_string())
}

/// A kryo.to API call with the chat sign-in: the public room.
pub async fn api(base: &str, token: &str, method: reqwest::Method, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    let mut req = client(base).request(method, format!("{base}{path}")).bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let r = req.send().await.map_err(|e| format!("Could not reach kryo.to: {e}"))?;
    if !r.status().is_success() {
        return Err(error_of(r).await);
    }
    r.json().await.map_err(|_| "kryo.to sent an unexpected answer.".to_string())
}

/// Send a chat report (messages the person picked, decrypted here) to staff.
pub async fn report(base: &str, token: &str, payload: &serde_json::Value) -> Result<(), String> {
    let r = client(base)
        .post(format!("{base}/api/chat/reports"))
        .bearer_auth(token)
        .json(payload)
        .send()
        .await
        .map_err(|e| format!("Could not reach kryo.to: {e}"))?;
    if !r.status().is_success() {
        return Err(error_of(r).await);
    }
    Ok(())
}

/// Sign this token out on kryo.to (removing chat from this PC).
pub async fn sign_out(base: &str, token: &str) {
    let _ = client(base).post(format!("{base}/api/auth/logout")).bearer_auth(token).send().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_codes_reach_the_page() {
        assert!(approve_script("ABCD-1234").unwrap().contains("code:'ABCD-1234'"));
        for bad in ["", "AB'CD", "AB</script>", "AB CD", "A\\B", &"A".repeat(25)] {
            assert!(approve_script(bad).is_err(), "{bad:?}");
        }
    }
}
