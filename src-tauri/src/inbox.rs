//! The phone inbox's desktop side: captures a phone dropped into the hosted
//! service's inbox are pulled here, handed to the main window to land through
//! the same funnel as the quick capture window (`vault.capture`: the editor
//! buffer when the target note is open, `note_insert` otherwise), and acked
//! once the main window reports them written.
//!
//! One background loop long-polls `GET /v1/inbox/pull` with the device token
//! for as long as the app runs and the account is signed in; a drop on the
//! phone answers the poll within a second, and between drops the connection
//! is simply held open. The loop hands each item to the main window exactly
//! once at a time: an item is *in flight* until the frontend acks it or
//! reports it could not land (no vault open, say), and a failed one is
//! offered again five minutes later. The server keeps an item until it is
//! acked, so a crash between landing and acking would land it twice — the
//! ledger below makes that a no-op.
//!
//! **The ledger.** `.vemry/inbox-seen.json` in the vault lists the ids already
//! written into it. It is written *before* the ack goes out: an ack lost to the
//! network leaves the item on the server, the next pull finds its id in the
//! ledger and acks it without landing it again. Because the file lives in the
//! vault, two computers sharing one vault through a sync folder share the ledger
//! too, and the second one skips what the first already landed.
//!
//! Pairing is the desktop's job as well: `inbox_pair_start` asks the service
//! for a single-use code and renders the QR code the phone scans (the capture
//! page's URL with the code in its fragment); the settings page lists the paired
//! phones and revokes one by id. Rust here never writes into the vault except
//! the ledger; the notes are the main window's.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::{account, AppState};

/// To the main window: land these items (the payload is `{ items: InboxItem[] }`).
const EV_ITEMS: &str = "inbox://items";
/// How long the service holds a pull open (its own cap is 25 s).
const WAIT_SECS: u64 = 25;
/// The pull's own timeout: the hold plus the network's share.
const PULL_TIMEOUT: Duration = Duration::from_secs(WAIT_SECS + 15);
/// Signed out: look again this often (a sign-in is noticed within it).
const IDLE_SIGNED_OUT: Duration = Duration::from_secs(10);
/// The main window could not land an item: offer it again after this.
const RETRY_FAILED: Duration = Duration::from_secs(5 * 60);
/// Everything the service returned is in flight to the main window: the next pull waits this long.
const IN_FLIGHT_PAUSE: Duration = Duration::from_secs(3);
const BACKOFF_MIN: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(5 * 60);
const LEDGER_FILE: &str = "inbox-seen.json";
/// The ledger keeps the newest this many ids; the server forgets an acked item after a week anyway.
const LEDGER_CAP: usize = 5000;

/// One item as the service hands it out (`PullItem` in the protocol); mirrored as
/// `InboxItem` in api.ts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboxItem {
    pub seq: i64,
    pub id: String,
    /// The phone's local time with its offset — the stamp the note gets.
    pub created_at: String,
    pub received_at: String,
    pub text: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub door_kind: String,
    #[serde(default)]
    pub door_name: String,
}

/// A paired phone, for the settings page (mirrored as `InboxDoor` in api.ts).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboxDoor {
    pub id: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub last_seen_at: Option<String>,
}

/// What the settings page shows for pairing: the QR code (an SVG using
/// `currentColor`, so it follows the theme) and when the code stops working.
#[derive(Clone, Debug, Serialize)]
pub struct PairStart {
    pub url: String,
    pub expires_at: String,
    pub svg: String,
}

#[derive(Default)]
pub struct InboxState(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    /// Handed to the main window, neither acked nor failed yet.
    in_flight: HashSet<String>,
    /// Could not land; offered again once the instant has passed.
    retry_at: HashMap<String, Instant>,
}

impl InboxState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        crate::lock(&self.0)
    }
}

// ---------------------------------------------------------------------------
// the ledger
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    #[serde(default)]
    ids: Vec<String>,
}

fn ledger_path(app: &AppHandle) -> Option<PathBuf> {
    let state = app.state::<AppState>();
    let guard = crate::lock(&state.vault);
    guard.as_ref().map(|v| v.meta_dir().join(LEDGER_FILE))
}

