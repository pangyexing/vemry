//! Translating a browser tab where it is: a pane opens inside the page — the
//! page on the left, every text block's translation on the right at the height
//! of its block — so the reader stays in the tab, docked or detached, with no
//! second window and no other view. The page is a native webview with zero IPC,
//! so everything goes through `eval`: a script installs the pane and collects
//! the page's text blocks (each tagged with an id), the job in vemry-agent
//! translates them, and every finished segment is written into the pane as text.
//! The pane lays its entries out from the blocks' positions on every scroll and
//! resize (and once a second, for content that shifts), pushing an entry down
//! when the one above it is taller than its block; a poll every second and a
//! half picks up blocks that appeared since (an endless feed, a page rendering as
//! it scrolls); a new document (a navigation, a reload) starts over. Pressing the
//! button again removes the pane and stops.
//!
//! What is collected is the body: a block with running text and no block
//! children, outside navigation, headers, footers, asides, figure captions,
//! tables, reference lists and bylines. Text goes in with `textContent`, never
//! as HTML; the page's only new global is `__vemryTr`, the precedent being
//! `__vemrySavedSelection`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use tauri::{AppHandle, Manager, State};
use vemry_agent::translate::{TranslateDoc, TranslateEvent, TranslateJob, TranslationCache};
use vemry_agent::HttpTransport;
use vemry_core::{SegKind, Segment};

use super::clip::eval_json;
use super::window::target_webview;
use super::{emit, BrowserEvent};
use crate::{settings, AppState};

const POLL_MS: u64 = 1000;

