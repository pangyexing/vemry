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
//! it scrolls). Moving to another page — a navigation, a reload, a single-page
//! app's route change — ends the translation: the pane is removed and the
//! button goes back to off, and the next page is translated only when asked.
//! Pressing the button again removes the pane and stops.
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
/// different value on the next poll means a different document. `TARGET` (the
/// translation's target language) is filled in by `collect_js`: a block already
/// in that language is not collected.
const COLLECT_JS: &str = r#"
(() => {
  const TARGET = "__TARGET__";
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
    // The page's real colours are the ones behind its text, not the body's (an app
    // shell often paints a dark body under a light content column): taken from the
    // first block's ancestors once there is one
    S.recolor = (el) => {
      if (S.recolored) return;
      S.recolored = true;
      for (let e = el; e; e = e.parentElement) {
        const c = getComputedStyle(e).backgroundColor;
        if (alpha(c) > 0.5) { pane.style.background = c; break; }
      }
      pane.style.color = getComputedStyle(el).color;
    };
    const GAP = 6;
    S.layout = () => {
      S.raf = 0;
      const rows = [];
      const paneLeft = S.pane.getBoundingClientRect().left;
      for (const it of S.items.values()) {
        if (!it.el.isConnected) { it.out.remove(); continue; }
        const r = it.el.getBoundingClientRect();
        if ((r.width === 0 && r.height === 0) || r.left + r.width / 2 >= paneLeft) { it.out.style.display = 'none'; continue; }
        it.out.style.display = '';
        rows.push({ it, r, t: r.top, hb: r.height, h: it.out.offsetHeight, y: 0 });
      }
      rows.sort((a, b) => a.t - b.t);
      // The pane's width is settled once from the page as it reflowed: an entry gets about
      // three quarters of its block's width, which is what a CJK translation of an English
      // paragraph needs to end near where the paragraph ends
      if (!S.sized && rows.length >= 3) {
        S.sized = true;
        const widths = rows.map((x) => x.r.width).filter((w) => w > 200).sort((a, b) => a - b);
        if (widths.length) {
          const median = widths[Math.floor(widths.length / 2)];
          const want = Math.round(Math.min(innerWidth * 0.48, Math.max(300, median * 0.75 + 26)));
          if (Math.abs(want - S.pane.clientWidth) > 24) {
            S.pane.style.width = want + 'px';
            document.documentElement.style.marginRight = want + 'px';
            S.schedule();
            return;
          }
        }
      }
      // forward: each entry at its block, or under the one above
      let bottom = -1e9;
      for (const x of rows) { x.y = Math.max(x.t, bottom + GAP); bottom = x.y + x.h; }
      // backward, twice: an entry pushed below its block lifts the one above it into the slack
      // above — by at most three tenths of that block's height, so it still starts by its block
      for (let round = 0; round < 2; round++) {
        for (let i = rows.length - 1; i >= 1; i--) {
          const cur = rows[i], prev = rows[i - 1];
          const over = cur.y - cur.t;
          if (over <= 0) continue;
          const floor = i >= 2 ? rows[i - 2].y + rows[i - 2].h + GAP : -1e9;
          const minPrev = Math.max(prev.t - 0.3 * prev.hb, floor);
          const up = Math.min(over, prev.y - minPrev);
          if (up > 0) { prev.y -= up; cur.y = Math.max(cur.t, prev.y + prev.h + GAP); }
        }
      }
      for (const x of rows) {
        x.it.out.style.top = x.y + 'px';
        // a placeholder shimmers only where a translation is on its way (in and near the viewport)
        if (x.it.pending) { const near = x.r.bottom > -innerHeight && x.r.top < innerHeight * 2.5; for (const b of x.it.out.children) if (b.style.animationName) b.style.animationPlayState = near ? 'running' : 'paused'; }
      }
    };
    S.schedule = () => { if (!S.raf) S.raf = requestAnimationFrame(S.layout); };
    document.addEventListener('scroll', S.schedule, true);
    window.addEventListener('resize', S.schedule);
    S.timer = setInterval(S.schedule, 1000);
    S.apply = (id, text, done) => {
      const it = S.items.get(id);
      if (!it) return;
      if (done) { it.pending = false; it.out.style.opacity = ''; it.out.style.borderLeftStyle = 'solid'; it.out.style.minHeight = ''; }
      else { it.out.style.opacity = '.7'; }
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
      S.schedule();
    };
    // The background stopped at its budget before block `at`: that block's entry says so and offers to go on
    S.skeleton = (o, lines) => {
      o.textContent = '';
      for (let i = 0; i < lines; i++) {
        const b = document.createElement('div');
        b.style.cssText = 'all:initial;display:block;height:.65em;margin:.4em 0;border-radius:4px;background:linear-gradient(90deg,rgba(127,127,127,.14),rgba(127,127,127,.32),rgba(127,127,127,.14));background-size:200% 100%;animation:vemryTrShimmer 1.4s linear infinite;' + (i === lines - 1 && lines > 1 ? 'width:60%;' : '');
        o.appendChild(b);
      }
    };
    S.wants = null;
    S.pausedAt = null;
    S.pause = (at, labels) => {
      const it = S.items.get(at);
      if (!it || !it.pending) return;
      S.pausedAt = at;
      it.out.textContent = '';
      it.out.style.opacity = '';
      const note = document.createElement('span');
      note.style.cssText = 'all:initial;font:inherit;color:inherit;opacity:.6;font-size:.85em;';
      note.textContent = labels.here + ' · ';
      it.out.appendChild(note);
      for (const [key, label] of [['more', labels.more], ['all', labels.all]]) {
        const b = document.createElement('span');
        b.style.cssText = 'all:initial;font:inherit;color:inherit;font-size:.85em;cursor:pointer;text-decoration:underline;margin-right:.8em;';
        b.textContent = label;
        b.addEventListener('click', (e) => { e.stopPropagation(); S.wants = key; S.resume(); });
        it.out.appendChild(b);
      }
      S.schedule();
    };
    S.resume = () => {
      const it = S.pausedAt == null ? null : S.items.get(S.pausedAt);
      S.pausedAt = null;
      if (it && it.pending) { S.skeleton(it.out, it.lines); it.out.style.opacity = '.45'; }
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
  // Already in the target language: a CJK target goes by the text itself (half the
  // letters or more in that script); any target goes by a lang attribute on the block
  // or an ancestor below the document root (the root's lang is the interface's, and
  // says nothing about a quoted post)
  const primary = TARGET.split('-')[0].toLowerCase();
  const inTarget = (el, text) => {
    const tagged = el.closest('[lang]');
    if (tagged && tagged !== document.documentElement && tagged !== document.body) {
      const l = (tagged.getAttribute('lang') || '').split('-')[0].toLowerCase();
      if (l) return l === primary;
    }
    if (primary === 'zh' || primary === 'ja' || primary === 'ko') {
      let cjk = 0, latin = 0;
      for (const ch of text) {
        if (/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u.test(ch)) cjk++;
        else if (/\p{Script=Latin}/u.test(ch)) latin++;
      }
      return cjk > 0 && cjk * 5 >= (cjk * 5 + latin) * 0.5;
    }
    return false;
  };
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
    if (inTarget(el, text)) continue;
    let covered = false;
    for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) { if (p.hasAttribute('data-vemry-tr')) { covered = true; break; } }
    if (covered) continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) continue;
    // What sits under the pane — a page's right-hand column, its promoted items — is not
    // what the reader is reading, and its translation would float beside nothing
    if (r.left + r.width / 2 >= S.pane.getBoundingClientRect().left) continue;
    const id = S.n++;
    el.setAttribute('data-vemry-tr', String(id));
    S.recolor(el);
    const cs = getComputedStyle(el);
    const o = document.createElement('div');
    o.setAttribute('data-vemry-tr-out', String(id));
    // A touch smaller and tighter than the block: a CJK translation of an English paragraph
    // then ends about where the paragraph does, and the rows below it stay aligned
    const fs = Math.max(12, parseFloat(cs.fontSize) * 0.94);
    o.style.cssText = 'all:initial;display:block;position:absolute;left:0;right:0;box-sizing:border-box;padding:0 12px 0 10px;margin:0;white-space:pre-wrap;overflow-wrap:anywhere;border-left:3px dotted rgba(127,127,127,.35);opacity:.45;cursor:pointer;color:inherit;font-family:' + cs.fontFamily + ';font-size:' + fs + 'px;font-weight:' + cs.fontWeight + ';line-height:1.5;';
    // the placeholder: the height the translation will most likely have (the block's), so a
    // translation landing above the reader does not push the rows below it; and as many
    // shimmering lines as the block has, so the pane shows the shape of what is coming
    const lh = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.5;
    const lines = Math.max(1, Math.min(4, Math.round(r.height / lh)));
    o.style.minHeight = Math.round(r.height) + 'px';
    S.skeleton(o, lines);
    o.addEventListener('click', () => { if (!o.querySelector('[data-vemry-tr-out]') ) S.select(id, true); });
    S.inner.appendChild(o);
    S.items.set(id, { el, out: o, math: maths, pending: true, lines });
    out.push({ id, text });
  }
  S.schedule();
  // the blocks in and near the viewport still waiting (a screen behind, two and a half
  // ahead — a page scrolls faster than a note): the reader's place goes to the front of the queue
  const visible = [];
  for (const [id, it] of S.items) {
    if (!it.pending || !it.el.isConnected) continue;
    const r = it.el.getBoundingClientRect();
    if (r.bottom > -innerHeight && r.top < innerHeight * 2.5) visible.push(id);
  }
  const wants = S.wants;
  S.wants = null;
  return { url: location.href, title: document.title, gen: S.gen, blocks: out, visible, wants };
})()
"#;