fn ledger_read(path: &PathBuf) -> Ledger {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Add ids to the vault's ledger (the file is created with the meta directory
/// on the first write). No vault open: nothing to remember, and the caller has
/// nothing to land either.
fn ledger_add(app: &AppHandle, ids: &[String]) -> Result<(), String> {
    let Some(path) = ledger_path(app) else { return Ok(()) };
    let mut ledger = ledger_read(&path);
    let known: HashSet<&String> = ledger.ids.iter().collect();
    let fresh: Vec<String> = ids.iter().filter(|i| !known.contains(i)).cloned().collect();
    if fresh.is_empty() {
        return Ok(());
    }
    ledger.ids.extend(fresh);
    if ledger.ids.len() > LEDGER_CAP {
        let drop = ledger.ids.len() - LEDGER_CAP;
        ledger.ids.drain(..drop);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::config_store::write(&path, &ledger)
}

// ---------------------------------------------------------------------------
// the poll loop
// ---------------------------------------------------------------------------

/// Start the background loop (once, at startup). It idles while signed out and
/// backs off while the service cannot be reached; a 401 clears the account the
/// way every other call does.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut backoff = BACKOFF_MIN;
        loop {
            let token = account::cached(&app).token;
            if token.is_empty() {
                tokio::time::sleep(IDLE_SIGNED_OUT).await;
                continue;
            }
            match pull(&app, &token).await {
                Ok(Pulled::Fresh(items)) => {
                    backoff = BACKOFF_MIN;
                    deliver(&app, items);
                }
                Ok(Pulled::Nothing) => backoff = BACKOFF_MIN,
                // The service answers at once while an unacked item exists, and every item it
                // returned is with the main window already: wait for that, do not spin
                Ok(Pulled::AllInFlight) => {
                    backoff = BACKOFF_MIN;
                    tokio::time::sleep(IN_FLIGHT_PAUSE).await;
                }
                Err(e) => {
                    eprintln!("inbox pull: {e}");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(BACKOFF_MAX);
                }
            }
        }
    });
}

enum Pulled {
    /// New items, marked in flight.
    Fresh(Vec<InboxItem>),
    /// The service had nothing (the hold timed out).
    Nothing,
    /// It had items, every one already with the main window or waiting for its retry.
    AllInFlight,
}

/// One long poll. Items already in flight, waiting for their retry, or in the
/// ledger are not returned: the last kind is acked right here, since the vault
/// already has it. The ledger is read under the same lock `inbox_ack` writes it
/// under, so an ack landing between the read and the in-flight check cannot make
/// an item look new twice.
async fn pull(app: &AppHandle, token: &str) -> Result<Pulled, String> {
    let url = format!("{}/v1/inbox/pull?since=0&wait={WAIT_SECS}", account::base());
    let resp = account::client()
        .get(url)
        .bearer_auth(token)
        .header(reqwest::header::USER_AGENT, vemry_agent::client::user_agent())
        .timeout(PULL_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        account::on_unauthorized(app);
        return Err("unauthorized".into());
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", body.chars().take(200).collect::<String>()));
    }
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let all: Vec<InboxItem> = serde_json::from_value(v.get("items").cloned().unwrap_or(Value::Array(vec![]))).map_err(|e| e.to_string())?;
    if all.is_empty() {
        return Ok(Pulled::Nothing);
    }
    let mut seen_already = Vec::new();
    let now = Instant::now();
    let fresh: Vec<InboxItem> = {
        let state = app.state::<AppState>();
        let mut inner = state.inbox.lock();
        let ledger: HashSet<String> = ledger_path(app).map(|p| ledger_read(&p).ids.into_iter().collect()).unwrap_or_default();
        inner.retry_at.retain(|_, at| *at > now);
        let mut out = Vec::new();
        for it in all {
            if ledger.contains(&it.id) {
                seen_already.push(it.id);
            } else if !inner.in_flight.contains(&it.id) && !inner.retry_at.contains_key(&it.id) {
                inner.in_flight.insert(it.id.clone());
                out.push(it);
            }
        }
        out
    };
    if !seen_already.is_empty() {
        // Already in the vault (a lost ack, or the other computer sharing this vault landed it)
        if let Err(e) = ack_remote(app, token, &seen_already).await {
            eprintln!("inbox ack of already-landed items: {e}");
        }
        if fresh.is_empty() {
            return Ok(Pulled::Nothing);
        }
    }
    Ok(if fresh.is_empty() { Pulled::AllInFlight } else { Pulled::Fresh(fresh) })
}

fn deliver(app: &AppHandle, items: Vec<InboxItem>) {
    let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
    if crate::browser::window::relay_to_main(app, EV_ITEMS, json!({ "items": items }), false).is_err() {
        // No main window to land in (the app is on its way out): back to the pool
        let state = app.state::<AppState>();
        let mut inner = state.inbox.lock();
        for id in ids {
            inner.in_flight.remove(&id);
        }
    }
}

