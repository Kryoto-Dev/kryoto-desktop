//! The native client is online even with its Store hidden or in the tray.
//! Website timers must never decide whether the launcher is still running.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

static REPORTED: AtomicBool = AtomicBool::new(false);

pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            beat(&app);
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

pub fn beat<R: Runtime>(app: &AppHandle<R>) {
    let enabled = crate::settings::load(app).share_playtime;
    send(app, enabled);
}

pub fn disconnect<R: Runtime>(app: &AppHandle<R>) {
    send(app, false);
}

fn send<R: Runtime>(app: &AppHandle<R>, connected: bool) {
    let Ok(view) = crate::store(app) else { return };
    let Ok(url) = view.url() else { return };
    if !crate::is_kryoto(&url, app) {
        return;
    }
    if !connected && !REPORTED.swap(false, Ordering::SeqCst) {
        return;
    }
    let playing = app.try_state::<crate::library::Running>().is_some_and(|state| {
        state.0.lock().is_ok_and(|games| games.values().any(|session| session.pid().is_some() && !session.stopped()))
    });
    let body = serde_json::json!({
        "clientId": crate::logging::install_id(app), "connected": connected, "playing": playing
    });
    let origin = serde_json::to_string(&url.origin().ascii_serialization()).unwrap_or_default();
    let script = format!(
        "if(location.origin==={origin})fetch('/api/desktop/presence',{{method:'POST',credentials:'include',keepalive:true,headers:{{'content-type':'application/json'}},body:JSON.stringify({body})}}).catch(()=>{{}})"
    );
    if view.eval(script).is_ok() && connected {
        REPORTED.store(true, Ordering::SeqCst);
    }
}