/// Install the pane (once per document) and collect the text blocks not yet
/// tagged. `gen` is minted when the script first runs in a document, so a
/// different value on the next poll means a different document.
const COLLECT_JS: &str = r#"
(() => {
  let S = window.__vemryTr;
  if (!S) {
    S = window.__vemryTr = { n: 0, gen: Math.random().toString(36).slice(2), items: new Map(), pane: null, inner: null, raf: 0, timer: 0 };
    const W = 'max(36%, 300px)';
    const body = getComputedStyle(document.body);
    const html = getComputedStyle(document.documentElement);
    const alpha = (c) => { const m = /rgba?\(([^)]+)\)/.exec(c || ''); return m ? (m[1].split(',')[3] === undefined ? 1 : parseFloat(m[1].split(',')[3])) : 0; };
    const bg = alpha(body.backgroundColor) > 0 ? body.backgroundColor : alpha(html.backgroundColor) > 0 ? html.backgroundColor : '#fff';
    const pane = document.createElement('div');
    pane.setAttribute('data-vemry-tr-out', 'pane');
    // `all:initial` first: the page's own rules for div (a border, a margin, a font) must not reach the pane
    pane.style.cssText = 'all:initial;display:block;position:fixed;top:0;right:0;bottom:0;width:' + W + ';z-index:2147483646;overflow:hidden;box-sizing:border-box;border-left:1px solid rgba(127,127,127,.35);background:' + bg + ';color:' + body.color + ';font-family:' + body.fontFamily + ';font-size:16px;line-height:1.6;';
    const inner = document.createElement('div');
    inner.style.cssText = 'all:initial;display:block;position:relative;height:100%;color:inherit;font:inherit;';
    pane.appendChild(inner);
    // the shimmer of a placeholder and the progress line need a stylesheet: the pane's own, with its own names
    const style = document.createElement('style');
    style.textContent = '@keyframes vemryTrShimmer{from{background-position:200% 0}to{background-position:-200% 0}}';
    pane.appendChild(style);
    const bar = document.createElement('div');
    bar.style.cssText = 'all:initial;display:block;position:absolute;left:0;top:0;height:2px;width:0;background:rgba(31,111,116,.9);transition:width .3s,opacity .6s;z-index:1;';
    pane.appendChild(bar);
    S.bar = bar;
    document.documentElement.appendChild(pane);
    S.progress = (done, total) => {
      const pct = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0;
      S.bar.style.width = pct + '%';
      S.bar.style.opacity = total > 0 && done >= total ? '0' : '1';
    };
    // Picking a block: it stays lit on the page, its entry lit in the pane, until another is picked or it is picked again
    S.selected = null;
    S.select = (id, scroll) => {
      const prev = S.selected == null ? null : S.items.get(S.selected);
      if (prev) {
        prev.el.style.background = prev.saved.bg; prev.el.style.boxShadow = prev.saved.sh; prev.el.style.borderRadius = prev.saved.br;
        prev.out.style.background = ''; prev.out.style.borderLeftColor = 'rgba(127,127,127,.35)';
      }
      if (S.selected === id) { S.selected = null; return; }
      const it = S.items.get(id);
      if (!it) { S.selected = null; return; }
      it.saved = { bg: it.el.style.background, sh: it.el.style.boxShadow, br: it.el.style.borderRadius };
      it.el.style.background = 'rgba(250,204,21,.28)'; it.el.style.boxShadow = '0 0 0 4px rgba(250,204,21,.28)'; it.el.style.borderRadius = '3px';
      it.out.style.background = 'rgba(127,127,127,.10)'; it.out.style.borderLeftColor = 'rgba(31,111,116,.95)';
      S.selected = id;
      if (scroll) {
        const r = it.el.getBoundingClientRect();
        if (r.top < 0 || r.bottom > innerHeight) it.el.scrollIntoView({ block: 'center', behavior: 'smooth' });
      }
    };
    S.onPageClick = (e) => {
      const t = e.target;
      if (!(t instanceof Element) || t.closest('a,button,input,select,textarea,[data-vemry-tr-out]')) return;
      const b = t.closest('[data-vemry-tr]');
      if (b) S.select(Number(b.getAttribute('data-vemry-tr')), false);
    };
    document.addEventListener('click', S.onPageClick, true);
    S.prevMargin = document.documentElement.style.marginRight;
    document.documentElement.style.marginRight = W;
    S.pane = pane; S.inner = inner;
    S.layout = () => {
      S.raf = 0;
      const entries = [...S.items.values()].filter((it) => it.el.isConnected);
      entries.sort((a, b) => a.el.getBoundingClientRect().top - b.el.getBoundingClientRect().top);
      let bottom = -1e9;
      for (const it of entries) {
        const r = it.el.getBoundingClientRect();
        if (r.width === 0 && r.height === 0) { it.out.style.display = 'none'; continue; }
        it.out.style.display = '';
        const top = Math.max(r.top, bottom + 6);
        it.out.style.top = top + 'px';
        bottom = top + it.out.offsetHeight;
      }
      for (const it of S.items.values()) if (!it.el.isConnected) { it.out.remove(); }
    };
    S.schedule = () => { if (!S.raf) S.raf = requestAnimationFrame(S.layout); };
    document.addEventListener('scroll', S.schedule, true);
    window.addEventListener('resize', S.schedule);
    S.timer = setInterval(S.schedule, 1000);
    S.apply = (id, text) => {
      const it = S.items.get(id);
      if (!it) return;
      it.pending = false;
      it.out.textContent = '';
      // a formula travelled as a placeholder; the page's own rendering of it comes back in its place
      const parts = text.split(/⟦m(\d+)⟧/);
      for (let i = 0; i < parts.length; i++) {
        if (i % 2 === 0) { if (parts[i]) it.out.appendChild(document.createTextNode(parts[i])); continue; }
        const m = it.math[Number(parts[i])];
        if (m) {
          const c = m.cloneNode(true);
          c.removeAttribute('id');
          it.out.appendChild(c);
          // a displayed formula wider than the pane is shrunk to fit rather than scrolled
          if (c instanceof HTMLElement && c.scrollWidth > c.clientWidth && c.clientWidth > 0) { c.style.fontSize = Math.floor((c.clientWidth / c.scrollWidth) * 96) + '%'; c.style.overflow = 'hidden'; }
        }
      }
      it.out.style.opacity = '';
      it.out.style.borderLeftStyle = 'solid';
      S.schedule();
    };
    S.clear = () => {
      clearInterval(S.timer);
      document.removeEventListener('scroll', S.schedule, true);
      document.removeEventListener('click', S.onPageClick, true);
      window.removeEventListener('resize', S.schedule);
      if (S.selected != null) S.select(S.selected, false);
      S.pane.remove();
      document.documentElement.style.marginRight = S.prevMargin || '';
      for (const it of S.items.values()) { it.el.removeAttribute('data-vemry-tr'); it.el.style.outline = ''; }
      S.items.clear();
      window.__vemryTr = null;
    };
  }
  // Only what the reader can type into is skipped as editable: a read-only editor (contenteditable="false", how a long-form article is rendered) is body text
  const SKIP = 'script,style,noscript,textarea,input,select,button,code,pre,nav,header,footer,aside,figcaption,caption,cite,time,address,sup,table,[contenteditable=""],[contenteditable="true"],[contenteditable="plaintext-only"],[aria-hidden="true"],[role="navigation"],[role="banner"],[role="contentinfo"],[role="doc-bibliography"],[role="doc-footnote"],.references,.reference,.refs,#references,.bibliography,.citation,.byline,.author,[rel="author"],[itemprop="author"],[data-vemry-tr-out]';
  // A rendered formula (MathML, KaTeX, MathJax): sent as a placeholder, put back as a clone
  const MATH = 'math,.mwe-math-element,.katex,.katex-display,mjx-container,.MathJax,.MathJax_Display,.MathJax_Preview';
  const textOf = (el) => {
    const maths = [...el.querySelectorAll(MATH)].filter((m) => !m.parentElement || !m.parentElement.closest(MATH));
    if (!maths.length) return { text: (el.innerText || '').replace(/\s+/g, ' ').trim(), maths: [] };
    const clone = el.cloneNode(true);
    const cloned = [...clone.querySelectorAll(MATH)].filter((m) => !m.parentElement || !m.parentElement.closest(MATH));
    cloned.forEach((m, i) => m.replaceWith(document.createTextNode(' ⟦m' + i + '⟧ ')));
    for (const hidden of clone.querySelectorAll('[style*="display:none"],[style*="display: none"],[hidden]')) hidden.remove();
    return { text: (clone.textContent || '').replace(/\s+/g, ' ').trim(), maths };
  };
  const BLOCK = new Set(['DIV','P','UL','OL','LI','TABLE','SECTION','ARTICLE','HEADER','FOOTER','ASIDE','NAV','BLOCKQUOTE','PRE','H1','H2','H3','H4','H5','H6','FIGURE','FORM','FIELDSET','DL','DD','DT','HR','TR','TD','TH','MAIN','DETAILS','SUMMARY']);
  const TEXTY = new Set(['P','H1','H2','H3','H4','H5','H6','LI','BLOCKQUOTE','DD','DT','DIV','SPAN','SECTION','ARTICLE','SUMMARY','MAIN']);
  const out = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
  for (let el = walker.currentNode; el; el = walker.nextNode()) {
    if (!TEXTY.has(el.tagName) || el.hasAttribute('data-vemry-tr')) continue;
    if (el.closest(SKIP) || el.closest(MATH)) continue;
    let hasBlockChild = false;
    for (const c of el.children) { if (BLOCK.has(c.tagName)) { hasBlockChild = true; break; } }
    if (hasBlockChild) continue;
    const { text, maths } = textOf(el);
    if (text.length < 20 || !/\p{L}{3}/u.test(text)) continue;
    let covered = false;
    for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) { if (p.hasAttribute('data-vemry-tr')) { covered = true; break; } }
    if (covered) continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) continue;
    const id = S.n++;
    el.setAttribute('data-vemry-tr', String(id));
    const cs = getComputedStyle(el);
    const o = document.createElement('div');
    o.setAttribute('data-vemry-tr-out', String(id));
    o.style.cssText = 'all:initial;display:block;position:absolute;left:0;right:0;box-sizing:border-box;padding:0 14px 0 12px;margin:0;white-space:pre-wrap;overflow-wrap:anywhere;border-left:3px dotted rgba(127,127,127,.35);opacity:.45;cursor:pointer;color:inherit;font-family:' + cs.fontFamily + ';font-size:' + cs.fontSize + ';font-weight:' + cs.fontWeight + ';line-height:' + cs.lineHeight + ';';
    // the placeholder: as many shimmering lines as the block has, so the pane shows the shape of what is coming
    const lh = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.5;
    const lines = Math.max(1, Math.min(4, Math.round(r.height / lh)));
    for (let i = 0; i < lines; i++) {
      const b = document.createElement('div');
      b.style.cssText = 'all:initial;display:block;height:.65em;margin:.4em 0;border-radius:4px;background:linear-gradient(90deg,rgba(127,127,127,.14),rgba(127,127,127,.32),rgba(127,127,127,.14));background-size:200% 100%;animation:vemryTrShimmer 1.4s linear infinite;' + (i === lines - 1 && lines > 1 ? 'width:60%;' : '');
      o.appendChild(b);
    }
    o.addEventListener('click', () => S.select(id, true));
    S.inner.appendChild(o);
    S.items.set(id, { el, out: o, math: maths, pending: true });
    out.push({ id, text });
  }
  S.schedule();
  // the blocks in and near the viewport still waiting: the reader's place goes to the front of the queue
  const visible = [];
  for (const [id, it] of S.items) {
    if (!it.pending || !it.el.isConnected) continue;
    const r = it.el.getBoundingClientRect();
    if (r.bottom > -400 && r.top < innerHeight + 400) visible.push(id);
  }
  return { url: location.href, title: document.title, gen: S.gen, blocks: out, visible };
})()
"#;

