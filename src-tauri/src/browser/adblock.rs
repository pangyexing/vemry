//! The ad-blocking pipeline (macOS first): subscribed filter lists → adblock-rust
//! compiles them into WKContentRuleList JSON → WebKit blocks natively. No JS
//! injection, no local proxy.
//!
//! The sequence: startup (or a settings change) → a background thread does the pure
//! Rust part (parsing rules into JSON, CPU-heavy) → on the main thread
//! WKContentRuleListStore checks its disk cache first (a content-addressed
//! identifier, so a hit skips compiling) → only a miss compiles →
//! `Retained<WKContentRuleList>` (!Send) is kept in a main-thread thread_local,
//! while the Send-safe state mirror travels through `AdblockState` and
//! `browser://event`.
//!
//! Where the rules come from: `app_config_dir/adblock/*.txt` alone. Nothing is
//! bundled — third-party filter lists are copyleft (EasyList is GPLv3+/CC BY-SA
//! 3.0+ and EasyList China states no licence at all), so shipping a snapshot inside
//! the binary would be redistributing them. The rules are fetched from the
//! subscription URLs the user configured, or imported from a file they downloaded
//! themselves; a frozen snapshot also rots (EasyList's own header says
//! `Expires: 4 days`). A downloaded body that does not look like a filter list is
//! refused rather than cached — see `MIN_RULE_LINES`.
//!
//! The network is touched at two points, both from something the user did: opening a
//! browser or reader page with the switch on and no rules on disk yet (once per app
//! run — see `ensure_rules_soon`), and "update now" in the settings.

use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

use super::{emit, BrowserEvent};
use crate::AppState;

/// WebKit's limit for one rule list is 50_000 rules; this truncates to a number
/// that leaves room for the always-last `ignore_previous_fp_documents` rule and a
/// little slack.
const MAX_CB_RULES: usize = 49_998;

/// The subscription files this module writes and reads. `rules-{i}.txt` mirrors the
/// i-th entry of `adblock_rule_urls`; `import-{n}.txt` comes from a file the user
/// picked by hand. Both are loaded, so an import works as a fallback for a
/// subscription URL that cannot be reached — and, because `import-` sorts first,
/// an imported rule also wins the dedupe (see `rule_files`).
const DOWNLOAD_PREFIX: &str = "rules-";
const IMPORT_PREFIX: &str = "import-";

/// The fewest rule lines a downloaded body must carry to be cached as a
/// subscription. What this guards is not a malformed list but a network that
/// answered *in the host's place*: a captive portal or an operator interstitial
/// replies `200` with a login page, which would otherwise be written to the cache,
/// compiled into a `Ready` state that blocks nothing, and — a non-empty cache
/// directory stands `ensure_rules_soon` down for the rest of that app run and every
/// run after it — never be fetched again. A real subscription carries tens of
/// thousands of rules, so the bar sits far below any of them and far above a page
/// of prose; a hand-written list shorter than this has to come in through the file
/// import instead.
const MIN_RULE_LINES: usize = 20;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AdblockStatus {
    /// The platform does not support it (not macOS).
    Unsupported,
    Disabled,
    /// On, but there is no rule text on disk yet: nothing is bundled, so the first
    /// browser page downloads the subscriptions (and the settings offer a manual
    /// download and a file import).
    NoRules,
    Downloading,
    Compiling,
    Ready { identifier: String, rules: usize },
    Failed { error: String },
}

#[derive(Default)]
pub struct AdblockInner {
    pub status: Option<AdblockStatus>,
    /// The identifier of the previously effective rules — removed from the store's disk cache after a generation change.
    pub prev_identifier: Option<String>,
    /// Whether a download holds the single fetch slot (see `DownloadSlot`).
    /// Reserved *before* the fetch task is spawned and released when the slot is
    /// dropped, which is what makes it readable from another thread — unlike the `Downloading` status mirror, which
    /// is only set once that task runs. It keeps two things from happening: the
    /// compile worker painting `NoRules` over a fetch that is about to set
    /// `Downloading`, and a second update writing the same `rules-{i}.txt` paths
    /// concurrently with the first.
    pub downloading: bool,
    /// Whether the automatic first fetch has already been attempted in this app run.
    /// One attempt only: a failure must not turn every later browser open into
    /// another download (the user retries from the settings).
    pub auto_fetch_done: bool,
}

#[derive(Default)]
pub struct AdblockState(pub Mutex<AdblockInner>);

/// The state mirror's current status (`None` before the app state is up).
fn current_status(app: &AppHandle) -> Option<AdblockStatus> {
    app.try_state::<AppState>().and_then(|s| s.adblock.0.lock().unwrap().status.clone())
}

pub(crate) fn set_status(app: &AppHandle, status: AdblockStatus) {
    if let Some(s) = app.try_state::<AppState>() {
        s.adblock.0.lock().unwrap().status = Some(status.clone());
    }
    emit(app, &BrowserEvent::AdblockStatus { status });
}

/// The reservation itself, handed back when it is dropped — every way out of a
/// fetch, an early return and a panic included, releases the slot without anyone
/// having to remember to. Forgetting would cost the whole app run: no later
/// download could start, and the compile worker would never write `NoRules` again.
struct DownloadSlot(AppHandle);

