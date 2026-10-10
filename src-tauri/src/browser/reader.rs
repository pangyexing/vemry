//! The fetching channel of the reading projection (v0.5 P2).
//!
//! A hidden `reader` window loads the page, the clipping pipeline's `extract_page`
//! extracts the body, and the main window's centre pane renders the Markdown — a
//! citation click lands in the projection by default, and a visible browser window is
//! the escape hatch.
//!
//! Fetching does not wait for onload: after navigating it polls readyState, tries a
//! grab as soon as the DOM is usable, and returns the moment the extraction is good
//! enough (not a fallback, and past the body threshold) — onload waits for every image
//! and font, which on a heavy page means seconds of waiting for nothing. Whether the
//! document changed is decided by two criteria, "the old document's marker" plus the
//! URL: before a navigation commits, an eval still reaches the old document, and
//! readyState alone would return the old page as the new one.
//!
//! Its security boundary is exactly that of a visible browser window: the label
//! matches no capability (zero IPC), the same navigation guard, and the same adblock
//! rules pre-attached at build time (plus the reader's own image/media/font blocking —
//! extracting a body needs no heavy resources). Fetching still goes through that same
//! controlled network egress, "a browser window"; the window is merely invisible. It
//! does **not** enter the `BrowserState` registry: the tab strip, the clip commands
//! and docking cannot see it.
//!
//! The projected content is only shown by the main window's frontend — it never
//! reaches disk, the block index or a prompt; writing something down still goes
//! through the user's explicit clip or "add to context".

use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use vemry_clip::{compose_clip_note, extract_page, quote_block, ClipMeta, ExtractedPage};

use super::clip::{eval_json, land_note, str_field, GRAB_PAGE};
use super::window::{is_app_origin, normalize_url};
use crate::AppState;

pub const READER_LABEL: &str = "reader";

/// How many projection pages are cached. Reopening the same URL is instant, and
/// clipping or a quote block reads the cache directly: no refetch, and byte-identical
/// to the projection. The entries are plain text, so 32 of them cost negligible
/// memory.
const CACHE_CAP: usize = 32;
/// The hard limit on the whole fetch (a backstop for an extremely slow page; the normal path returned long before).
const LOAD_TIMEOUT: Duration = Duration::from_secs(15);
/// SPA settle: with readyState already complete but the result still thin or a fallback, wait this long and grab once more for a better one.
const SETTLE_DELAY: Duration = Duration::from_millis(1000);
/// Markdown shorter than this many characters counts as "not finished rendering" and does not qualify.
const THIN_MARKDOWN: usize = 200;
/// The polling rhythm: dense for the first 3 seconds (so a page with a fast first byte returns as soon as possible), slower after that.
const POLL_FAST: Duration = Duration::from_millis(150);
const POLL_SLOW: Duration = Duration::from_millis(400);
const FAST_POLLS: u32 = 20;

/// A light probe: whether the document changed (mark/url) + readiness (readyState) +
/// how much text there is (and whether it changed, which decides whether a regrab is
/// worth it — serialising outerHTML is not cheap on a large page).
const PROBE: &str = "(() => ({ mark: window.__vemryReadMark === true, url: location.href, \
     ready: document.readyState, len: document.body ? document.body.textContent.length : 0 }))()";
/// Mark the old document and record its URL before navigating. A new document is a new
/// global, so the marker disappears by itself; a same-document navigation (a bare
/// #fragment) keeps the marker but changes the URL — the two legs cover both kinds of
/// page change.
const MARK_PROBE: &str = "(() => { window.__vemryReadMark = true; return { url: location.href }; })()";