/// Move the pane's progress line.
fn progress_js(done: usize, total: usize) -> String {
    format!("(() => {{ const S = window.__vemryTr; if (S) S.progress({done}, {total}); }})()")
}

/// Remove the pane and untag the blocks.
const CLEAR_JS: &str = "(() => { if (window.__vemryTr) window.__vemryTr.clear(); return true; })()";

/// Put a translation into its pane entry.
fn apply_js(id: u32, text: &str) -> String {
    let payload = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    format!("(() => {{ const S = window.__vemryTr; if (S) S.apply({id}, {payload}); }})()")
}

#[derive(Deserialize)]
struct Collected {
    url: String,
    #[serde(default)]
    gen: String,
    #[serde(default)]
    blocks: Vec<CollectedBlock>,
    /// The untranslated blocks in and near the viewport, to be sent first.
    #[serde(default)]
    visible: Vec<u32>,
}

#[derive(Deserialize)]
struct CollectedBlock {
    id: u32,
    text: String,
}

/// One tab being translated.
pub struct PageTranslation {
    label: String,
    url: String,
    gen: String,
    job: Arc<TranslateJob>,
    active: Arc<AtomicBool>,
    done: Arc<AtomicUsize>,
}

#[derive(Default)]
pub struct PageTranslations {
    tabs: Mutex<HashMap<String, Arc<PageTranslation>>>,
    /// A page's translations by `url + target`, kept in memory across closing and
    /// reopening the pane on the same page (the same parking the reading
    /// projection's translation has; never on disk).
    caches: Mutex<Vec<(String, TranslationCache)>>,
}