impl Drop for DownloadSlot {
    fn drop(&mut self) {
        if let Some(state) = self.0.try_state::<AppState>() {
            state.adblock.0.lock().unwrap().downloading = false;
        }
    }
}

/// Reserve the single download slot. `None` = a fetch is already in flight (or the
/// app state is not up yet, in which case nothing else in this module works either).
fn begin_download(app: &AppHandle) -> Option<DownloadSlot> {
    let state = app.try_state::<AppState>()?;
    let mut inner = state.adblock.0.lock().unwrap();
    // Already reserved: return without a slot, and without clearing the flag the other fetch is holding
    (!std::mem::replace(&mut inner.downloading, true)).then(|| DownloadSlot(app.clone()))
}

/// Settle a `Compiling` that nothing is going to resolve. The compile worker stands
/// down without writing `NoRules` while a fetch holds the slot (see
/// `download_in_flight`); if that fetch then ends with no status of its own — no
/// source configured to fetch from — the write it skipped is nobody's job any more
/// and `Compiling` would stand until a restart. Read and set under one lock: a read
/// followed by a separate `set_status` would be the very TOCTOU the slot exists to
/// close, and could paint over a `Ready` that landed in between.
fn settle_stranded_compiling(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else { return };
    {
        let mut inner = state.adblock.0.lock().unwrap();
        if !matches!(inner.status, Some(AdblockStatus::Compiling)) {
            return;
        }
        inner.status = Some(AdblockStatus::NoRules);
    }
    emit(app, &BrowserEvent::AdblockStatus { status: AdblockStatus::NoRules });
}

fn download_in_flight(app: &AppHandle) -> bool {
    app.try_state::<AppState>().is_some_and(|s| s.adblock.0.lock().unwrap().downloading)
}

/// Rule text → (WKContentRuleList JSON, a content-addressed identifier, the rule
/// count). A pure function, compiled and tested on every platform.
pub fn compile_content_rules(lists: &[&str]) -> Result<(String, String, usize), String> {
    // debug=true is a precondition of into_content_blocking (it reads the raw rule lines)
    let mut set = adblock::lists::FilterSet::new(true);
    for text in lists {
        set.add_filter_list((*text).to_string(), adblock::lists::ParseOptions::default());
    }
    let (mut rules, _used) =
        set.into_content_blocking().map_err(|_| "the rules cannot be converted to the content-blocking format".to_string())?;
    if rules.len() > MAX_CB_RULES {
        // It cannot simply be truncated at the tail: into_content_blocking puts the
        // @@ exceptions (ignore-previous-rules) at the end, so truncating there would
        // drop the exceptions first and break sites upstream deliberately allowed.
        // Non-exception rules are cut and every exception is kept (only if it is
        // still over the limit are exceptions truncated too).
        use adblock::content_blocking::CbType;
        let (exceptions, blocks): (Vec<_>, Vec<_>) = rules
            .into_iter()
            .partition(|r| matches!(r.action.typ, CbType::IgnorePreviousRules));
        let keep = MAX_CB_RULES.saturating_sub(exceptions.len());
        let mut kept: Vec<_> = blocks.into_iter().take(keep).collect();
        kept.extend(exceptions);
        kept.truncate(MAX_CB_RULES);
        rules = kept;
    }
    // Must always be the last one: it lets first-party document requests through (an approximation of the ABP semantics)
    rules.push(adblock::content_blocking::ignore_previous_fp_documents());
    let count = rules.len();
    // `trigger.resource-type` is a HashSet in the crate, so serde writes it in
    // per-process random order. Hashed as-is, every launch produced a new
    // identifier: WKContentRuleListStore compiled the same rules again and kept
    // every generation in its disk cache (measured on one machine: 444 lists,
    // 7.3 GB). The set is order-free for WebKit, so it is sorted before hashing
    let mut value = serde_json::to_value(&rules).map_err(|e| e.to_string())?;
    if let Some(arr) = value.as_array_mut() {
        for rule in arr {
            if let Some(types) = rule.pointer_mut("/trigger/resource-type").and_then(|v| v.as_array_mut()) {
                types.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
            }
        }
    }
    let json = serde_json::to_string(&value).map_err(|e| e.to_string())?;
    // The produced JSON is content-addressed: change the rules or the compilation
    // behaviour and the identifier changes, so WKContentRuleListStore's disk cache
    // rolls over on its own
    let identifier = format!("vemry-adblock-{:016x}", fnv1a64(json.as_bytes()));
    Ok((json, identifier, count))
}

/// The heavy-resource blocking rules of the hidden reader window: extracting a body
/// needs no image/media/font, and blocking them brings onload far forward so the
/// fetch polling converges sooner. Attached to the reader window alone — a visible
/// browser window loads as usual, and images in the projection are displayed through
/// the vemry-asset proxy, decoupled from the fetching window.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // the consumer (WKContentRuleList) is macOS-only
pub const READER_BLOCK_RULES: &str = r#"[{"trigger":{"url-filter":".*","resource-type":["image","media","font"]},"action":{"type":"block"}}]"#;

/// A content-addressed identifier: change the rules and it rolls over on its own, so the store's disk cache never serves the old ones.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn reader_block_identifier() -> String {
    format!("vemry-reader-block-{:016x}", fnv1a64(READER_BLOCK_RULES.as_bytes()))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The rule cache directory (`app_config_dir/adblock`). Not created here — the