/// The pause marker's copy, in the user's language (the page shows it, so it comes
/// from the catalog like every other string the user reads).
fn pause_js(at: u32) -> String {
    let lang = crate::i18n::current();
    let labels = serde_json::json!({
        "here": crate::i18n::t(lang, "translate.pausedHere"),
        "more": crate::i18n::t(lang, "translate.continue"),
        "all": crate::i18n::t(lang, "translate.translateAll"),
    });
    format!("(() => {{ const S = window.__vemryTr; if (S) S.pause({at}, {labels}); }})()")
}

/// Move the pane's progress line.
fn progress_js(done: usize, total: usize) -> String {
    format!("(() => {{ const S = window.__vemryTr; if (S) S.progress({done}, {total}); }})()")
}

/// The collect script for a target language.
fn collect_js(target: &str) -> String {
    COLLECT_JS.replace("__TARGET__", &target.replace(['"', '\\'], ""))
}

/// What page the tab is on, without touching it: the pane's document token (null
/// when this document has no pane — the tab navigated or reloaded) and the URL.
const PROBE_JS: &str = "(() => { const S = window.__vemryTr; return { gen: S ? S.gen : null, url: location.href }; })()";

/// The page's identity for the poll: the URL without its fragment (an anchor jump
/// stays on the page; a single-page app's route change does not).
fn page_key(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

/// Remove the pane and untag the blocks.
const CLEAR_JS: &str = "(() => { if (window.__vemryTr) window.__vemryTr.clear(); return true; })()";

/// Put a translation into its pane entry — the text so far while it streams, final when done.
fn apply_js(id: u32, text: &str, done: bool) -> String {
    let payload = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    format!("(() => {{ const S = window.__vemryTr; if (S) S.apply({id}, {payload}, {done}); }})()")
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
    /// A click on the pause marker: "more" or "all".
    #[serde(default)]
    wants: Option<String>,
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
    caches: crate::translate::ParkedCaches,
}

impl PageTranslations {
    fn park(&self, url: &str, target: &str, cache: Option<TranslationCache>) {
        if let Some(cache) = cache {
            self.caches.park(url, target, cache);
        }
    }

    fn take(&self, url: &str, target: &str) -> Option<TranslationCache> {
        self.caches.take(url, target)
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
    let (p, c) = t.job.usage();
    emit(
        app,
        &BrowserEvent::Translate {
            label: t.label.clone(),
            active: t.active.load(Ordering::Relaxed),
            done: t.done.load(Ordering::Relaxed),
            total: t.job.len(),
            tokens: p + c,
        },
    );
}

/// Start translating a collected document and poll it for more.
fn start(app: &AppHandle, state: &State<'_, AppState>, label: String, webview: tauri::Webview, collected: Collected) -> Result<Arc<PageTranslation>, String> {
    let s = settings::load(app);
    let provider = crate::translate::provider_of(&s)?;
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
    let tokens: Arc<AtomicUsize> = Arc::default();
    let ev_tokens = Arc::clone(&tokens);
    let on_event: Box<dyn Fn(TranslateEvent) + Send + Sync> = Box::new(move |event| {
        if !ev_active.load(Ordering::Relaxed) {
            return;
        }
        match event {
            TranslateEvent::Usage { prompt, completion } => {
                ev_tokens.store((prompt + completion) as usize, Ordering::Relaxed);
            }
            TranslateEvent::Segment { id, text, done: true } => {
                let _ = ev_webview.eval(apply_js(id, &text, true));
                if counted.lock().unwrap().insert(id) {
                    ev_done.fetch_add(1, Ordering::Relaxed);
                }
            }
            // the text so far, streamed into the entry as it arrives
            TranslateEvent::Segment { id, text, done: false } => {
                let _ = ev_webview.eval(apply_js(id, &text, false));
                return;
            }
            TranslateEvent::Paused { at } => {
                let _ = ev_webview.eval(pause_js(at));
                return;
            }
            TranslateEvent::Failed { .. } | TranslateEvent::Idle => {}
        }
        let (done, total) = (ev_done.load(Ordering::Relaxed), ev_total.load(Ordering::Relaxed));
        let _ = ev_webview.eval(progress_js(done, total));
        emit(&ev_app, &BrowserEvent::Translate { label: ev_label.clone(), active: true, done, total, tokens: ev_tokens.load(Ordering::Relaxed) as u64 });
    });
    let effort = s.reasoning_active().then(|| s.reasoning_effort.clone());
    let job = TranslateJob::new(job_id, doc, target, s.translate_addendum.clone(), provider, effort, Arc::new(HttpTransport::new()), Some(cache), on_event);
    // Cache hits land in the pane at once
    for (id, text) in job.translations() {
        let _ = webview.eval(apply_js(id, &text, true));
        done.fetch_add(1, Ordering::Relaxed);
    }
    // The viewport's blocks are what the reader wants — asked for first, so the first
    // (short) batch is theirs; the document follows as background work
    let ids: Vec<u32> = job.segments().iter().map(|seg| seg.id).collect();
    job.request(&collected.visible, false);
    job.queue(&ids);
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
            // First a look without touching the page: another page (a navigation, a
            // reload, a route change — the document token is gone or the URL's path
            // moved) ends the translation rather than carrying it over
            let Ok(probe) = eval_json(&webview, PROBE_JS).await else { continue };
            if !poll_t.active.load(Ordering::Relaxed) {
                break;
            }
            let same_doc = probe.get("gen").and_then(|g| g.as_str()) == Some(poll_t.gen.as_str());
            let same_page = probe.get("url").and_then(|u| u.as_str()).is_some_and(|u| page_key(u) == page_key(&poll_t.url));
            if !same_doc || !same_page {
                let state: State<'_, AppState> = poll_app.state();
                state.browser_translate.stop(&poll_t.label);
                let _ = webview.eval(CLEAR_JS);
                emit(&poll_app, &BrowserEvent::Translate { label: poll_t.label.clone(), active: false, done: 0, total: 0, tokens: 0 });
                break;
            }
            let Ok(data) = eval_json(&webview, &collect_js(&poll_t.job.target)).await else { continue };
            // Stopped while the eval was out: the collect just re-installed the pane the
            // clear removed, so it is cleared again and nothing is restarted
            if !poll_t.active.load(Ordering::Relaxed) {
                let _ = webview.eval(CLEAR_JS);
                break;
            }
            let Ok(c) = serde_json::from_value::<Collected>(data) else { continue };
            match c.wants.as_deref() {
                Some("all") => poll_t.job.continue_background(None),
                Some("more") => poll_t.job.continue_background(Some(vemry_agent::translate::BACKGROUND_BUDGET_CHARS)),
                _ => {}
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
                    let _ = webview.eval(apply_js(id, &text, true));
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
        emit(&app, &BrowserEvent::Translate { label: t.label.clone(), active: false, done: 0, total: 0, tokens: 0 });
        return Ok(false);
    }
    let target = crate::translate::target_of(&settings::load(&app));
    let data = eval_json(&webview, &collect_js(&target)).await?;
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