const CACHE_CAP: usize = 32;

impl PageTranslations {
    fn park(&self, url: &str, target: &str, cache: Option<TranslationCache>) {
        let Some(cache) = cache else { return };
        if cache.is_empty() {
            return;
        }
        let key = format!("{url}\u{0}{target}");
        let mut caches = self.caches.lock().unwrap();
        caches.retain(|(k, _)| *k != key);
        caches.push((key, cache));
        while caches.len() > CACHE_CAP {
            caches.remove(0);
        }
    }

    fn take(&self, url: &str, target: &str) -> Option<TranslationCache> {
        let key = format!("{url}\u{0}{target}");
        let mut caches = self.caches.lock().unwrap();
        let at = caches.iter().position(|(k, _)| *k == key)?;
        Some(caches.remove(at).1)
    }

    /// Stop a tab's translation (the job, the poll) without touching the page;
    /// the caller clears the page when it still exists.
    fn stop(&self, label: &str) -> Option<Arc<PageTranslation>> {
        let t = self.tabs.lock().unwrap().remove(label)?;
        t.active.store(false, Ordering::Relaxed);
        t.job.cancel();
        self.park(&t.url, &t.job.target, t.job.take_cache());
        Some(t)
    }
}

fn segments_of(blocks: &[CollectedBlock]) -> Vec<Segment> {
    blocks
        .iter()
        .map(|b| Segment {
            id: b.id,
            kind: SegKind::Para,
            prefix: String::new(),
            text: b.text.clone(),
            translate: true,
            private: false,
            span: 0..0,
            page: None,
        })
        .collect()
}