/// readers all tolerate its absence, and only a successful download creates it.
fn rule_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    crate::config_store::path(app, "adblock").ok_or_else(|| "no config dir".to_string())
}

/// Every cached rule file, in name order — paths only, so "is anything cached at
/// all?" costs one readdir instead of megabytes of rule text. The order is
/// load-bearing: `import-` sorts ahead of `rules-`, and `dedupe_rule_lines` keeps
/// the first occurrence of a line, so an imported file wins both the dedupe and the
/// WebKit rule budget against a subscription carrying the same rule.
fn rule_files(app: &AppHandle) -> Vec<std::path::PathBuf> {
    let Ok(dir) = rule_dir(app) else { return vec![] };
    // The subscription list in the settings is the truth and this directory is only
    // its cache: a `rules-{i}.txt` past the end of the current list is the tail a
    // source the user removed left behind. Ignoring it at every read is what makes
    // the removal take effect at once and stay in effect — the sweep that deletes
    // the file only runs after a download that reached the network, so on its own it
    // would leave the removed list blocking across restarts.
    let subscribed = crate::settings::load(app).adblock_rule_urls.len();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .filter(|p| file_index(p, DOWNLOAD_PREFIX).is_none_or(|i| i < subscribed))
        .collect();
    files.sort();
    files
}

/// The `i` of a `{prefix}{i}.txt` cache file — `None` for every other name (an
/// import, when the prefix is the download one, or anything a user dropped in).
fn file_index(path: &std::path::Path, prefix: &str) -> Option<usize> {
    path.file_stem()?.to_str()?.strip_prefix(prefix)?.parse().ok()
}

/// Drop cached rule files named `{prefix}{i}.txt` whose index `keep` rejects: the
/// tail a subscription the user removed left behind, and the previous import set.
/// Best effort — a file that refuses to go is retried by the next sweep, and failing
/// a download that already reached disk over it would be the worse trade.
fn sweep_rule_files(dir: &std::path::Path, prefix: &str, keep: impl Fn(usize) -> bool) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if file_index(&path, prefix).is_some_and(|i| !keep(i)) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Every cached rule file's text, deduped. Empty when nothing has been downloaded
/// or imported yet — there is no bundled fallback.
fn load_rule_texts(app: &AppHandle) -> Vec<String> {
    let texts: Vec<String> = rule_files(app).iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect();
    dedupe_rule_lines(&texts)
}

/// A line that carries a rule: not blank, not a comment (`!`), not a section header (`[..]`).
fn is_rule_line(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && !t.starts_with('!') && !t.starts_with('[')
}

/// Does this body look like a filter list at all? See `MIN_RULE_LINES` for what it
/// is really being asked: was this answered by the subscription host, or by
/// something standing in front of it?
fn looks_like_filter_list(body: &str) -> bool {
    let body = body.trim_start_matches('\u{feff}');
    if body.trim_start().starts_with('<') {
        return false; // an HTML/XML document — no filter list starts with a tag
    }
    body.lines().filter(|l| is_rule_line(l)).count() >= MIN_RULE_LINES
}

/// Drop rule lines that have already been seen in an earlier file. Two sources can
/// overlap heavily — the same list fetched from a mirror is the extreme case — and a
/// duplicate rule would compile to a second content-blocking entry, eating the
/// 50_000 budget twice over. Comments (`!`) and section headers (`[..]`) are left
/// alone: they carry no rule and are cheap to keep, and stripping them would lose
/// the `! Title:`/`! Version:` lines that make a cached file identifiable by hand.
fn dedupe_rule_lines(texts: &[String]) -> Vec<String> {
    // The set borrows out of `texts` rather than cloning every line into it: a real
    // subscription set is ~100k rule lines, and owning them would duplicate the
    // whole 2.5 MB on the heap for the length of the call.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut kept = Vec::with_capacity(texts.len());
    for text in texts {
        let mut out = String::with_capacity(text.len());
        for line in text.lines() {
            let trimmed = line.trim();
            if !is_rule_line(line) || seen.insert(trimmed) {
                out.push_str(line);
                out.push('\n');
            }
        }
        kept.push(out);
    }
    kept
}

/// The reconciliation entry point after startup or a settings change: compile or disable according to settings.
pub fn reconcile(app: &AppHandle) {
    reconcile_with(app, false);
}