/// Injected at document-start (in the reader window only): a hidden window renders
/// nothing, so an IntersectionObserver never fires — and a lazy-loading library's
/// "write the real URL into src" therefore never happens either (which is the root
/// cause of images being fine in a visible browser window and all placeholders in the
/// projection). This replaces IO with a stub that intersects the moment it is
/// registered, so a lazy-loading library swaps them in right after mounting;
/// GRAB_PAGE's attribute recovery handles the kind that has a data-src in the DOM, and
/// this handles the kind whose real URL lives only in JS state. The side effect (an
/// IO-driven infinite scroll loading forever) is held by the fetch loop's convergence
/// and its timeout, and affects this hidden window alone.
const EAGER_LAZY_INIT: &str = r#"(() => {
  if (!window.IntersectionObserver) return;
  window.IntersectionObserver = class IntersectionObserver {
    constructor(cb) { this._cb = cb; }
    get root() { return null; }
    get rootMargin() { return '0px'; }
    get thresholds() { return [0]; }
    observe(el) {
      const entry = {
        isIntersecting: true, intersectionRatio: 1, target: el, time: 0,
        boundingClientRect: null, intersectionRect: null, rootBounds: null,
      };
      try { entry.boundingClientRect = entry.intersectionRect = el.getBoundingClientRect(); } catch (_) {}
      queueMicrotask(() => { try { this._cb([entry], this); } catch (_) {} });
    }
    unobserve() {}
    disconnect() {}
    takeRecords() { return []; }
  };
})();"#;

#[derive(Default)]
pub struct ReaderState(pub Mutex<ReaderInner>);

#[derive(Default)]
pub struct ReaderInner {
    /// The request generation: incremented by every page_read, and the waits and results of an older generation are all void.
    generation: u64,
    cache: Vec<CachedPage>,
}

pub struct CachedPage {
    /// The actual location.href at grab time (after redirects).
    url: String,
    /// The URL the user asked for (before redirects) — a citation click carries this one, so a hit has to recognise it.
    requested: String,
    /// document.title (as of the grab); the fallback when the extracted title is empty.
    title: String,
    page: ExtractedPage,
}

impl ReaderInner {
    fn cache_insert(&mut self, requested: String, url: String, title: String, page: ExtractedPage) {
        self.cache.retain(|c| c.url != url);
        self.cache.insert(0, CachedPage { url, requested, title, page });
        self.cache.truncate(CACHE_CAP);
    }

    fn cache_get(&self, url: &str) -> Option<&CachedPage> {
        self.cache.iter().find(|c| c.url == url || c.requested == url)
    }

    /// The projected page for `url`, if it is still cached: its final URL, its
    /// title and its Markdown (what side-by-side translation cuts into segments).
    pub(crate) fn cached_page(&self, url: &str) -> Option<(String, String, String)> {
        self.cache_get(url).map(|c| (c.url.clone(), c.title.clone(), c.page.markdown.clone()))
    }
}

/// The payload of `reading://event`: Rust only sends signals, and the projection is state in the main window's frontend.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReadingEvent {
    Open { url: String },
}

pub fn emit_reading(app: &AppHandle, event: &ReadingEvent) {
    let _ = app.emit("reading://event", event);
}

/// The projection's result. `url` is the actual location.href at grab time (after
/// redirects) — the frontend's display, relative link resolution and the clip cache
/// lookup all go by it.
#[derive(Serialize)]
pub struct ReadPage {
    pub url: String,
    pub title: String,
    pub byline: Option<String>,
    pub markdown: String,
    /// Readability's extraction failed and this took the whole-page fallback — the frontend shows a "the layout may be poor" banner from it.
    pub fallback: bool,
}

fn read_page_of(url: String, doc_title: &str, page: &ExtractedPage) -> ReadPage {
    let title = if page.title.trim().is_empty() {
        doc_title.to_string()
    } else {
        page.title.trim().to_string()
    };
    ReadPage {
        url,
        title,
        byline: page.byline.clone(),
        markdown: page.markdown.clone(),
        fallback: page.fallback,
    }
}

/// Pick the better of two grabs: first by "did it take the fallback", then by body length.
fn score(page: &ExtractedPage) -> (bool, usize) {
    (!page.fallback, page.markdown.chars().count())
}

/// A good enough extraction = worth returning at once, without waiting for slow resources.
fn grab_good(page: &ExtractedPage) -> bool {
    !page.fallback && page.markdown.chars().count() >= THIN_MARKDOWN
}

/// One grab: outerHTML → extract_page. Returns (the actual URL, document.title, the body).
async fn grab(webview: &tauri::WebviewWindow) -> Result<(String, String, ExtractedPage), String> {
    let data = eval_json(webview.as_ref(), &GRAB_PAGE).await?;
    let html = str_field(&data, "html");
    if html.trim().is_empty() {
        return Err("the page has not finished loading".into());
    }
    let url = str_field(&data, "url");
    let page = extract_page(&html, &url).map_err(|e| e.to_string())?;
    Ok((url, str_field(&data, "title"), page))
}