fn progress(app: &AppHandle, t: &PageTranslation) {
    emit(
        app,
        &BrowserEvent::Translate {
            label: t.label.clone(),
            active: t.active.load(Ordering::Relaxed),
            done: t.done.load(Ordering::Relaxed),
            total: t.job.len(),
        },
    );
}

/// Start translating a collected document and poll it for more.
fn start(app: &AppHandle, state: &State<'_, AppState>, label: String, webview: tauri::Webview, collected: Collected) -> Result<Arc<PageTranslation>, String> {
    let s = settings::load(app);
    let mut provider = s.provider_config();
    if provider.api_key.is_empty() {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.noApiKey"));
    }
    let m = s.translate_model.trim();
    if !m.is_empty() {
        provider.model = m.to_string();
    }
    let target = crate::translate::target_of(&s);
    let cache = state.browser_translate.take(&collected.url, &target).unwrap_or_else(|| TranslationCache::in_memory(&provider.model));
    let active = Arc::new(AtomicBool::new(true));
    let done = Arc::new(AtomicUsize::new(0));
    let job_id = format!("page-{}-{}", label, collected.gen);
    let doc = TranslateDoc { key: collected.url.clone(), title: collected.url.clone(), segments: segments_of(&collected.blocks) };
    // The events go into the page, not to a view: a finished segment lands in the
    // pane, and the chrome hears the count through browser://event
    let ev_webview = webview.clone();
    let ev_app = app.clone();
    let ev_label = label.clone();
    let ev_active = Arc::clone(&active);
    let ev_done = Arc::clone(&done);
    let counted: Arc<Mutex<std::collections::HashSet<u32>>> = Arc::default();
    let total: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(collected.blocks.len()));
    let ev_total = Arc::clone(&total);
    let on_event: Box<dyn Fn(TranslateEvent) + Send + Sync> = Box::new(move |event| {
        if !ev_active.load(Ordering::Relaxed) {
            return;
        }
        match event {
            TranslateEvent::Segment { id, text, done: true } => {
                let _ = ev_webview.eval(apply_js(id, &text));
                if counted.lock().unwrap().insert(id) {
                    ev_done.fetch_add(1, Ordering::Relaxed);
                }
            }
            TranslateEvent::Segment { .. } => return,
            TranslateEvent::Failed { .. } | TranslateEvent::Idle => {}
        }
        let (done, total) = (ev_done.load(Ordering::Relaxed), ev_total.load(Ordering::Relaxed));
        let _ = ev_webview.eval(progress_js(done, total));
        emit(&ev_app, &BrowserEvent::Translate { label: ev_label.clone(), active: true, done, total });
    });
    let effort = s.reasoning_active().then(|| s.reasoning_effort.clone());
    let job = TranslateJob::new(job_id, doc, target, s.translate_addendum.clone(), provider, effort, Arc::new(HttpTransport::new()), Some(cache), on_event);
    // Cache hits land in the pane at once
    for (id, text) in job.translations() {
        let _ = webview.eval(apply_js(id, &text));
        done.fetch_add(1, Ordering::Relaxed);
    }
    // The viewport's blocks first, then the rest in document order — one request, so the
    // first batches to leave are the reader's
    let mut order: Vec<u32> = collected.visible.clone();
    let ids: Vec<u32> = job.segments().iter().map(|seg| seg.id).collect();
    order.extend(ids.iter().copied().filter(|id| !collected.visible.contains(id)));
    job.request(&order, false);
    let _ = webview.eval(progress_js(done.load(Ordering::Relaxed), ids.len()));
    let t = Arc::new(PageTranslation { label: label.clone(), url: collected.url, gen: collected.gen, job, active, done });
    state.browser_translate.tabs.lock().unwrap().insert(label.clone(), Arc::clone(&t));
    progress(app, &t);
    // The poll: new blocks join the job; a new document starts over
    let poll_app = app.clone();
    let poll_t = Arc::clone(&t);
    let poll_total = total;
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
            if !poll_t.active.load(Ordering::Relaxed) {
                break;
            }
            let Ok(data) = eval_json(&webview, COLLECT_JS).await else { continue };
            // Stopped while the eval was out: the collect just re-installed the pane the
            // clear removed, so it is cleared again and nothing is restarted
            if !poll_t.active.load(Ordering::Relaxed) {
                let _ = webview.eval(CLEAR_JS);
                break;
            }
            let Ok(c) = serde_json::from_value::<Collected>(data) else { continue };
            if c.gen != poll_t.gen {
                // another document: the pane went with the old one, and this one has a fresh pane
                let state: State<'_, AppState> = poll_app.state();
                state.browser_translate.stop(&poll_t.label);
                if let Err(e) = start(&poll_app, &state, poll_t.label.clone(), webview.clone(), c) {
                    let _ = webview.eval(CLEAR_JS);
                    emit(&poll_app, &BrowserEvent::Translate { label: poll_t.label.clone(), active: false, done: 0, total: 0 });
                    let _ = e;
                }
                break;
            }
            if !c.visible.is_empty() {
                poll_t.job.request(&c.visible, false);
            }
            if c.blocks.is_empty() {
                continue;
            }
            poll_t.job.extend(segments_of(&c.blocks));
            poll_total.store(poll_t.job.len(), Ordering::Relaxed);
            // blocks that came from the cache landed through `extend` without an event
            for (id, text) in poll_t.job.translations() {
                if c.blocks.iter().any(|b| b.id == id) {
                    let _ = webview.eval(apply_js(id, &text));
                }
            }
            progress(&poll_app, &poll_t);
        }
    });
    Ok(t)
}