/// `force`: skip the "already compiling" gate — right after a rule update reaches
/// disk it has to be recompiled, and update_rules itself has already set the state
/// to Compiling (the UI feedback during the download), so without skipping the gate
/// the new rules would never be compiled and the state would be stuck.
fn reconcile_with(app: &AppHandle, force: bool) {
    if cfg!(not(target_os = "macos")) {
        set_status(app, AdblockStatus::Unsupported);
        return;
    }
    if !crate::settings::load(app).adblock_enabled {
        // Windows that are already built keep the rules attached to them (WebKit has
        // no simple per-webview unload); new windows no longer get them — build_shell
        // checks the switch. The warm-up shell window had the rules attached when it
        // was built, so it is rebuilt, and only the next new window is really
        // unblocked
        set_status(app, AdblockStatus::Disabled);
        crate::browser::window::rebuild_pool_if_present(app);
        return;
    }
    if !force && matches!(current_status(app), Some(AdblockStatus::Compiling) | Some(AdblockStatus::Downloading)) {
        return; // already working
    }
    set_status(app, AdblockStatus::Compiling);
    let app = app.clone();
    std::thread::spawn(move || {
        let texts = load_rule_texts(&app);
        if texts.is_empty() {
            // Nothing downloaded or imported yet. Not a failure: the next browser page
            // fetches the subscriptions, and the settings offer both a manual download
            // and a file import. A download started between this thread's spawn and
            // now must not be painted over with "no rules" — that state would stick
            // for the whole download, since nothing sets `Downloading` a second time.
            // The reservation flag is what is read here rather than the status
            // mirror: it is taken before the fetch task is spawned, so it is already
            // true by the time this thread can observe it.
            if !download_in_flight(&app) {
                set_status(&app, AdblockStatus::NoRules);
            }
            return;
        }
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        match compile_content_rules(&refs) {
            Ok((json, identifier, rules)) => install_rules(app, json, identifier, rules),
            Err(e) => set_status(&app, AdblockStatus::Failed { error: e }),
        }
    });
}

/// Called from the paths that are about to load a web page (a browser window, a
/// reader fetch): with the switch on and no rules on disk, download the
/// subscriptions once. Opening a page is itself a user action that goes to the
/// network, which is what makes this fetch an extension of it rather than a new
/// egress — the switch stays off-by-default-silent everywhere else.
///
/// One attempt per app run (`auto_fetch_done`): a user behind a firewall that cannot
/// reach the subscription hosts must not pay a download timeout on every page they
/// open. The settings keep a retry and an "import from file" escape hatch.
pub fn ensure_rules_soon(app: &AppHandle) {
    if cfg!(not(target_os = "macos")) {
        return; // nothing consumes the rules here — do not spend the request
    }
    if !crate::settings::load(app).adblock_enabled {
        return;
    }
    {
        let Some(state) = app.try_state::<AppState>() else { return };
        let mut inner = state.adblock.0.lock().unwrap();
        // Only a fetch already in flight is a reason to stand down. `Compiling` is
        // not: `reconcile_with` sets it *synchronously* before spawning its thread,
        // so the toggle-on path (reconcile then ensure_rules_soon) would always find
        // it and never fetch — landing on exactly the "no rules, click again" state
        // this call exists to avoid. Whether rules exist is a question for the disk,
        // which the spawned task asks.
        if inner.downloading {
            return;
        }
        if std::mem::replace(&mut inner.auto_fetch_done, true) {
            return;
        }
        // Taken here, in the same lock, rather than inside the task: the compile
        // worker the same toggle started reads this before it may write `NoRules`,
        // and has to find the slot already reserved.
        inner.downloading = true;
    }
    // The reservation just taken, in the shape that releases it on every path out
    let slot = DownloadSlot(app.clone());
    let app = app.clone();
    // The readdir goes with the fetch, off the caller's thread: this runs while a
    // browser window is being built
    tauri::async_runtime::spawn(async move {
        if !rule_files(&app).is_empty() {
            return; // a previous run already left rules on disk; dropping `slot` hands the reservation back unused
        }
        let _ = fetch_and_reconcile(&app, slot).await;
    });
}

/// Update the rule subscriptions by hand: download → write to the cache directory →
/// rerun the pipeline. Triggered explicitly by the user from the settings.
///
/// Returns the per-source failures (empty = every source came through). A partial
/// download is still a success: the sources are independent lists, and one
/// unreachable host must not cost the user the others — which matters most exactly
/// where the default hosts are hardest to reach.
#[tauri::command]
pub async fn adblock_update_rules(app: AppHandle) -> Result<Vec<String>, String> {
    // A second click has to be turned away here: the button in one settings window
    // disables itself while it runs, but a second window's does not, and the
    // automatic first fetch does not go through a button at all. Two downloads write
    // the same `rules-{i}.txt` paths, and two writers on one path can leave a
    // spliced file behind.
    let Some(slot) = begin_download(&app) else {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.rulesUpdateRunning"));
    };
    fetch_and_reconcile(&app, slot).await
}

/// The shared body of the manual update and the automatic first fetch. It takes the
/// download slot the caller reserved (`begin_download`, or the reservation
/// `ensure_rules_soon` takes before it spawns) and releases it explicitly before the
/// recompile: the worker that one starts must not read a finished download as one
/// still in flight, or it would leave the status stuck on `Compiling`.
async fn fetch_and_reconcile(app: &AppHandle, slot: DownloadSlot) -> Result<Vec<String>, String> {
    let urls = crate::settings::load(app).adblock_rule_urls;
    if urls.is_empty() {
        // Reached with the switch just turned on and the source list emptied: the
        // compile worker found nothing on disk and stood down on this reservation,
        // and there is nothing here to fetch either. Somebody has to end the turn.
        settle_stranded_compiling(app);
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.noRuleSource"));
    }
    set_status(app, AdblockStatus::Downloading);
    let downloaded = download_rule_lists(app, &urls).await;
    drop(slot); // before the recompile below, not at the end of the function — see above
    match downloaded {
        Ok(errors) => {
            reconcile_with(app, true);
            Ok(errors)
        }
        Err(e) => {
            // On an error the state must not be left mid-flight: that would gate off every later reconcile
            set_status(app, AdblockStatus::Failed { error: e.clone() });
            Err(e)
        }
    }
}