/// Merge the better one: a new grab only replaces the existing result when it scores higher.
fn merge_best(best: &mut Option<(String, String, ExtractedPage)>, grabbed: (String, String, ExtractedPage)) {
    let better = match best.as_ref() {
        Some(b) => score(&grabbed.2) > score(&b.2),
        None => true,
    };
    if better {
        *best = Some(grabbed);
    }
}

/// Fetching for the reading projection: the hidden reader window loads `url`, extracts
/// the body and returns Markdown.
/// When clicking citations in quick succession supersedes an older request it returns
/// `None` — being superseded is normal rather than an error, and the frontend ignores
/// a null silently instead of entering an error state.
#[tauri::command]
pub async fn page_read(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
) -> Result<Option<ReadPage>, String> {
    let url = normalize_url(&url)?;
    // The hidden reader window gets the blocking rules too
    super::adblock::ensure_rules_soon(&app);

    // A cache hit is instant. Except for a fallback page — that is grabbed once more in
    // the hope of a better extraction (the page may not have finished loading before);
    // a non-fallback page counts as stable within the session, which matches the "no
    // refetch" semantics of the frontend's back and forward
    {
        let inner = state.reader.0.lock().unwrap();
        if let Some(c) = inner.cache_get(url.as_str()).filter(|c| !c.page.fallback) {
            return Ok(Some(read_page_of(c.url.clone(), &c.title, &c.page)));
        }
    }

    let generation = {
        let mut inner = state.reader.0.lock().unwrap();
        inner.generation += 1;
        inner.generation
    };
    let superseded = || state.reader.0.lock().unwrap().generation != generation;

    // The old document's marker has to be settled before navigating (an awaited callback), or it might land on the new document instead
    let old_url = if let Some(webview) = app.get_webview_window(READER_LABEL) {
        let marked = eval_json(webview.as_ref(), MARK_PROBE).await.ok();
        marked.map(|v| str_field(&v, "url")).unwrap_or_default()
    } else {
        "about:blank".to_string()
    };

    // Building and navigating are serialised on the main thread with a generation check:
    // - on a concurrent first build both requests may see "no window", so it is checked
    //   again at the point of execution and a window that now exists is navigated
    //   instead, rather than letting the later builder fail with "the label already
    //   exists" on top of the new request the user actually wants
    // - a stale request's build or navigation is simply dropped at the point of
    //   execution, so it cannot land after a newer request and grab (and cache)
    //   somebody else's page as its own result
    let (btx, brx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let app2 = app.clone();
    let url2 = url.clone();
    app.run_on_main_thread(move || {
        let result = (|| {
            let stale = app2
                .try_state::<AppState>()
                .map(|s| s.reader.0.lock().unwrap().generation != generation)
                .unwrap_or(true);
            if stale {
                return Ok(()); // this request will return None at the polling point shortly
            }
            match app2.get_webview_window(READER_LABEL) {
                Some(w) => w.navigate(url2).map_err(|e| e.to_string()),
                None => build_reader_window(&app2, &url2),
            }
        })();
        let _ = btx.send(result);
    })
    .map_err(|e| e.to_string())?;
    brx.await.map_err(|_| "window creation was interrupted".to_string())??;

    let deadline = Instant::now() + LOAD_TIMEOUT;
    let mut best: Option<(String, String, ExtractedPage)> = None;
    let mut last_len: i64 = -1;
    let mut saw_new_doc = false;
    let mut polls: u32 = 0;
    loop {
        let timed_out = Instant::now() >= deadline;
        if !timed_out {
            tokio::time::sleep(if polls < FAST_POLLS { POLL_FAST } else { POLL_SLOW }).await;
        }
        polls += 1;
        if superseded() {
            return Ok(None);
        }
        let webview = app.get_webview_window(READER_LABEL).ok_or("the reading fetch window is closed")?;
        let Ok(probe) = eval_json(webview.as_ref(), PROBE).await else {
            // An eval can fail at the instant a navigation switches over, which is not fatal
            if timed_out {
                break;
            }
            continue;
        };
        let mark = probe.get("mark").and_then(|v| v.as_bool()).unwrap_or(false);
        let ready = str_field(&probe, "ready");
        let len = probe.get("len").and_then(|v| v.as_i64()).unwrap_or(0);
        // The marker is still there and the URL has not changed = the new document has not committed, and grabbing now would only reach the old page
        if mark && str_field(&probe, "url") == old_url {
            if timed_out {
                break;
            }
            continue;
        }
        saw_new_doc = true;
        if ready == "loading" {
            if timed_out {
                break;
            }
            continue;
        }
        // No change in the amount of text means no regrab; a stable length after complete = the content will not grow any further, so it converges
        if len == last_len && best.is_some() {
            if ready == "complete" || timed_out {
                break;
            }
            continue;
        }
        last_len = len;
        if let Ok(grabbed) = grab(&webview).await {
            let good = grab_good(&grabbed.2);
            merge_best(&mut best, grabbed);
            if good {
                break; // an early return: the body is readable, so no waiting for onload and slow resources
            }
            if ready == "complete" {
                // SPA settle: onload has passed but it is still thin or a fallback, so wait one round, grab the better one and converge
                tokio::time::sleep(SETTLE_DELAY).await;
                if superseded() {
                    return Ok(None);
                }
                if let Ok(retry) = grab(&webview).await {
                    merge_best(&mut best, retry);
                }
                break;
            }
        }
        if timed_out {
            break;
        }
    }

    if superseded() {
        return Ok(None);
    }
    // The backstop: nothing was ever grabbed before the timeout, but the new document is confirmed — force one last grab
    if best.is_none() && saw_new_doc {
        if let Some(webview) = app.get_webview_window(READER_LABEL) {
            if let Ok(grabbed) = grab(&webview).await {
                merge_best(&mut best, grabbed);
            }
        }
    }
    let (final_url, doc_title, page) = best.ok_or("the page timed out while loading")?;
    if superseded() {
        return Ok(None);
    }

    let read = read_page_of(final_url.clone(), &doc_title, &page);
    state.reader.0.lock().unwrap().cache_insert(url.to_string(), final_url, doc_title, page);
    Ok(Some(read))
}

/// Build the hidden window on the main thread: the same navigation guard as a visible
/// browser window, with the adblock rules plus the reader's own heavy-resource
/// blocking pre-attached (see adblock::reader_webview_configuration); it is never
/// shown.
fn build_reader_window(app: &AppHandle, url: &tauri::Url) -> Result<(), String> {
    #[allow(unused_mut)]
    let mut builder = WebviewWindowBuilder::new(app, READER_LABEL, WebviewUrl::External(url.clone()))
        .title(crate::i18n::t(crate::i18n::current(), "window.readerTitle"))
        .inner_size(1080.0, 900.0)
        .visible(false)
        .focused(false)
        .initialization_script(EAGER_LAZY_INIT)
        .on_navigation(|target| matches!(target.scheme(), "http" | "https") && !is_app_origin(target))
        // A hidden window has no interaction context, so a popup is simply swallowed (a visible browser window opens it back in the same window)
        .on_new_window(|_, _| NewWindowResponse::Deny);

    #[cfg(target_os = "macos")]
    {
        let adblock = crate::settings::load(app).adblock_enabled;
        if let Some(cfg) = super::adblock::reader_webview_configuration(adblock) {
            builder = builder.with_webview_configuration(cfg);
        }
    }

    builder.build().map_err(|e| e.to_string())?;
    Ok(())
}

/// Clipping from the projection: the cache is used directly to compose a note into
/// clips/ — no refetch, and byte-identical to the projection. Once it lands, image
/// localisation follows the same chain as a whole-page clip from a browser window
/// (maybe_localize, exactly as clip_page does).
#[tauri::command]
pub fn reading_clip(app: AppHandle, state: State<'_, AppState>, url: String) -> Result<String, String> {
    let (meta, page) = {
        let inner = state.reader.0.lock().unwrap();
        let cached = inner.cache_get(&url).ok_or("the projection cache has expired; reopen the page before clipping")?;
        let meta = ClipMeta {
            url: cached.url.clone(),
            title: cached.title.clone(),
            clipped_at: chrono::Local::now(),
        };
        (meta, cached.page.clone())
    };
    let rel = land_note(&app, &state, &meta, &compose_clip_note(&page, &meta, &crate::i18n::t(crate::i18n::current(), "clip.sourceLabel")))?;
    super::clip::maybe_localize(&app, rel.clone(), meta.url.clone());
    Ok(rel)
}

/// A projection selection → quote callout text (the frontend's insertAtCursor puts it
/// into the current note). The selection is right there in the main webview, so none
/// of the browser window's "keep the selection through a focus loss" gymnastics are
/// needed. `html` is the fragment the frontend restored from the selection's
/// cloneContents (proxied images and placeholder chips are back to their original
/// URLs, and rendered math is back to text) — htmd converts it to Markdown so images
/// and links survive; with nothing convertible it falls back to the plain-text
/// `selection`.
#[tauri::command]
pub fn reading_quote(
    state: State<'_, AppState>,
    url: String,
    selection: String,
    html: String,
) -> Result<String, String> {
    if selection.trim().is_empty() && html.trim().is_empty() {
        return Err("nothing is selected".into());
    }
    let title = {
        let inner = state.reader.0.lock().unwrap();
        inner
            .cache_get(&url)
            .map(|c| if c.page.title.trim().is_empty() { c.title.clone() } else { c.page.title.clone() })
            .unwrap_or_default()
    };
    let meta = ClipMeta { url, title, clipped_at: chrono::Local::now() };
    let body = match vemry_clip::fragment_to_markdown(&html) {
        md if !md.is_empty() => md,
        _ => selection,
    };
    Ok(quote_block(&body, &meta, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(markdown: &str, fallback: bool) -> ExtractedPage {
        ExtractedPage { markdown: markdown.into(), title: String::new(), byline: None, fallback }
    }

    #[test]
    fn score_prefers_readability_then_length() {
        // Not a fallback beats a fallback, even when the fallback's body is longer
        assert!(score(&page("短", false)) > score(&page("很长很长的降级正文", true)));
        // Between two non-fallbacks, the longer wins
        assert!(score(&page("较长的正文", false)) > score(&page("短", false)));
    }

    #[test]
    fn grab_good_requires_readability_and_length() {
        assert!(!grab_good(&page("短", false)), "too short does not qualify");
        assert!(!grab_good(&page(&"长".repeat(300), true)), "a fallback does not qualify");
        assert!(grab_good(&page(&"长".repeat(300), false)));
    }

    #[test]
    fn merge_best_keeps_higher_score() {
        let mut best = None;
        merge_best(&mut best, ("u1".into(), "t1".into(), page("短", true)));
        assert_eq!(best.as_ref().unwrap().0, "u1");
        // A better one (not a fallback) replaces it
        merge_best(&mut best, ("u2".into(), "t2".into(), page("好", false)));
        assert_eq!(best.as_ref().unwrap().0, "u2");
        // A worse one (a fallback) does not
        merge_best(&mut best, ("u3".into(), "t3".into(), page("很长很长的降级", true)));
        assert_eq!(best.as_ref().unwrap().0, "u2");
    }

    #[test]
    fn cache_dedups_and_caps() {
        let mut inner = ReaderInner::default();
        for i in 0..(CACHE_CAP + 3) {
            inner.cache_insert(
                format!("https://e.com/{i}?from=cite"),
                format!("https://e.com/{i}"),
                String::new(),
                page("x", false),
            );
        }
        assert_eq!(inner.cache.len(), CACHE_CAP);
        // Reinserting the same (final) URL deduplicates and moves it to the front
        let last = format!("https://e.com/{}", CACHE_CAP + 2);
        inner.cache_insert(last.clone(), last.clone(), "t".into(), page("y", false));
        assert_eq!(inner.cache.len(), CACHE_CAP);
        assert_eq!(inner.cache[0].url, last);
        assert_eq!(inner.cache[0].page.markdown, "y");
    }

    #[test]
    fn cache_hits_on_requested_url_too() {
        // A citation click carries the pre-redirect URL — a hit has to recognise both
        let mut inner = ReaderInner::default();
        inner.cache_insert(
            "http://e.com/short".into(),
            "https://e.com/final".into(),
            String::new(),
            page("x", false),
        );
        assert!(inner.cache_get("https://e.com/final").is_some());
        assert!(inner.cache_get("http://e.com/short").is_some());
        assert!(inner.cache_get("https://nowhere.example/x").is_none());
    }
}