async fn ack_remote(app: &AppHandle, token: &str, ids: &[String]) -> Result<(), String> {
    let req = account::client().post(format!("{}/v1/inbox/ack", account::base())).bearer_auth(token).json(&json!({ "ids": ids }));
    account::send(app, req).await.map(|_| ())
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

/// The main window wrote these items into the vault: remember them, then tell
/// the service. The ledger comes first (see the module comment).
#[tauri::command]
pub async fn inbox_ack(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    {
        // Under the lock the pull loop reads the ledger with: the id leaves the in-flight
        // set and enters the ledger in one step, so no pull sees it in neither
        let state = app.state::<AppState>();
        let mut inner = state.inbox.lock();
        ledger_add(&app, &ids)?;
        for id in &ids {
            inner.in_flight.remove(id);
        }
    }
    let token = account::cached(&app).token;
    if token.is_empty() {
        return Ok(());
    }
    ack_remote(&app, &token, &ids).await
}

/// The main window could not land these (no vault open, a note that cannot be
/// written): they are offered again in a few minutes.
#[tauri::command]
pub fn inbox_failed(app: AppHandle, ids: Vec<String>) {
    let state = app.state::<AppState>();
    let mut inner = state.inbox.lock();
    let at = Instant::now() + RETRY_FAILED;
    for id in ids {
        inner.in_flight.remove(&id);
        inner.retry_at.insert(id, at);
    }
}

/// A fresh pairing code, rendered as the QR code the phone scans.
#[tauri::command]
pub async fn inbox_pair_start(app: AppHandle) -> Result<PairStart, String> {
    let token = signed_in(&app)?;
    let req = account::client().post(format!("{}/v1/inbox/pair/start", account::base())).bearer_auth(&token).json(&json!({}));
    let (_, _, text) = account::send(&app, req).await?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let url = v.get("url").and_then(Value::as_str).ok_or("no url in the answer")?.to_string();
    let expires_at = v.get("expires_at").and_then(Value::as_str).unwrap_or_default().to_string();
    let svg = qr_svg(&url)?;
    Ok(PairStart { url, expires_at, svg })
}

/// The paired phones.
#[tauri::command]
pub async fn inbox_doors(app: AppHandle) -> Result<Vec<InboxDoor>, String> {
    let token = signed_in(&app)?;
    let req = account::client().get(format!("{}/v1/inbox/doors", account::base())).bearer_auth(&token);
    let (_, _, text) = account::send(&app, req).await?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    serde_json::from_value(v.get("doors").cloned().unwrap_or(Value::Array(vec![]))).map_err(|e| e.to_string())
}

/// This phone's token stops working; what it dropped stays.
#[tauri::command]
pub async fn inbox_door_revoke(app: AppHandle, id: i64) -> Result<(), String> {
    let token = signed_in(&app)?;
    let req = account::client().delete(format!("{}/v1/inbox/doors/{id}", account::base())).bearer_auth(&token);
    account::send(&app, req).await.map(|_| ())
}

fn signed_in(app: &AppHandle) -> Result<String, String> {
    let token = account::cached(app).token;
    if token.is_empty() {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.hosted.auth_invalid"));
    }
    Ok(token)
}

/// The QR code as an SVG drawn in `currentColor` on nothing, so the settings
/// page's theme colours it.
fn qr_svg(data: &str) -> Result<String, String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(data.as_bytes()).map_err(|e| e.to_string())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .dark_color(svg::Color("currentColor"))
        .light_color(svg::Color("transparent"))
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_renders_the_url_in_current_color() {
        let svg = qr_svg("https://api.vemry.app/c/#pair=abc").unwrap();
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"));
        assert!(svg.contains("currentColor"));
    }

    #[test]
    fn ledger_keeps_the_newest_ids() {
        let dir = std::env::temp_dir().join(format!("vemry-inbox-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(LEDGER_FILE);
        let mut ledger = Ledger::default();
        ledger.ids = (0..LEDGER_CAP + 10).map(|i| i.to_string()).collect();
        let drop = ledger.ids.len() - LEDGER_CAP;
        ledger.ids.drain(..drop);
        std::fs::create_dir_all(&dir).unwrap();
        crate::config_store::write(&path, &ledger).unwrap();
        let back = ledger_read(&path);
        assert_eq!(back.ids.len(), LEDGER_CAP);
        assert_eq!(back.ids.first().map(String::as_str), Some("10"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
