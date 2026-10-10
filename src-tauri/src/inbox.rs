//! The phone inbox's desktop side: captures a phone dropped into the hosted
//! service's inbox are pulled here, handed to the main window to land through
//! the same funnel as the quick capture window (`vault.capture`: the editor
//! buffer when the target note is open, `note_insert` otherwise), and acked
//! once the main window reports them written. What a capture says, when it was
//! had and where it goes are worked out here for both doors (below), so the
//! window only writes what it is handed.
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
//! **The ready gate.** A Tauri event reaches only the listeners registered at
//! that moment; one emitted to the main window before its page has run is
//! dropped without an error. The account is warm from the first millisecond,
//! and the service answers a pull at once while an unacked item exists, so
//! without a gate the very first pull of a launch handed the items dropped
//! while the app was closed to a window that was not listening yet, and they
//! stayed in flight for the life of the process. The loop therefore waits until
//! the main window says it is ready — its listener registered and a vault open
//! (`inbox_ready`, sent again on every vault opened later, which also re-offers
//! what could not land) — before pulling for it, and a page load of the main
//! window (`page_reloading`) closes the gate again and returns what was in
//! flight to the pool: the page those items went to is gone. A direct landing
//! (below) needs no window and does not wait for one.
//!
//! **The landing vault.** A phone cannot see which vault is open on the desk,
//! and privacy zones are a vault's: a private thought must not land in the work
//! vault because that one happened to be open. So the vault a phone's captures
//! land in is fixed (`inbox_vault` in settings.json: the vault open when the
//! first pairing code was shown, changed in Settings › Quick capture › Phone),
//! and `route` decides the door per pull. The landing vault open in the main
//! window: the items go through the window as above (its editor buffer may hold
//! the target note). Any other vault open, or none: `land_direct` writes them
//! here, straight into the landing vault — opened as a second `Vault` for the
//! moment, at the same capture spot (`capture_spot`, the mirror of capture.ts's
//! defaultCaptureSpot: inbox.md, today's journal note, or the vault's own pin),
//! with the same line and the same provenance line the window's landing writes.
//! Nothing on this side watches that vault and no buffer can hold its notes, so
//! the write is as safe as any external program's. No landing vault fixed (a
//! pairing from before the field existed): the open vault, through the window.
//!
//! **The ledger.** `.vemry/inbox-seen.json` in the landing vault lists the ids
//! already written into it. It is written *before* the ack goes out: an ack lost
//! to the network leaves the item on the server, the next pull finds its id in
//! the ledger and acks it without landing it again. Because the file lives in
//! the vault, two computers sharing one vault through a sync folder share the
//! ledger too, and the second one skips what the first already landed.
//!
//! Pairing is the desktop's job as well: `inbox_pair_start` asks the service
//! for a single-use code and renders the QR code the phone scans (the capture
//! page's URL with the code in its fragment); the settings page lists the paired
//! phones and revokes one by id. Every landing, through the window or direct,
//! is remembered in the settings (`inbox_last_landed`) so the settings page can
//! say when the last capture from a phone was filed and where (served with the
//! settings).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use vemry_core::{InsertOrigin, Vault};

use crate::commands::CapturePin;
use crate::settings::{CaptureTarget, InboxLanding};
use crate::{account, AppState};

/// To the main window: land these items (a `WindowBatch`; mirrored as `InboxBatch` in api.ts).
const EV_ITEMS: &str = "inbox://items";
/// To the main window: captures written straight into the landing vault, for the
/// receipt (`{ count, vault, target }`; mirrored as `InboxLanded` in api.ts).
const EV_LANDED: &str = "inbox://landed";
/// To the main window: the landing vault could not take the captures
/// (`{ vault, error }`); they are offered again in a few minutes.
const EV_HOME_FAILED: &str = "inbox://home-failed";
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
/// Everything the service returned is waiting for its retry: the next pull waits at most this
/// long. The service answers at once while an unacked item exists, so no pull is held open in
/// the meantime, and this is how late a new drop can be while a failed one waits.
const RETRY_PAUSE_MAX: Duration = Duration::from_secs(15);
const BACKOFF_MIN: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(5 * 60);
const LEDGER_FILE: &str = "inbox-seen.json";
/// The ledger keeps the newest this many ids; the server forgets an acked item after a week anyway.
const LEDGER_CAP: usize = 5000;