/// Open the translation pane in the tab, or close it when it is open. Returns
/// whether it is open afterwards.
#[tauri::command]
pub async fn browser_translate_toggle(
    app: AppHandle,
    state: State<'_, AppState>,
    label: Option<String>,
) -> Result<bool, String> {
    let webview = target_webview(&app, &state, label)?;
    let label = webview.label().to_string();
    if let Some(t) = state.browser_translate.stop(&label) {
        let _ = webview.eval(CLEAR_JS);
        emit(&app, &BrowserEvent::Translate { label: t.label.clone(), active: false, done: 0, total: 0 });
        return Ok(false);
    }
    let data = eval_json(&webview, COLLECT_JS).await?;
    let collected: Collected = serde_json::from_value(data).map_err(|e| e.to_string())?;
    if collected.blocks.is_empty() {
        let _ = webview.eval(CLEAR_JS);
        return Err(crate::i18n::t(crate::i18n::current(), "translate.nothingToTranslate"));
    }
    if let Err(e) = start(&app, &state, label, webview.clone(), collected) {
        let _ = webview.eval(CLEAR_JS);
        return Err(e);
    }
    Ok(true)
}

/// A tab closed: its translation stops with it (nothing to clear — the page is gone).
pub fn tab_closed(state: &AppState, label: &str) {
    state.browser_translate.stop(label);
}