/// Download every subscription. Each URL is fetched on its own task, so the sources
/// run concurrently — they are independent lists on independent hosts, and awaiting
/// them in turn would make the worst case the sum of the timeouts rather than the
/// longest one (exactly the case this feature exists for). A source that fails
/// leaves the copy from the previous run in place instead of taking the whole set
/// down with it. `Err` is reserved for "not one source could be fetched" (and for a
/// filesystem failure) — that is the case where the old rules are all the user has
/// and the state has to say so.
async fn download_rule_lists(app: &AppHandle, urls: &[String]) -> Result<Vec<String>, String> {
    let dir = rule_dir(app)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let handles: Vec<_> = urls
        .iter()
        .enumerate()
        .map(|(i, url)| {
            // The client is an Arc handle, so cloning it shares the connection pool
            let (client, url) = (client.clone(), url.clone());
            let path = dir.join(format!("{DOWNLOAD_PREFIX}{i}.txt"));
            tauri::async_runtime::spawn(async move {
                let fetch = async {
                    let res = client.get(&url).send().await.and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
                    let body = res.text().await.map_err(|e| e.to_string())?;
                    if !looks_like_filter_list(&body) {
                        // The user reads this one (it lands in the "some sources could
                        // not be fetched" toast), so it comes from the catalog — the
                        // transport errors it is joined with stay in whatever language
                        // the network stack produced them in
                        return Err(crate::i18n::t(crate::i18n::current(), "ipc.notAFilterList"));
                    }
                    std::fs::write(&path, &body).map_err(|e| e.to_string())
                };
                fetch.await.map_err(|e| format!("{url}: {e}"))
            })
        })
        .collect();
    let mut errors: Vec<String> = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(e),
            Err(e) => errors.push(e.to_string()),
        }
    }
    // Every task pushes at most one error, so this is "not one source came through"
    if errors.len() == urls.len() {
        return Err(errors.join("; "));
    }
    // A source the user removed leaves its file behind, and load_rule_texts reads
    // the whole directory — so the tail past the current list is cleared. Imports
    // are named apart and are never touched here.
    sweep_rule_files(&dir, DOWNLOAD_PREFIX, |i| i < urls.len());
    Ok(errors)
}