/// One item as the service hands it out (`PullItem` in the protocol; the fields
/// this side does not read are left to serde to skip).
#[derive(Clone, Debug, Deserialize)]
pub struct InboxItem {
    pub id: String,
    /// The phone's local time with its offset — the stamp the note gets.
    pub created_at: String,
    pub received_at: String,
    pub text: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

/// What the main window lands (`EV_ITEMS`): every item's finished text and stamp
/// moment, and the one spot they all go to — the same values a direct landing
/// writes, so the two doors cannot file an item differently.
#[derive(Debug, Serialize)]
struct WindowBatch {
    items: Vec<WindowItem>,
    path: String,
    heading: Option<String>,
    /// The spot's label, for the receipt and the ack.
    target: String,
    /// The root of the vault the spot was worked out for (the open one at the moment
    /// of handing over): a window that has switched vaults since lands nothing and
    /// reports the items failed, so they cannot land in a vault that is not theirs.
    root: String,
}

#[derive(Debug, Serialize)]
struct WindowItem {
    id: String,
    text: String,
    /// RFC 3339: the moment the stamp names (`stamp_time`).
    at: String,
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
    /// The landing vault after this code was shown (the open vault became it
    /// when none was fixed yet), so the settings page can show it at once.
    pub inbox_vault: Option<String>,
}

#[derive(Default)]
pub struct InboxState {
    inner: Mutex<Inner>,
    /// Woken by `inbox_ready`; the loop waits on it while the gate is closed.
    ready: tokio::sync::Notify,
}

#[derive(Default)]
struct Inner {
    /// The main window is listening and has a vault open (see the module comment).
    ready: bool,
    /// Handed to the main window, neither acked nor failed yet, with the root of the vault
    /// the batch was worked out for: its ledger is the one the ack writes, whichever vault
    /// the window has open by the time the ack comes back.
    in_flight: HashMap<String, PathBuf>,
    /// Could not land; offered again once the instant has passed.
    retry_at: HashMap<String, Instant>,
}

impl InboxState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        crate::lock(&self.inner)
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

/// The open vault's ledger (the door the main window lands through).
fn ledger_path(app: &AppHandle) -> Option<PathBuf> {
    let state = app.state::<AppState>();
    let guard = crate::lock(&state.vault);
    guard.as_ref().map(|v| v.meta_dir().join(LEDGER_FILE))
}

fn ledger_read(path: &Path) -> Ledger {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Add ids to the ledger at this path (`config_store::write` creates the meta
/// directory on the first write).
fn ledger_add_at(path: &Path, ids: &[String]) -> Result<(), String> {
    let mut ledger = ledger_read(path);
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
    crate::config_store::write(path, &ledger)
}

// ---------------------------------------------------------------------------
// the route: which door the items go through
// ---------------------------------------------------------------------------

/// Where a pull's items go (the module comment, "The landing vault").
#[derive(Clone, Debug, PartialEq, Eq)]
enum Route {
    /// Through the main window's `vault.capture`: the open vault is the landing
    /// vault, or none is fixed.
    Window,
    /// Written here, straight into the landing vault at this root: it is not the
    /// vault the main window has open.
    Direct(PathBuf),
}

fn route(app: &AppHandle) -> Route {
    let home = crate::settings::load(app).inbox_vault.filter(|h| !h.trim().is_empty());
    let Some(home) = home else { return Route::Window };
    let state = app.state::<AppState>();
    let open = crate::lock(&state.vault).as_ref().map(|v| v.root().to_string_lossy().to_string());
    pick_route(&home, open.as_deref())
}

/// The landing vault against the open vault's root, both spelled the way
/// `vault_open` stores them (canonical, verbatim prefix washed).
fn pick_route(home: &str, open: Option<&str>) -> Route {
    if open == Some(home) {
        Route::Window
    } else {
        Route::Direct(PathBuf::from(home))
    }
}

// ---------------------------------------------------------------------------
// the poll loop
// ---------------------------------------------------------------------------

/// Start the background loop (once, at startup). It waits for the main window
/// to be ready, idles while signed out and backs off while the service cannot
/// be reached; a 401 clears the account the way every other call does.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut backoff = BACKOFF_MIN;
        loop {
            let route = wait_routable(&app).await;
            if account::cached(&app).token.is_empty() {
                tokio::time::sleep(IDLE_SIGNED_OUT).await;
                continue;
            }
            match pull(&app, &route).await {
                Ok(Pulled::Fresh(items)) => {
                    backoff = BACKOFF_MIN;
                    land(&app, items).await;
                }
                Ok(Pulled::Nothing) => backoff = BACKOFF_MIN,
                // The service answers at once while an unacked item exists, and every item it
                // returned is with the main window or waiting for its retry: wait for that, do not
                // spin. A ready (a vault opened, which re-offers the failed ones) ends the wait early
                Ok(Pulled::AllInFlight(pause)) => {
                    backoff = BACKOFF_MIN;
                    let state = app.state::<AppState>();
                    tokio::select! {
                        _ = tokio::time::sleep(pause) => {}
                        _ = state.inbox.ready.notified() => {}
                    }
                }
                // The gate closed under the pull (the page is reloading): back to waiting
                Ok(Pulled::NotReady) => backoff = BACKOFF_MIN,
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
    /// New items: not in flight, not waiting for a retry, not in the open
    /// vault's ledger (a direct landing checks the landing vault's ledger itself).
    Fresh(Vec<InboxItem>),
    /// The service had nothing (the hold timed out).
    Nothing,
    /// It had items, every one already with the main window or waiting for its retry:
    /// pull again after this long (the in-flight pause, or until the first retry is due).
    AllInFlight(Duration),
    /// It had items, but the main window is not listening: nothing was handed over.
    NotReady,
}

/// Block until the items have a door: the landing vault is fixed and not the open
/// one (they are written here), or the main window is ready. `notify_one` stores a
/// permit when nobody is waiting, so a readiness (or a landing vault, `wake`)
/// announced between the check and the wait is not lost.
async fn wait_routable(app: &AppHandle) -> Route {
    let state = app.state::<AppState>();
    loop {
        let route = route(app);
        if matches!(route, Route::Direct(_)) || state.inbox.lock().ready {
            return route;
        }
        state.inbox.ready.notified().await;
    }
}

/// Something the route depends on changed (the landing vault in the settings):
/// the loop looks again if it was waiting.
pub fn wake(app: &AppHandle) {
    app.state::<AppState>().inbox.ready.notify_one();
}

/// One long poll. Items already in flight, waiting for their retry, or in the
/// open vault's ledger are not returned: the last kind is acked right here, since
/// the vault already has it. The ledger is read under the same lock `inbox_ack`
/// writes it under, so an ack landing between the read and the in-flight check
/// cannot make an item look new twice.
async fn pull(app: &AppHandle, route: &Route) -> Result<Pulled, String> {
    let req = account::authed(app, reqwest::Method::GET, &format!("/v1/inbox/pull?since=0&wait={WAIT_SECS}"))?;
    let (_, _, body) = account::send_timed(app, req, PULL_TIMEOUT).await?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let all: Vec<InboxItem> = serde_json::from_value(v.get("items").cloned().unwrap_or(Value::Array(vec![]))).map_err(|e| e.to_string())?;
    if all.is_empty() {
        return Ok(Pulled::Nothing);
    }
    let mut seen_already = Vec::new();
    let now = Instant::now();
    let mut in_flight = false;
    let mut first_retry: Option<Instant> = None;
    // The open vault's ledger answers for the window's door only; a direct landing reads
    // the landing vault's ledger once that vault is open. Its path is taken before the inbox
    // lock: the vault lock can be held through a long vault operation, and the page-load
    // callback and the window's commands wait on the inbox lock
    let ledger_at = match route {
        Route::Window => ledger_path(app),
        Route::Direct(_) => None,
    };
    let fresh: Vec<InboxItem> = {
        let state = app.state::<AppState>();
        let mut inner = state.inbox.lock();
        // Checked under the lock the gate is closed under (the page reloaded under the pull)
        if *route == Route::Window && !inner.ready {
            return Ok(Pulled::NotReady);
        }
        let ledger: HashSet<String> = ledger_at.map(|p| ledger_read(&p).ids.into_iter().collect()).unwrap_or_default();
        inner.retry_at.retain(|_, at| *at > now);
        let mut out = Vec::new();
        for it in all {
            if ledger.contains(&it.id) {
                seen_already.push(it.id);
            } else if inner.in_flight.contains_key(&it.id) {
                in_flight = true;
            } else if let Some(&at) = inner.retry_at.get(&it.id) {
                first_retry = Some(first_retry.map_or(at, |f| f.min(at)));
            } else {
                out.push(it);
            }
        }
        out
    };
    if !seen_already.is_empty() {
        // Already in the vault (a lost ack, or the other computer sharing this vault landed it)
        if let Err(e) = ack_remote(app, &seen_already).await {
            eprintln!("inbox ack of already-landed items: {e}");
        }
    }
    if !fresh.is_empty() {
        return Ok(Pulled::Fresh(fresh));
    }
    // Every item was acked just now: nothing is left on the service to answer at once
    Ok(match (in_flight, first_retry) {
        (false, None) => Pulled::Nothing,
        (false, Some(at)) => Pulled::AllInFlight(at.saturating_duration_since(now).min(RETRY_PAUSE_MAX)),
        (true, _) => Pulled::AllInFlight(IN_FLIGHT_PAUSE),
    })
}

/// Land what a pull returned. The route is taken again here rather than kept
/// from the pull: a vault opened or closed during the hold moves the items to
/// the other door, and only the door that is right now writes.
async fn land(app: &AppHandle, items: Vec<InboxItem>) {
    match route(app) {
        Route::Window => deliver(app, items),
        Route::Direct(root) => land_direct(app, root, items).await,
    }
}

/// Hand the items to the main window. They enter the in-flight set under the
/// lock the gate is closed under, and only while the window is listening: with
/// the gate closed nothing is handed over, and the next pull returns them.
fn deliver(app: &AppHandle, items: Vec<InboxItem>) {
    let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
    let (pin, root) = {
        let state = app.state::<AppState>();
        let guard = crate::lock(&state.vault);
        guard.as_ref().map(|v| (crate::commands::capture_pin(v), v.root().to_string_lossy().to_string())).unwrap_or_default()
    };
    let spot = capture_spot(crate::settings::load(app).capture_target, &pin);
    let batch = WindowBatch {
        items: items.iter().map(|it| WindowItem { id: it.id.clone(), text: inbox_text(it), at: stamp_time(it).to_rfc3339() }).collect(),
        target: spot.label(),
        path: spot.path,
        heading: spot.heading,
        root,
    };
    {
        let state = app.state::<AppState>();
        let mut inner = state.inbox.lock();
        if !inner.ready {
            return;
        }
        inner.in_flight.extend(ids.iter().map(|id| (id.clone(), PathBuf::from(&batch.root))));
    }
    if crate::browser::window::relay_to_main(app, EV_ITEMS, json!(batch), false).is_err() {
        // No main window to land in (the app is on its way out): back to the pool
        let state = app.state::<AppState>();
        let mut inner = state.inbox.lock();
        for id in ids {
            inner.in_flight.remove(&id);
        }
    }
}

/// Write the items straight into the landing vault (the module comment, "The
/// landing vault"): open it, skip what its ledger lists, land the rest at the
/// capture spot, write the ledger, ack. The vault itself unreachable, or a
/// capture that cannot be written: offered again in a few minutes, and the main
/// window is told so the user learns the landing vault needs looking at.
async fn land_direct(app: &AppHandle, root: PathBuf, items: Vec<InboxItem>) {
    let mode = crate::settings::load(app).capture_target;
    let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
    let at_root = root.clone();
    // A full vault open and a few file writes: off the async runtime's threads
    let outcome = tauri::async_runtime::spawn_blocking(move || land_into(&at_root, mode, &items))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    let vault_name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| root.to_string_lossy().to_string());
    let landing = match outcome {
        Ok(l) => l,
        Err(e) => {
            eprintln!("inbox: landing vault {}: {e}", root.display());
            retry_later(app, &ids);
            home_failed(app, &vault_name, &e);
            return;
        }
    };
    let failed: Vec<String> = landing.failed.iter().map(|(id, _)| id.clone()).collect();
    retry_later(app, &failed);
    let ack: Vec<String> = landing.landed.iter().chain(&landing.already).cloned().collect();
    if !ack.is_empty() {
        if let Err(e) = ack_remote(app, &ack).await {
            eprintln!("inbox ack after a direct landing: {e}");
        }
    }
    if !landing.landed.is_empty() {
        remember_landing(app, &landing.target, Some(vault_name.clone()));
        let _ = crate::browser::window::relay_to_main(
            app,
            EV_LANDED,
            json!({ "count": landing.landed.len(), "vault": vault_name, "target": landing.target }),
            false,
        );
    }
    if let Some((_, e)) = landing.failed.first() {
        home_failed(app, &vault_name, e);
    }
}

fn home_failed(app: &AppHandle, vault: &str, error: &str) {
    let _ = crate::browser::window::relay_to_main(app, EV_HOME_FAILED, json!({ "vault": vault, "error": error }), false);
}

/// The settings page's "last one filed": written after every landing, through
/// the window (the ack names the spot) or direct.
fn remember_landing(app: &AppHandle, target: &str, vault: Option<String>) {
    let landing = InboxLanding { at: Local::now().to_rfc3339(), target: target.to_string(), vault };
    if let Err(e) = crate::settings::update(app, |s| {
        s.inbox_last_landed = Some(landing);
        true
    }) {
        eprintln!("inbox: remembering the landing: {e}");
    }
}

fn retry_later(app: &AppHandle, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    let mut inner = state.inbox.lock();
    let at = Instant::now() + RETRY_FAILED;
    for id in ids {
        inner.retry_at.insert(id.clone(), at);
    }
}

/// What a direct landing did, by item id.
#[derive(Debug, Default)]
struct Landing {
    landed: Vec<String>,
    /// In the vault's ledger already: acked, not written again.
    already: Vec<String>,
    /// Could not be written, with the reason (the user's language).
    failed: Vec<(String, String)>,
    /// The spot's label for the receipt (`note.md › Section`, as capture.ts spells it).
    target: String,
}

/// The synchronous heart of a direct landing, on a vault opened for the purpose.
/// The ledger is written before the caller acks (the module comment).
fn land_into(root: &Path, mode: CaptureTarget, items: &[InboxItem]) -> Result<Landing, String> {
    // The ledger is a plain file (`Vault::meta_dir`): read before the vault is opened, so
    // items another computer already landed cost no full open of the vault
    let ledger_path = root.join(".vemry").join(LEDGER_FILE);
    let known: HashSet<String> = ledger_read(&ledger_path).ids.into_iter().collect();
    if items.iter().all(|it| known.contains(&it.id)) {
        return Ok(Landing { already: items.iter().map(|it| it.id.clone()).collect(), ..Landing::default() });
    }
    let mut vault = Vault::open(root).map_err(crate::i18n::err)?;
    let spot = capture_spot(mode, &crate::commands::capture_pin(&vault));
    let mut landing = Landing { target: spot.label(), ..Landing::default() };
    for it in items {
        if known.contains(&it.id) {
            landing.already.push(it.id.clone());
            continue;
        }
        match land_one(&mut vault, &spot, it) {
            Ok(()) => landing.landed.push(it.id.clone()),
            Err(e) => landing.failed.push((it.id.clone(), e)),
        }
    }
    if !landing.landed.is_empty() {
        // The notes are written: a ledger that cannot be written does not un-write them,
        // and the caller acks them all the same (the ledger only guards a lost ack) —
        // failing the batch here would land it again at every retry
        if let Err(e) = ledger_add_at(&ledger_path, &landing.landed) {
            eprintln!("inbox: the ledger of {}: {e}", root.display());
        }
    }
    Ok(landing)
}

/// One capture into the vault: the same line the main window's `vault.capture`
/// builds, inserted the way `note_insert` does, with the provenance line
/// `provenance_record` writes (origin capture; a private note records nothing).
fn land_one(vault: &mut Vault, spot: &Spot, it: &InboxItem) -> Result<(), String> {
    let id = vault.note_id(&spot.path).map_err(crate::i18n::err)?;
    let line = capture_line(&inbox_text(it), stamp_time(it));
    vault.insert_note(&id, &line, spot.heading.as_deref()).map_err(crate::i18n::err)?;
    crate::commands::record_insert(vault, &id, &line, InsertOrigin::Capture, None, None);
    Ok(())
}

// ---------------------------------------------------------------------------
// the capture spot and the capture line: mirrors of the frontend's
// ---------------------------------------------------------------------------

/// A note and, optionally, the heading of the section a capture lands in
/// (capture.ts CaptureSpot).
#[derive(Debug, PartialEq, Eq)]
struct Spot {
    path: String,
    heading: Option<String>,
}

impl Spot {
    fn inbox() -> Spot {
        Spot { path: "inbox.md".into(), heading: None }
    }

    /// `note.md › Section` (capture.ts spotLabel).
    fn label(&self) -> String {
        match &self.heading {
            Some(h) => format!("{} › {h}", self.path),
            None => self.path.clone(),
        }
    }
}

/// Where a capture lands, from the capture target and the vault's pin — the
/// mirror of capture.ts's `defaultCaptureSpot`, for a vault the main window does
/// not have open: inbox.md, today's journal note, or the pinned note and section
/// (a pin with no note set falls back to inbox.md rather than failing).
fn capture_spot(mode: CaptureTarget, pin: &CapturePin) -> Spot {
    match mode {
        CaptureTarget::Inbox => Spot::inbox(),
        CaptureTarget::Journal => Spot { path: journal_today(), heading: None },
        CaptureTarget::Pinned => {
            let p = pin.path.trim();
            if p.is_empty() {
                return Spot::inbox();
            }
            let path = if p.to_ascii_lowercase().ends_with(".md") { p.to_string() } else { format!("{p}.md") };
            let heading = pin.heading.trim();
            Spot { path, heading: (!heading.is_empty()).then(|| heading.to_string()) }
        }
    }
}

/// Today's journal note, `journal/YYYY-MM-DD.md` by this machine's clock
/// (tasks.ts journalToday — the two must agree).
fn journal_today() -> String {
    format!("journal/{}.md", Local::now().format("%Y-%m-%d"))
}

/// The capture as one list item, the mirror of `vault.capture` in
/// vault.svelte.ts: `- ` before the first line, two spaces before every other,
/// and the stamp `*(YYYY-MM-DD HH:MM)*` on the last line (a space before it
/// unless that line is empty). The stamp's shape is what tasks.ts's STAMP_RE
/// strips, so the three must not drift.
fn capture_line(text: &str, at: DateTime<Local>) -> String {
    let stamp = format!("*({})*", at.format("%Y-%m-%d %H:%M"));
    let lines: Vec<&str> = text.split('\n').map(str::trim_end).collect();
    let last = lines.len() - 1;
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let lead = if i == 0 { "- " } else { "  " };
            if i == last {
                format!("{lead}{l}{}{stamp}", if l.is_empty() { "" } else { " " })
            } else {
                format!("{lead}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The item's text with its link: a share carries a URL (and a title), which
/// becomes the last line.
fn inbox_text(it: &InboxItem) -> String {
    let Some(url) = it.url.as_deref().filter(|u| !u.is_empty()) else { return it.text.clone() };
    let link = match it.title.as_deref().filter(|t| !t.is_empty()) {
        Some(title) => format!("[{title}]({url})"),
        None => url.to_string(),
    };
    if it.text.trim().is_empty() {
        link
    } else {
        format!("{}\n{link}", it.text)
    }
}

/// The stamp's moment: the phone's clock (the moment the thought was had),
/// unless it is a day or more away from the service's (a phone with the wrong
/// date), then the service's; neither parsing, now. In this machine's zone.
fn stamp_time(it: &InboxItem) -> DateTime<Local> {
    let parse = |s: &str| DateTime::parse_from_rfc3339(s.trim()).ok().map(|d| d.with_timezone(&Local));
    match (parse(&it.created_at), parse(&it.received_at)) {
        (Some(phone), Some(service)) if (phone - service).num_seconds().abs() > 86_400 => service,
        (Some(phone), _) => phone,
        (None, Some(service)) => service,
        (None, None) => Local::now(),
    }
}

async fn ack_remote(app: &AppHandle, ids: &[String]) -> Result<(), String> {
    let req = account::authed(app, reqwest::Method::POST, "/v1/inbox/ack")?.json(&json!({ "ids": ids }));
    account::send(app, req).await.map(|_| ())
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

/// The main window wrote these items into the vault: remember them, then tell
/// the service. The ledger comes first (see the module comment). `target` is the
/// spot's label (capture.ts spotLabel), for the settings page's "last one filed".
#[tauri::command]
pub async fn inbox_ack(app: AppHandle, ids: Vec<String>, target: Option<String>) -> Result<(), String> {
    // The open vault's path, for an id no longer in flight (the page reloaded under it); taken
    // before the inbox lock, as in `pull` (the vault lock is never taken under it)
    let open_ledger = ledger_path(&app);
    let ledger = {
        // Under the lock the pull loop reads the ledger with: the id leaves the in-flight
        // set and enters the ledger in one step, so no pull sees it in neither. The items
        // are written whether or not the ledger could be: they leave the in-flight set and
        // are acked all the same (the ledger only guards a lost ack), rather than staying
        // in flight for the life of the page and landing again after it. Each id goes into
        // the ledger of the vault it was handed over for: the window may have switched vaults
        // since, and a ledger in the wrong vault guards nothing. No vault at all: nothing to
        // remember, and the window had nothing to land in either
        let state = app.state::<AppState>();
        let mut inner = state.inbox.lock();
        let mut by_ledger: HashMap<PathBuf, Vec<String>> = HashMap::new();
        for id in &ids {
            let handed = inner.in_flight.remove(id).filter(|root| !root.as_os_str().is_empty());
            let at = handed.map(|root| root.join(".vemry").join(LEDGER_FILE)).or_else(|| open_ledger.clone());
            if let Some(at) = at {
                by_ledger.entry(at).or_default().push(id.clone());
            }
        }
        by_ledger.iter().map(|(at, ids)| ledger_add_at(at, ids)).fold(Ok(()), Result::and)
    };
    if let Some(target) = target.filter(|_| !ids.is_empty()) {
        remember_landing(&app, &target, None);
    }
    if !account::cached(&app).token.is_empty() {
        ack_remote(&app, &ids).await?;
    }
    ledger
}

/// The main window is listening and has a vault open: items may be handed over
/// from now on, and what could not land earlier is offered again at once (sent
/// on every vault opened, so a capture that found no vault lands when one is).
#[tauri::command]
pub fn inbox_ready(app: AppHandle) {
    let state = app.state::<AppState>();
    {
        let mut inner = state.inbox.lock();
        inner.ready = true;
        inner.retry_at.clear();
    }
    state.inbox.ready.notify_one();
}

/// The main window's page is starting to load (the first time, or anew): its
/// listeners are gone, so the gate closes until it reports ready again, and what
/// was handed to the page that went away returns to the pool. Landed there but
/// not yet acked, an item lands twice — the ledger is written at the ack — which
/// is the failure the design prefers to losing it.
pub fn page_reloading(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut inner = state.inbox.lock();
    inner.ready = false;
    inner.in_flight.clear();
}

/// The main window could not land these (no vault open, a note that cannot be
/// written): they are offered again in a few minutes.
#[tauri::command]
pub fn inbox_failed(app: AppHandle, ids: Vec<String>) {
    {
        let state = app.state::<AppState>();
        let mut inner = state.inbox.lock();
        for id in &ids {
            inner.in_flight.remove(id);
        }
    }
    retry_later(&app, &ids);
}

/// A fresh pairing code, rendered as the QR code the phone scans.
#[tauri::command]
pub async fn inbox_pair_start(app: AppHandle) -> Result<PairStart, String> {
    let req = account::authed(&app, reqwest::Method::POST, "/v1/inbox/pair/start")?.json(&json!({}));
    let (_, _, text) = account::send(&app, req).await?;
    let v: Value = serde_json::from_str(&text).map_err(|e| bad_answer(&e.to_string()))?;
    let url = v.get("url").and_then(Value::as_str).ok_or_else(|| bad_answer("no url in the answer"))?.to_string();
    let expires_at = v.get("expires_at").and_then(Value::as_str).unwrap_or_default().to_string();
    let svg = qr_svg(&url)?;
    // The landing vault defaults to the vault open when the code is shown (the module
    // comment, "The landing vault"); once fixed it is the settings page's to change
    let state = app.state::<AppState>();
    let open = crate::lock(&state.vault).as_ref().map(|v| v.root().to_string_lossy().to_string());
    let mut inbox_vault = None;
    crate::settings::update(&app, |s| {
        let fix = s.inbox_vault.is_none() && open.is_some();
        if fix {
            s.inbox_vault = open;
        }
        inbox_vault = s.inbox_vault.clone();
        fix
    })?;
    Ok(PairStart { url, expires_at, svg, inbox_vault })
}

/// The paired phones.
#[tauri::command]
pub async fn inbox_doors(app: AppHandle) -> Result<Vec<InboxDoor>, String> {
    let req = account::authed(&app, reqwest::Method::GET, "/v1/inbox/doors")?;
    let (_, _, text) = account::send(&app, req).await?;
    let v: Value = serde_json::from_str(&text).map_err(|e| bad_answer(&e.to_string()))?;
    serde_json::from_value(v.get("doors").cloned().unwrap_or(Value::Array(vec![]))).map_err(|e| bad_answer(&e.to_string()))
}

/// An answer from the service this side cannot read, in the user's words (the settings page shows it).
fn bad_answer(detail: &str) -> String {
    crate::i18n::tf(crate::i18n::current(), "ipc.hosted.failed", &[("error", detail)])
}

/// This phone's token stops working; what it dropped stays.
#[tauri::command]
pub async fn inbox_door_revoke(app: AppHandle, id: i64) -> Result<(), String> {
    let req = account::authed(&app, reqwest::Method::DELETE, &format!("/v1/inbox/doors/{id}"))?;
    account::send(&app, req).await.map(|_| ())
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

    fn at(h: u32, m: u32) -> DateTime<Local> {
        use chrono::TimeZone;
        Local.with_ymd_and_hms(2026, 9, 22, h, m, 0).unwrap()
    }

    fn item(id: &str, text: &str) -> InboxItem {
        InboxItem {
            id: id.into(),
            created_at: "2026-09-22T10:05:00+08:00".into(),
            received_at: "2026-09-22T02:05:30Z".into(),
            text: text.into(),
            url: None,
            title: None,
        }
    }

    /// The line the main window builds (vault.capture), byte for byte.
    #[test]
    fn capture_line_mirrors_the_windows_capture() {
        let t = at(10, 5);
        assert_eq!(capture_line("a thought", t), "- a thought *(2026-09-22 10:05)*");
        assert_eq!(capture_line("first\nsecond  ", t), "- first\n  second *(2026-09-22 10:05)*");
        assert_eq!(capture_line("", t), "- *(2026-09-22 10:05)*");
        assert_eq!(capture_line("line\n", t), "- line\n  *(2026-09-22 10:05)*");
    }

    #[test]
    fn inbox_text_appends_the_link_the_way_the_window_does() {
        let mut it = item("x", "see this");
        assert_eq!(inbox_text(&it), "see this");
        it.url = Some("https://example.org/p".into());
        assert_eq!(inbox_text(&it), "see this\nhttps://example.org/p");
        it.title = Some("A page".into());
        assert_eq!(inbox_text(&it), "see this\n[A page](https://example.org/p)");
        it.text = "  ".into();
        assert_eq!(inbox_text(&it), "[A page](https://example.org/p)");
    }

    /// The phone's clock stamps the note; a phone a day or more off yields to the service's.
    #[test]
    fn stamp_time_prefers_the_phone_unless_a_day_off() {
        let mut it = item("x", "t");
        let phone = DateTime::parse_from_rfc3339("2026-09-22T10:05:00+08:00").unwrap();
        assert_eq!(stamp_time(&it), phone.with_timezone(&Local));
        it.created_at = "2026-09-20T10:05:00+08:00".into();
        let service = DateTime::parse_from_rfc3339("2026-09-22T02:05:30Z").unwrap();
        assert_eq!(stamp_time(&it), service.with_timezone(&Local));
        it.created_at = "yesterday".into();
        assert_eq!(stamp_time(&it), service.with_timezone(&Local));
    }

    #[test]
    fn capture_spot_follows_the_target_and_falls_back_to_inbox() {
        let none = CapturePin::default();
        assert_eq!(capture_spot(CaptureTarget::Inbox, &none), Spot::inbox());
        assert_eq!(capture_spot(CaptureTarget::Pinned, &none), Spot::inbox());
        let pinned = CapturePin { path: "notes/ideas".into(), heading: " Later ".into() };
        let spot = capture_spot(CaptureTarget::Pinned, &pinned);
        assert_eq!(spot, Spot { path: "notes/ideas.md".into(), heading: Some("Later".into()) });
        assert_eq!(spot.label(), "notes/ideas.md › Later");
        assert!(capture_spot(CaptureTarget::Journal, &none).path.starts_with("journal/"));
    }

    #[test]
    fn the_route_is_the_window_only_for_the_landing_vault_itself() {
        assert_eq!(pick_route("/v/home", Some("/v/home")), Route::Window);
        assert_eq!(pick_route("/v/home", Some("/v/work")), Route::Direct(PathBuf::from("/v/home")));
        assert_eq!(pick_route("/v/home", None), Route::Direct(PathBuf::from("/v/home")));
    }

    /// A direct landing writes the capture line, the ledger and the provenance
    /// line into a vault the main window does not have open, and a second
    /// landing of the same items writes nothing again.
    #[test]
    fn land_into_writes_once_and_remembers() {
        let dir = std::env::temp_dir().join(format!("vemry-inbox-land-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".vemry")).unwrap();
        std::fs::write(dir.join("inbox.md"), "# Inbox\n\n- older *(2026-09-01 09:00)*\n").unwrap();
        std::fs::write(dir.join(".vemry/config.json"), r#"{"capture_pin":{"path":"notes/ideas","heading":"Later"}}"#).unwrap();
        let mut with_link = item("b", "");
        with_link.url = Some("https://example.org".into());
        with_link.title = Some("Example".into());
        let items = vec![item("a", "a thought"), with_link];

        let l = land_into(&dir, CaptureTarget::Inbox, &items).unwrap();
        assert_eq!((l.landed.len(), l.already.len(), l.failed.len()), (2, 0, 0));
        assert_eq!(l.target, "inbox.md");
        let inbox = std::fs::read_to_string(dir.join("inbox.md")).unwrap();
        // The stamp is the phone's moment on this machine's clock, so the expected
        // text is worked out in the local zone rather than written down
        let at = stamp_time(&items[0]).format("%Y-%m-%d %H:%M");
        assert!(inbox.contains(&format!("- a thought *({at})*")), "{inbox}");
        assert!(inbox.contains(&format!("- [Example](https://example.org) *({at})*")), "{inbox}");
        let ledger = ledger_read(&dir.join(".vemry").join(LEDGER_FILE));
        assert_eq!(ledger.ids, vec!["a".to_string(), "b".to_string()]);
        let prov = std::fs::read_to_string(dir.join(".vemry/provenance.jsonl")).unwrap();
        assert!(prov.contains("\"origin\":\"capture\""), "{prov}");

        let again = land_into(&dir, CaptureTarget::Inbox, &items).unwrap();
        assert_eq!((again.landed.len(), again.already.len()), (0, 2));
        assert_eq!(std::fs::read_to_string(dir.join("inbox.md")).unwrap(), inbox);

        // The pinned mode reads the vault's own pin, and a missing note is created
        let l = land_into(&dir, CaptureTarget::Pinned, &[item("c", "pinned")]).unwrap();
        assert_eq!(l.target, "notes/ideas.md › Later");
        let ideas = std::fs::read_to_string(dir.join("notes/ideas.md")).unwrap();
        assert!(ideas.contains("- pinned *("), "{ideas}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ledger_keeps_the_newest_ids() {
        let dir = std::env::temp_dir().join(format!("vemry-inbox-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(LEDGER_FILE);
        ledger_add_at(&path, &(0..LEDGER_CAP + 10).map(|i| i.to_string()).collect::<Vec<_>>()).unwrap();
        let back = ledger_read(&path);
        assert_eq!(back.ids.len(), LEDGER_CAP);
        assert_eq!(back.ids.first().map(String::as_str), Some("10"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