/// Import rule files the user downloaded themselves. The escape hatch for a network
/// that cannot reach the subscription hosts: the file is read locally, so nothing
/// here goes near the network. Imports replace earlier imports wholesale (picking two
/// files twice must not accumulate) and live alongside the downloaded subscriptions.
#[tauri::command]
pub async fn adblock_import_rules(app: AppHandle, paths: Vec<String>) -> Result<usize, String> {
    if paths.is_empty() {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.noRuleFile"));
    }
    let dir = rule_dir(&app)?;
    let mut texts: Vec<String> = Vec::new();
    for path in &paths {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        texts.push(text);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    sweep_rule_files(&dir, IMPORT_PREFIX, |_| false);
    for (i, text) in texts.iter().enumerate() {
        std::fs::write(dir.join(format!("{IMPORT_PREFIX}{i}.txt")), text).map_err(|e| e.to_string())?;
    }
    reconcile_with(&app, true);
    Ok(texts.len())
}

#[cfg(target_os = "macos")]
pub use imp::{ensure_reader_rules, install_rules, reader_webview_configuration, webview_configuration_with_rules};

#[cfg(not(target_os = "macos"))]
pub fn ensure_reader_rules(_app: &AppHandle) {}

#[cfg(target_os = "macos")]
mod imp {
    use super::{set_status, AdblockStatus};
    use crate::AppState;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore, WKWebView, WKWebViewConfiguration};
    use std::cell::RefCell;
    use tauri::{AppHandle, Manager};

    thread_local! {
        /// The compiled rule list. !Send — read and written on the main thread alone (both the WebKit callbacks and window building are there).
        static RULE_LIST: RefCell<Option<Retained<WKContentRuleList>>> = const { RefCell::new(None) };
    }

    /// The main-thread callback skeleton for "look up the store's disk cache → only
    /// compile on a miss". Shared by both compilation entry points (the main adblock
    /// rules and the reader blocking rules) — the retain semantics of the nested
    /// RcBlocks are maintained once. `on_fail` is called when the store is
    /// unavailable or the compilation fails (a lookup miss is not itself a failure).
    fn lookup_or_compile(
        app: &AppHandle,
        identifier: String,
        json: String,
        on_ready: impl Fn(&AppHandle, Retained<WKContentRuleList>) + Clone + Send + 'static,
        on_fail: impl Fn(&AppHandle, String) + Clone + Send + 'static,
    ) {
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            let mtm = MainThreadMarker::new().expect("run_on_main_thread");
            let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(mtm) }) else {
                on_fail(&app2, "WKContentRuleListStore is unavailable".into());
                return;
            };
            let ident_ns = NSString::from_str(&identifier);
            let lookup_app = app2.clone();
            let lookup_store = store.clone();
            let lookup = RcBlock::new(move |list: *mut WKContentRuleList, _err: *mut NSError| {
                if let Some(list) = unsafe { Retained::retain(list) } {
                    on_ready(&lookup_app, list);
                    return;
                }
                // A cache miss: really compile (on the WebKit side, hundreds of milliseconds to seconds at EasyList scale)
                let compile_app = lookup_app.clone();
                let compile_ready = on_ready.clone();
                let compile_fail = on_fail.clone();
                let compile = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
                    if let Some(list) = unsafe { Retained::retain(list) } {
                        compile_ready(&compile_app, list);
                    } else {
                        let msg = unsafe { err.as_ref() }
                            .map(|e| e.localizedDescription().to_string())
                            .unwrap_or_else(|| "compilation failed".into());
                        compile_fail(&compile_app, msg);
                    }
                });
                let ident_ns = NSString::from_str(&identifier);
                let json_ns = NSString::from_str(&json);
                unsafe {
                    lookup_store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
                        Some(&ident_ns),
                        Some(&json_ns),
                        Some(&compile),
                    );
                }
            });
            unsafe {
                store.lookUpContentRuleListForIdentifier_completionHandler(Some(&ident_ns), Some(&lookup));
            }
        });
    }

    /// Installation on the main thread once the background thread has produced the
    /// JSON: lookup/compile → store it in the thread_local, broadcast the state and
    /// attach it to windows that are already open (see finish_ready).
    pub fn install_rules(app: AppHandle, json: String, identifier: String, rules: usize) {
        let ident = identifier.clone();
        lookup_or_compile(
            &app,
            identifier,
            json,
            move |app, list| finish_ready(app, list, &ident, rules),
            |app, error| set_status(app, AdblockStatus::Failed { error }),
        );
    }

    /// The main-thread callback: the rule list is ready. Store it in the
    /// thread_local, clear the previous generation from the disk cache, broadcast the
    /// state and attach it to browser windows that are already open (a runtime
    /// addContentRuleList, effective from the next navigation).
    fn finish_ready(app: &AppHandle, list: Retained<WKContentRuleList>, identifier: &str, rules: usize) {
        RULE_LIST.with(|c| c.replace(Some(list)));
        // Generation cleanup: a previous identifier that differs is deleted from the store's disk cache
        if let Some(state) = app.try_state::<AppState>() {
            let prev = {
                let mut inner = state.adblock.0.lock().unwrap();
                let prev = inner.prev_identifier.take();
                inner.prev_identifier = Some(identifier.to_string());
                prev
            };
            if let Some(prev) = prev.filter(|p| p != identifier) {
                if let Some(mtm) = MainThreadMarker::new() {
                    if let Some(store) = unsafe { WKContentRuleListStore::defaultStore(mtm) } {
                        let prev_ns = NSString::from_str(&prev);
                        let noop = RcBlock::new(move |_err: *mut NSError| {});
                        unsafe {
                            store.removeContentRuleListForIdentifier_completionHandler(
                                Some(&prev_ns),
                                Some(&noop),
                            );
                        }
                    }
                }
            }
        }
        set_status(app, AdblockStatus::Ready { identifier: identifier.to_string(), rules });
        attach_to_existing(app);
        // Rebuild the warm-up shell window: a runtime attachment can only add (the
        // old list cannot be unloaded), and only a rebuild carries the new rules
        // cleanly; a visible window cannot be rebuilt, so the existing compromise of
        // attaching stands there
        crate::browser::window::rebuild_pool_if_present(app);
    }

    thread_local! {
        /// The heavy-resource blocking rule list (image/media/font) belonging to the
        /// reader window. Kept apart from the adblock rule list — it does not follow
        /// the adblock switch and always applies to the reader window.
        static READER_RULE_LIST: RefCell<Option<Retained<WKContentRuleList>>> = const { RefCell::new(None) };
    }

    /// Precompile the reader blocking rules at startup (two rules, a matter of
    /// milliseconds; a hit in the store's disk cache skips compiling). A failure is
    /// silent — without the rules the fetching is merely slower, and correctness is
    /// unaffected.
    pub fn ensure_reader_rules(app: &AppHandle) {
        lookup_or_compile(
            app,
            super::reader_block_identifier(),
            super::READER_BLOCK_RULES.to_string(),
            finish_reader_ready,
            |_, _| {},
        );
    }

    /// The rules are ready: store them in the thread_local; should the reader window
    /// have been built before the compilation finished, attach them at runtime
    /// (effective from the next navigation).
    fn finish_reader_ready(app: &AppHandle, list: Retained<WKContentRuleList>) {
        READER_RULE_LIST.with(|c| c.replace(Some(list)));
        if let Some(webview) = app.get_webview_window(crate::browser::reader::READER_LABEL) {
            attach_list_to(webview.as_ref(), &READER_RULE_LIST);
        }
    }

    /// Attach a rule list at runtime to the native WKWebView of a window that is
    /// already built (effective from the next load).
    /// The with_webview closure runs on the main thread, where the thread_local's list
    /// can be read.
    fn attach_list_to(
        webview: &tauri::Webview,
        cell: &'static std::thread::LocalKey<RefCell<Option<Retained<WKContentRuleList>>>>,
    ) {
        let _ = webview.with_webview(|pw| {
            cell.with(|c| {
                if let Some(list) = c.borrow().as_ref() {
                    let wk = pw.inner().cast::<WKWebView>();
                    if let Some(wk) = unsafe { wk.as_ref() } {
                        unsafe { wk.configuration().userContentController().addContentRuleList(list) };
                    }
                }
            });
        });
    }

    /// Build a window configuration with the given rule lists pre-attached; None when
    /// there is not one list (the default configuration is used).
    /// Called on the main thread.
    fn configuration_with(
        lists: Vec<Retained<WKContentRuleList>>,
    ) -> Option<Retained<WKWebViewConfiguration>> {
        if lists.is_empty() {
            return None;
        }
        let mtm = MainThreadMarker::new()?;
        let cfg = unsafe { WKWebViewConfiguration::init(WKWebViewConfiguration::alloc(mtm)) };
        let ucc = unsafe { cfg.userContentController() };
        for list in &lists {
            unsafe { ucc.addContentRuleList(list) };
        }
        Some(cfg)
    }

    /// The reader window's build configuration: heavy-resource blocking plus the adblock rules (when the switch is on and they are ready).
    pub fn reader_webview_configuration(adblock_enabled: bool) -> Option<Retained<WKWebViewConfiguration>> {
        let mut lists: Vec<_> = READER_RULE_LIST.with(|c| c.borrow().clone()).into_iter().collect();
        if adblock_enabled {
            lists.extend(RULE_LIST.with(|c| c.borrow().clone()));
        }
        configuration_with(lists)
    }

    /// A new browser window's build configuration: the adblock rules are pre-attached when ready (so the first frame already blocks).
    pub fn webview_configuration_with_rules() -> Option<Retained<WKWebViewConfiguration>> {
        configuration_with(RULE_LIST.with(|c| c.borrow().clone()).into_iter().collect())
    }

    /// Attachment for tabs built before the compilation was ready: a runtime
    /// addContentRuleList on the native WKWebView of every registered tab
    /// (effective from the next load).
    fn attach_to_existing(app: &AppHandle) {
        let labels: Vec<String> = app
            .try_state::<AppState>()
            .map(|s| s.browser.0.lock().unwrap().tabs.keys().cloned().collect())
            .unwrap_or_default();
        for label in labels {
            if let Some(webview) = app.get_webview(&label) {
                attach_list_to(&webview, &RULE_LIST);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install_rules(app: AppHandle, _json: String, _identifier: String, _rules: usize) {
    set_status(&app, AdblockStatus::Unsupported);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RULES: &str = "\
[Adblock Plus 2.0]
! Title: sample
||ads.example.com^
||tracker.example.net^$third-party
@@||goodsite.example.com/ads/allowed^
example.com###ad-banner
";

    #[test]
    fn compile_produces_wk_json_shape() {
        let (json, identifier, count) = compile_content_rules(&[SAMPLE_RULES]).unwrap();
        assert!(count >= 3, "expected several rules, got {count}");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), count);
        // Every rule takes the shape {trigger: {url-filter}, action: {type}}
        for rule in arr {
            assert!(rule.get("trigger").and_then(|t| t.get("url-filter")).is_some(), "{rule}");
            assert!(rule.get("action").and_then(|a| a.get("type")).is_some(), "{rule}");
        }
        // Always last: ignore-previous-rules (letting first-party documents through)
        let last = arr.last().unwrap();
        assert_eq!(last["action"]["type"], "ignore-previous-rules");
        assert!(identifier.starts_with("vemry-adblock-"));
    }

    #[test]
    fn identifier_is_content_addressed_and_stable() {
        let (_, id1, _) = compile_content_rules(&[SAMPLE_RULES]).unwrap();
        let (_, id2, _) = compile_content_rules(&[SAMPLE_RULES]).unwrap();
        assert_eq!(id1, id2, "the same input must give the same identifier (the disk cache hit depends on it)");
        let (_, id3, _) = compile_content_rules(&[SAMPLE_RULES, "||more.example.org^"]).unwrap();
        assert_ne!(id1, id3, "changed rules must roll the identifier over");
    }

    #[test]
    fn identifier_does_not_depend_on_hash_set_order() {
        // Several resource types on one rule: the crate keeps them in a HashSet,
        // and an unsorted serialisation rolled the identifier on every launch
        let rules = "[Adblock Plus 2.0]\n||ads.example.com^$script,image,stylesheet,font,media\n";
        let (json, _, _) = compile_content_rules(&[rules]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut seen = false;
        for rule in parsed.as_array().unwrap() {
            if let Some(types) = rule.pointer("/trigger/resource-type").and_then(|v| v.as_array()) {
                if types.len() > 1 {
                    seen = true;
                    let names: Vec<&str> = types.iter().map(|t| t.as_str().unwrap()).collect();
                    let mut sorted = names.clone();
                    sorted.sort();
                    assert_eq!(names, sorted, "{rule}");
                }
            }
        }
        assert!(seen, "the sample must yield a rule with several resource types");
    }

    #[test]
    fn oversized_lists_truncate_with_trailing_exception() {
        // Build a rule set past the limit
        let mut big = String::from("[Adblock Plus 2.0]\n");
        for i in 0..(MAX_CB_RULES + 500) {
            big.push_str(&format!("||ads{i}.example.com^\n"));
        }
        let (json, _, count) = compile_content_rules(&[&big]).unwrap();
        assert_eq!(count, MAX_CB_RULES + 1, "truncated to the limit plus the one always-last rule");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.as_array().unwrap().last().unwrap()["action"]["type"], "ignore-previous-rules");
    }

    #[test]
    fn reader_block_rules_are_valid_content_blocking_json() {
        let parsed: serde_json::Value = serde_json::from_str(READER_BLOCK_RULES).unwrap();
        let arr = parsed.as_array().unwrap();
        assert!(!arr.is_empty());
        for rule in arr {
            assert!(rule.get("trigger").and_then(|t| t.get("url-filter")).is_some(), "{rule}");
            assert_eq!(rule["action"]["type"], "block", "{rule}");
            let kinds: Vec<&str> = rule["trigger"]["resource-type"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|v| v.as_str())
                .collect();
            // Heavy resources only — script/style/document must not be on this list,
            // or the page itself or its rendering logic would break
            for k in &kinds {
                assert!(matches!(*k, "image" | "media" | "font"), "unexpected resource type: {k}");
            }
        }
        assert!(reader_block_identifier().starts_with("vemry-reader-block-"));
    }

    #[test]
    fn several_lists_compile_together() {
        // Two sources at once, the shape of a real subscription set: everything is
        // merged into one rule list (no snapshot is bundled any more, so the sizes
        // here are the fixture's, not EasyList's)
        let second = "[Adblock Plus 2.0]\n||banner.example.org^\nexample.org###promo\n";
        let (json, _, count) = compile_content_rules(&[SAMPLE_RULES, second]).unwrap();
        assert!(count >= 5, "both sources must contribute rules, got {count}");
        assert!(count <= MAX_CB_RULES + 1);
        // The url-filter is a regex, so the host appears with its dots escaped
        assert!(json.contains("banner"), "the second source is missing from the output: {json}");
    }

    #[test]
    fn duplicate_rule_lines_are_dropped_across_sources() {
        // The same list from a mirror: identical rules must not be compiled twice, or
        // they eat the WebKit rule budget for nothing
        let deduped = dedupe_rule_lines(&[SAMPLE_RULES.to_string(), SAMPLE_RULES.to_string()]);
        let (_, _, once) = compile_content_rules(&[SAMPLE_RULES]).unwrap();
        let refs: Vec<&str> = deduped.iter().map(String::as_str).collect();
        let (_, _, twice) = compile_content_rules(&refs).unwrap();
        assert_eq!(once, twice, "a duplicated source must compile to the same rule count");
        // Comments and headers survive: a cached file stays identifiable by hand
        assert!(deduped[1].contains("! Title: sample"));
        assert!(!deduped[1].contains("||ads.example.com^"));
    }

    #[test]
    fn distinct_rules_survive_dedupe() {
        let other = "[Adblock Plus 2.0]\n||other.example.net^\n";
        let deduped = dedupe_rule_lines(&[SAMPLE_RULES.to_string(), other.to_string()]);
        assert!(deduped[1].contains("||other.example.net^"));
    }

    #[test]
    fn a_page_answered_in_the_hosts_place_is_not_cached_as_rules() {
        // The captive-portal case: 200 OK carrying a login page. Cached, it would
        // compile into a Ready state that blocks nothing — and a non-empty cache
        // directory means it would never be fetched over.
        assert!(!looks_like_filter_list("<!DOCTYPE html>\n<html><body>Sign in to continue</body></html>\n"));
        assert!(!looks_like_filter_list("\u{feff}  \n<html>a page behind a BOM and blank space</html>"));
        assert!(!looks_like_filter_list("Service Unavailable"));
        // A header and comments carry no rule at all
        assert!(!looks_like_filter_list("[Adblock Plus 2.0]\n! Title: sample\n! Expires: 4 days\n"));
    }

    #[test]
    fn a_real_subscription_passes_the_guard() {
        let mut list = String::from("[Adblock Plus 2.0]\n! Title: sample\n");
        for i in 0..MIN_RULE_LINES {
            list.push_str(&format!("||ads{i}.example.com^\n"));
        }
        assert!(looks_like_filter_list(&list));
        // A hosts-style list too: its `#` comments count as rule lines here, which
        // only makes the guard more permissive — being permissive is the point, the
        // bar exists for pages of prose, not for lists in an unexpected dialect
        let hosts: String = (0..MIN_RULE_LINES).map(|i| format!("0.0.0.0 ads{i}.example.net\n")).collect();
        assert!(looks_like_filter_list(&hosts));
    }

    #[test]
    fn only_matching_names_carry_an_index() {
        use std::path::Path;
        // What the sweep deletes and what the read-time filter drops both hang off this
        assert_eq!(file_index(Path::new("/x/rules-2.txt"), DOWNLOAD_PREFIX), Some(2));
        assert_eq!(file_index(Path::new("/x/import-3.txt"), IMPORT_PREFIX), Some(3));
        assert_eq!(file_index(Path::new("/x/import-0.txt"), DOWNLOAD_PREFIX), None);
        // Anything else the user put in the directory is left alone by both
        assert_eq!(file_index(Path::new("/x/mine.txt"), DOWNLOAD_PREFIX), None);
        assert_eq!(file_index(Path::new("/x/rules-old.txt"), DOWNLOAD_PREFIX), None);
    }
}
