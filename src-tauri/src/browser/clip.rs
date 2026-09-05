//! The clip commands: fetch from a browser window (a selection, the whole page, a
//! link), compose through vemry-clip, and land on disk through
//! `Vault::write_note` + `commands::broadcast` — which makes the frontend refresh
//! and the clips/ workflow trigger free.
//!
//! Fetching is a one-way pull driven by a user command with an extraction script
//! fixed by the application: Rust evals into the browser webview, and a page script
//! has no way to write into the vault (the security boundary is in ARCHITECTURE).

use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};
use vemry_clip::{clip_rel_path, compose_clip_note, extract_page, quote_block, ClipMeta, ExtractedPage};

use super::window::target_webview;
use crate::commands::{broadcast, with_vault, with_vault_mut};
use crate::AppState;

/// The normalisation JS, shared by the whole-page and selection fetch paths. It
/// defines:
/// - `fixImgs(pairs)`: lazy-loading image repair. The real URL often lives in an
///   attribute like data-src or data-original while src is only a placeholder — an
///   inline data: thumbnail, or a 1px http placeholder named blank or loading. An
///   image that already rendered prefers the live DOM's currentSrc (srcset
///   resolved, and the final address after a lazy-loading library swapped it in);
///   a cloned node has no currentSrc or naturalWidth runtime state, so pairs is a
///   table of [cloned img, live img|null] whose pairing rule is the caller's to
///   choose (document order for a whole page, the src attribute for a selection).
///   The fallback is attribute recovery — the same idea as Readability's
///   fixLazyImages, but it has to happen here: the extraction side sees only
///   outerHTML, and once a placeholder src reaches the Markdown it is beyond
///   saving.
/// - `fixMath(root)`: math normalisation, turning what KaTeX/MathJax/MathML
///   rendered back into LaTeX source wrapped in an `<x-vemry-math>` sentinel
///   (which vemry-clip converts to `$…$`). The sentinel's content is squashed onto
///   one line, since LaTeX needs no line breaks.
/// - `fixCode(root, pairs)`: code-block normalisation, restoring plain text
///   wrapped in an `<x-vemry-code>` sentinel (which vemry-clip converts to a fenced
///   block). A rendered code viewer (a lake card, a CodeMirror and their like) has
///   no newline text node between its lines, so serialising outerHTML splices the
///   code onto one line, and a shell comment landing at the start of a line becomes
///   a phantom heading.
///   Two steps: a lake card's original code, newlines and all, is hidden in the
///   URL-encoded JSON of its value attribute and is decoded directly (the highest
///   fidelity, unaffected by render timing or the virtualisation of a long block);
///   everything else is recognised as a code container on the live DOM by its
///   computed style, has innerText restore the newlines from the CSS rendering box,
///   and is written back to the corresponding node on the clone side (pairs match
///   by document order, the same idea as fixImgs). What cannot be recognised keeps
///   htmd's default path — never worse than not normalising at all.
///
/// All three functions modify the cloned node they were given and nothing else —
/// clipping from a visible browser window must not touch the page the user is
/// looking at, and working on a copy is a hard constraint.
const NORMALIZE_JS: &str = r#"
  const squash = (s) => (s || '').replace(/\s+/g, ' ').trim();
  const put = (el, tex, display) => {
    tex = squash(tex);
    if (!tex) return;
    const m = document.createElement('x-vemry-math');
    if (display) m.setAttribute('display', 'block');
    m.textContent = tex;
    el.replaceWith(m);
  };
  const annTex = (el) => {
    const a = el.querySelector('annotation[encoding="application/x-tex"]');
    return a ? a.textContent : '';
  };
  const URLISH = /^(https?:)?\/\//;
  const IMGEXT = /\.(avif|jpe?g|png|webp|gif|svg)(\?|#|$)/i;
  const PLACEHOLDER = /(blank|spacer|placeholder|loading|pixel|transparent|1x1|grey|gray)[^\/]*\.(gif|png|jpe?g)(\?|#|$)/i;
  const fixImgs = (pairs) => {
    for (const pair of pairs) {
      const img = pair[0], live = pair[1];
      const src = img.getAttribute('src') || '';
      const cs = (live && live.currentSrc) || '';
      if (URLISH.test(cs) && live.naturalWidth > 2 && !PLACEHOLDER.test(cs)) {
        if (cs !== src) img.setAttribute('src', cs);
        continue;
      }
      // A 1px image that did load, and one named like a placeholder, are treated as placeholders and go to attribute recovery
      const tiny = live && live.complete && live.naturalWidth > 0 &&
        live.naturalWidth <= 2 && live.naturalHeight <= 2;
      if (src && !src.startsWith('data:') && !tiny && !PLACEHOLDER.test(src)) continue;
      let best = '';
      for (const a of img.attributes) {
        const v = (a.value || '').trim();
        if (a.name === 'src' || a.name === 'srcset' || !v) continue;
        if (URLISH.test(v) && IMGEXT.test(v)) { best = v; break; }
        if (!best && /src|original|lazy|echo|url/.test(a.name) && URLISH.test(v) && !v.includes(' ')) best = v;
      }
      if (!best) {
        const ss = img.getAttribute('srcset') || img.getAttribute('data-srcset') || '';
        const first = ss.split(',')[0].trim().split(/\s+/)[0];
        if (URLISH.test(first)) best = first;
      }
      if (best) img.setAttribute('src', best);
    }
  };
  // Code container candidates: pre plus the class-name signatures of common viewers
  // and highlighters. A match only gets it shortlisted; whether it is converted as
  // code still has to pass fixCode's computed-style threshold
  const CODE_SEL = 'pre, [class*="CodeMirror"], [class*="cm-editor"], [class*="codeblock"], [class*="highlight"], [class*="language-"]';
  const codeLang = (el) => {
    const LANG = /(?:language|lang)-([A-Za-z0-9_+.-]+)/;
    let m = LANG.exec(el.getAttribute('class') || '');
    if (!m) {
      const sub = el.querySelector('[class*="language-"], [class*="lang-"]');
      if (sub) m = LANG.exec(sub.getAttribute('class') || '');
    }
    return (m && m[1]) || el.getAttribute('data-language') || el.getAttribute('data-lang') || '';
  };
  const putCode = (el, code, lang) => {
    // CodeMirror pads its indentation with nbsp — semantically that is an ordinary space in code
    code = (code || '').replace(/\u00a0/g, ' ');
    if (!code.trim()) return false;
    const c = document.createElement('x-vemry-code');
    if (lang) c.setAttribute('lang', String(lang));
    c.textContent = code;
    el.replaceWith(c);
    return true;
  };
  // The dedup key only needs the opening stretch: slice first, squash after, rather than squashing a whole long block for a 4000-character key
  const codeKey = (s) => squash((s || '').slice(0, 8000)).slice(0, 4000);
  // Decoding a lake card directly: the value attribute is URL-encoded JSON
  // {mode, code}. Returns the key set of the decoded code, so both fetch paths can
  // deduplicate a viewer that "keeps the card and hangs a rendered node beside it"
  const fixCodeCards = (root) => {
    const seen = new Set();
    for (const card of root.querySelectorAll('card[name="codeblock"]')) {
      try {
        const j = JSON.parse(decodeURIComponent((card.getAttribute('value') || '').replace(/^data:/, '')));
        if (typeof j.code === 'string' && putCode(card, j.code, j.mode || '')) seen.add(codeKey(j.code));
      } catch (e) {}
    }
    return seen;
  };
  // A container's code text: the content subregion first (avoiding CodeMirror's
  // gutter and line-number column); a table-style highlighter puts one td.code per
  // line and all of them have to be collected — taking only the first would cut the
  // code down to one line. innerText restores the newlines from the CSS rendering
  // box. A known limitation: while a long CodeMirror block is virtualised the lines
  // outside the viewport are not in the DOM and innerText cannot reach them (the
  // lake card path is unaffected)
  const innerCode = (el) => {
    const grab = (n) => (n.innerText != null ? n.innerText : n.textContent);
    const sub = el.querySelector('.CodeMirror-code, .cm-content');
    if (sub) return grab(sub);
    const tds = el.querySelectorAll('td.code');
    if (tds.length) return Array.from(tds, grab).join('\n');
    return grab(el);
  };
  const fixCode = (root, pairs) => {
    const seen = fixCodeCards(root);
    // The general case: recognise a code container by the live DOM's computed style, taking only the outermost
    const monoish = (cs) => /mono|menlo|consolas|courier|code/i.test(cs.fontFamily || '');
    // The candidates come from querySelectorAll (document order): an ancestor always
    // precedes its descendants, and once we leave the subtree of an accepted
    // container we never return to it — so popping while the stack top does not
    // contain the current candidate is enough, and O(n²) is avoided
    const accepted = [];
    for (const pair of pairs) {
      const clone = pair[0], live = pair[1];
      if (!live) continue;
      while (accepted.length && !accepted[accepted.length - 1].contains(live)) accepted.pop();
      if (accepted.length) continue;
      // The threshold: a pre qualifies by itself; anything else needs a monospace
      // font plus either a pre-family white-space or a code class name.
      // getComputedStyle forces a style resolution, so a pre that is taken outright
      // should not pay for one
      let ok = live.tagName === 'PRE';
      if (!ok) {
        const cs = getComputedStyle(live);
        ok = monoish(cs) &&
          (/^pre/.test(cs.whiteSpace || '') || /CodeMirror|cm-editor|codeblock|highlight|language-/i.test(live.getAttribute('class') || ''));
      }
      if (!ok) continue;
      accepted.push(live);
      // The card decoding has already replaced this subtree (the viewer hangs inside the card)
      if (!root.contains(clone)) continue;
      const text = innerCode(live);
      // A viewer beside a card: deduplicated by identical code, keeping only the card's decoding
      if (seen.size && seen.has(codeKey(text))) { clone.remove(); continue; }
      putCode(clone, text, codeLang(live));
    }
  };
  const fixMath = (root) => {
    // KaTeX: the TeX source is in the annotation; display containers come first, and the .katex that remain are inline
    for (const el of root.querySelectorAll('.katex-display')) put(el, annTex(el), true);
    for (const el of root.querySelectorAll('.katex')) put(el, annTex(el), false);
    // MathJax v2: the TeX source is in <script type="math/tex">, and the rendered/preview nodes are removed
    for (const el of root.querySelectorAll('script[type^="math/tex"]')) {
      put(el, el.textContent, (el.getAttribute('type') || '').includes('mode=display'));
    }
    for (const el of root.querySelectorAll('.MathJax, [class*="MathJax_"]')) el.remove();
    // MathJax v3: the DOM holds no TeX source, so fall back to the assistive MathML's alttext or text
    for (const el of root.querySelectorAll('mjx-container')) {
      const math = el.querySelector('math');
      put(el, (math && math.getAttribute('alttext')) || (math ? math.textContent : ''),
          el.getAttribute('display') === 'true');
    }
    // MediaWiki (Wikipedia): the whole .mwe-math-element counts as one unit, and the
    // TeX is taken from the annotation → <math alttext> → the fallback image's alt in
    // turn, so MathML and the image do not both produce output
    for (const el of root.querySelectorAll('.mwe-math-element')) {
      const math = el.querySelector('math');
      const img = el.querySelector('img[alt]');
      const tex = annTex(el) || (math && math.getAttribute('alttext')) || (img && img.getAttribute('alt')) || '';
      const display = !!((math && math.getAttribute('display') === 'block') ||
        el.querySelector('[class*="display"]') || (el.className || '').includes('display'));
      put(el, tex, display);
    }
    // The bare MathML fallback
    for (const el of root.querySelectorAll('math')) {
      put(el, annTex(el) || el.getAttribute('alttext') || el.textContent,
          el.getAttribute('display') === 'block');
    }
  };
"#;

/// The selection cache injected into a browser window at document-start: a login
/// popup or a page script often clears the text the user just selected, and the clip
/// command arriving afterwards can only report "nothing selected". This keeps a Range
/// clone of the most recent non-empty selection, and the fetching side falls back to
/// it when the live selection is empty — a hard navigation invalidates it naturally
/// with the document's lifetime; a soft SPA navigation (pushState keeps the
/// document) is invalidated by comparing the stored href against the current
/// location, or the previous article's cache would be clipped out under the new
/// page's url and title. It only ever holds something the user really selected.
/// A page script could in principle override this function, but a page can already
/// manipulate the Selection API itself, so this is no new trust surface; clipping
/// remains a one-way pull driven by an explicit user action.
pub(crate) const SELECTION_CACHE_JS: &str = r#"
(() => {
  let saved = null;
  document.addEventListener('selectionchange', () => {
    const s = getSelection();
    if (!s || !s.rangeCount || s.isCollapsed) return;
    // selectionchange fires continuously while dragging out a selection: each time
    // this only does an O(number of ranges) clone, and the O(selected text) text
    // extraction is deferred until the fetching side really falls back to the cache
    const ranges = [];
    for (let i = 0; i < s.rangeCount; i++) ranges.push(s.getRangeAt(i).cloneRange());
    saved = { ranges, href: location.href };
  });
  window.__vemrySavedSelection = () => {
    if (!saved || saved.href !== location.href) return null;
    const text = saved.ranges.map((r) => r.toString()).join('\n');
    return text.trim() ? { ranges: saved.ranges, text } : null;
  };
})();
"#;

/// Fetching a selection: besides the plain text, the selection's Range is cloned into
/// a DOM fragment, put through the same normalisation, and returned as fragment HTML
/// — which is how images, math and links inside a selection reach a quote block
/// (plain text would evaporate them all). Relative addresses are resolved into
/// absolute URLs in place through the element attribute getters (the cloned
/// fragment's ownerDocument is still the page's, so the resolution base is right),
/// and the htmd side no longer needs a base URL.
/// With the live selection empty it falls back to the most recent selection stored by
/// SELECTION_CACHE_JS; should that cached Range have been invalidated by a re-render,
/// cloneContents yields an empty fragment while the plain text is still there — the
/// Rust side falls back to the plain-text path naturally, which is never worse than
/// an error.
/// An IIFE expression: wry JSON-serialises the evaluation result into the callback, so
/// there is no need to stringify it here.
pub(crate) static GRAB_SELECTION: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    [
        "(() => {",
        NORMALIZE_JS,
        r#"
  const sel = getSelection();
  let text = String(sel);
  let ranges = [];
  if (sel && sel.rangeCount && !sel.isCollapsed) {
    for (let i = 0; i < sel.rangeCount; i++) ranges.push(sel.getRangeAt(i));
  } else if (typeof window.__vemrySavedSelection === 'function') {
    const c = window.__vemrySavedSelection();
    if (c && c.text && c.text.trim()) { ranges = c.ranges; text = c.text; }
  }
  let html = '';
  if (ranges.length) {
    const box = document.createElement('div');
    for (const r of ranges) box.appendChild(r.cloneContents());
    // A selection clone loses the runtime state and does not line up with document order, so it is paired with the live DOM by the src attribute
    const bySrc = new Map();
    for (const l of document.images) {
      const k = l.getAttribute('src') || '';
      if (!bySrc.has(k)) bySrc.set(k, l);
    }
    const pairs = [];
    for (const img of box.querySelectorAll('img')) {
      pairs.push([img, bySrc.get(img.getAttribute('src') || '') || null]);
    }
    fixImgs(pairs);
    // A selection clone is a detached node (no computed style, no innerText newline
    // rendering), so the general path cannot run: only the lake card decoding
    // happens; a surviving pre (whose real newline text is there anyway, the gain
    // being the fence upgrade) is turned into a sentinel outright — with <br> first
    // materialised into a newline (textContent would evaporate it, which htmd's
    // default path did handle), and a viewer beside a card deduplicated through seen;
    // everything else falls back to htmd's default
    const seenCode = fixCodeCards(box);
    for (const el of box.querySelectorAll('pre')) {
      for (const b of el.querySelectorAll('br')) b.replaceWith('\n');
      if (seenCode.size && seenCode.has(codeKey(el.textContent))) { el.remove(); continue; }
      putCode(el, el.textContent, codeLang(el));
    }
    fixMath(box);
    for (const el of box.querySelectorAll('img[src]')) {
      const abs = el.src;
      if (abs && !abs.startsWith('data:')) el.setAttribute('src', abs);
    }
    for (const el of box.querySelectorAll('a[href]')) el.setAttribute('href', el.href);
    html = box.innerHTML;
  }
  return { sel: ranges.length ? text : '', html, url: location.href, title: document.title };
})()"#,
    ]
    .concat()
});

/// Fetching a whole page: math and lazy-loading normalisation on a **clone of the
/// DOM**, then outerHTML serialisation.
/// The live DOM's img elements are paired with the clone's by document order (the
/// clone happens before any modification, so the two orders agree).
pub(crate) static GRAB_PAGE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    [
        "(() => {",
        NORMALIZE_JS,
        r#"
  const root = document.documentElement.cloneNode(true);
  const liveImgs = document.querySelectorAll('img');
  const cloneImgs = root.querySelectorAll('img');
  const pairs = [];
  for (let n = 0; n < cloneImgs.length; n++) pairs.push([cloneImgs[n], liveImgs[n] || null]);
  // Code containers are paired like the images, by document order: the clone happens
  // before any modification, so the two orders agree.
  // The pairing has to be collected before fixImgs/fixCode/fixMath modify the clone.
  // The live side has to query down from documentElement —
  // document.querySelectorAll would count <html> itself among the candidates (a
  // theme class name often matches something like [class*="highlight"]), while the
  // clone side's Element.querySelectorAll never includes its root, and one element of
  // difference throws the whole pairing out of line
  const liveCode = document.documentElement.querySelectorAll(CODE_SEL);
  const cloneCode = root.querySelectorAll(CODE_SEL);
  const codePairs = [];
  for (let n = 0; n < cloneCode.length; n++) codePairs.push([cloneCode[n], liveCode[n] || null]);
  fixImgs(pairs);
  fixCode(root, codePairs);
  fixMath(root);
  return { html: root.outerHTML, url: location.href, title: document.title };
})()"#,
    ]
    .concat()
});

/// eval_with_callback plus a oneshot bridge back. With a timeout as a backstop: a stalled page must not hold up the command.
pub(crate) async fn eval_json(webview: &tauri::Webview, js: &str) -> Result<serde_json::Value, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = std::sync::Mutex::new(Some(tx));
    webview
        .eval_with_callback(js, move |result| {
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(result);
            }
        })
        .map_err(|e| e.to_string())?;
    let raw = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .map_err(|_| "the page did not respond in time".to_string())?
        .map_err(|_| "the browser window is closed".to_string())?;
    serde_json::from_str(&raw).map_err(|e| format!("failed to parse the page data: {e}"))
}

pub(crate) fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|s| s.as_str()).unwrap_or_default().to_string()
}

fn meta_from(v: &serde_json::Value) -> ClipMeta {
    ClipMeta { url: str_field(v, "url"), title: str_field(v, "title"), clipped_at: chrono::Local::now() }
}

/// Land the composed note in clips/ and return its relative path. It goes through
/// broadcast, so the index, the frontend refresh and the clips/ workflow all follow.
pub(crate) fn land_note(
    app: &AppHandle,
    state: &State<'_, AppState>,
    meta: &ClipMeta,
    content: &str,
) -> Result<String, String> {
    let (rel, event) = with_vault_mut(state, |v| {
        let rel = v.dedup_rel(&clip_rel_path(meta));
        let id = v.note_id(&rel).map_err(|e| e.to_string())?;
        let event = v.write_note(&id, content).map_err(|e| e.to_string())?;
        Ok((id.to_string(), event))
    })?;
    broadcast(app, &event);
    Ok(rel)
}

#[derive(Serialize)]
pub struct ClipSelectionResult {
    /// The quote block's text (with save_to_clips=false the frontend inserts it into the current note).
    pub text: String,
    /// The source page's URL: the frontend calls `clip_localize_note` with it once the insertion is done, to localise the images.
    pub url: String,
    pub saved_path: Option<String>,
}

/// Clip the selected text: the selection's DOM fragment → Markdown → a quote callout
/// (so images, math and links survive; with nothing convertible in the fragment it
/// falls back to plain text). `save_to_clips` decides between landing in clips/ and
/// handing the text back to the frontend to insert into a note — the former triggers
/// image localisation here directly, and the latter has the frontend call
/// `clip_localize_note` once the insertion is done (only the frontend knows where it
/// landed).
#[tauri::command]
pub async fn clip_selection(
    app: AppHandle,
    state: State<'_, AppState>,
    save_to_clips: bool,
    label: Option<String>,
) -> Result<ClipSelectionResult, String> {
    // label: the toolbar points straight at the browser window it is paired with (so
    // with several windows open nothing is guessed from "focused last"); the main
    // window's command palette and menu entry points pass None and keep the
    // focused-last semantics
    let webview = target_webview(&app, &state, label)?;
    let data = eval_json(&webview, &GRAB_SELECTION).await?;
    let sel = str_field(&data, "sel");
    let html = str_field(&data, "html");
    if sel.trim().is_empty() && html.trim().is_empty() {
        return Err("nothing is selected".into());
    }
    let meta = meta_from(&data);
    let body = match vemry_clip::fragment_to_markdown(&html) {
        md if !md.is_empty() => md,
        _ => sel,
    };
    // Inserted into a note, a long quote folds; a clip note that is nothing but the
    // quote opens expanded (the composer's own flag — its text is not used in that case)
    let quote = quote_block(&body, &meta, !save_to_clips);
    if !save_to_clips {
        return Ok(ClipSelectionResult { text: quote, url: meta.url.clone(), saved_path: None });
    }
    let page = ExtractedPage { markdown: quote.clone(), title: meta.display_title(), byline: None, fallback: false };
    let rel = land_note(&app, &state, &meta, &compose_clip_note(&page, &meta, &crate::i18n::t(crate::i18n::current(), "clip.sourceLabel")))?;
    maybe_localize(&app, rel.clone(), meta.url.clone());
    Ok(ClipSelectionResult { text: quote, url: meta.url, saved_path: Some(rel) })
}

/// The single entry point for image localisation: the settings gate plus a background
/// task. With the setting off it is a no-op.
/// All four clip landings (selection, whole page, projection, the frontend's later
/// attachment) come through here, so the gating is maintained once.
pub(crate) fn maybe_localize(app: &AppHandle, rel: String, url: String) {
    if !crate::settings::load(app).clip_localize_images {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move { localize_clip_images(app, rel, url).await });
}

/// The image localisation entry point after the frontend inserted a selection quote
/// block (only the frontend knows the destination note: the current note, the pinned
/// target, or pick-note). With the setting off it is a no-op. It is still an
/// extension of an explicit user clip action — the agent has no way to trigger it.
#[tauri::command]
pub fn clip_localize_note(app: AppHandle, rel: String, url: String) -> Result<(), String> {
    if !rel.ends_with(".md") {
        return Err("the target has to be a note".into());
    }
    maybe_localize(&app, rel, url);
    Ok(())
}

/// Clipping a whole page: outerHTML → Readability → Markdown → clips/<title>.md.
/// The note lands in a moment carrying remote image URLs; the image localisation runs
/// in the background and rewrites the note when it finishes (broadcast pushes it to
/// the frontend, and the images "develop" within seconds).
#[tauri::command]
pub async fn clip_page(
    app: AppHandle,
    state: State<'_, AppState>,
    label: Option<String>,
) -> Result<String, String> {
    let (meta, page) = grab_page_extracted(&app, &state, label).await?;
    let rel = land_note(&app, &state, &meta, &compose_clip_note(&page, &meta, &crate::i18n::t(crate::i18n::current(), "clip.sourceLabel")))?;
    maybe_localize(&app, rel.clone(), meta.url.clone());
    Ok(rel)
}

/// Fetch the current page → Readability extraction: the fetching prelude shared by whole-page clipping and "add to context".
async fn grab_page_extracted(
    app: &AppHandle,
    state: &State<'_, AppState>,
    label: Option<String>,
) -> Result<(ClipMeta, ExtractedPage), String> {
    let webview = target_webview(app, state, label)?;
    let data = eval_json(&webview, &GRAB_PAGE).await?;
    let html = str_field(&data, "html");
    if html.trim().is_empty() {
        return Err("the page has not finished loading".into());
    }
    let meta = meta_from(&data);
    let page = extract_page(&html, &meta.url).map_err(|e| e.to_string())?;
    Ok((meta, page))
}

/// The localisation budget for one image and for one clip. The fetching side (assets)
/// has its own 20MB hard limit and SSRF validation; the limits here govern the vault's
/// disk usage. An image over the limit, or one that failed, keeps its remote URL, so
/// no information is lost.
const LOCALIZE_MAX_PER_IMAGE: usize = 10 * 1024 * 1024;
const LOCALIZE_MAX_TOTAL: usize = 50 * 1024 * 1024;
const LOCALIZE_MAX_IMAGES: usize = 30;
const LOCALIZE_CONCURRENCY: usize = 4;

/// Localising a clip's images: download the body's images into clips/assets/ (named
/// by content hash, which deduplicates naturally) and rewrite the remote URLs in the
/// note into vault-root-relative paths. It uses the existing assets::fetch_image
/// pipeline (three layers of SSRF protection, no cookies, the page's Referer, the
/// disk cache). Before rewriting it rereads the note from disk and only substitutes
/// literals — an edit the user made while the download ran is not overwritten.
/// The only caller is `maybe_localize` (which holds the settings gate).
async fn localize_clip_images(app: AppHandle, rel: String, page_url: String) {
    let state = app.state::<AppState>();
    let Ok(content) = with_vault(&state, |v| {
        let id = v.note_id(&rel).map_err(|e| e.to_string())?;
        v.read_note(&id).map_err(|e| e.to_string())
    }) else {
        return;
    };

    // One image referenced in several places is downloaded once: resolved URL → the literals it appeared as
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for r in vemry_clip::image_refs(&content, &page_url) {
        match groups.iter_mut().find(|g| g.0 == r.resolved) {
            Some(g) => {
                if !g.1.contains(&r.literal) {
                    g.1.push(r.literal);
                }
            }
            None => groups.push((r.resolved, vec![r.literal])),
        }
    }
    if groups.is_empty() {
        return;
    }
    let mut partial = groups.len() > LOCALIZE_MAX_IMAGES;
    groups.truncate(LOCALIZE_MAX_IMAGES);

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(LOCALIZE_CONCURRENCY));
    let mut set = tokio::task::JoinSet::new();
    for (idx, (url, _)) in groups.iter().enumerate() {
        let (app, url, referer, sem) = (app.clone(), url.clone(), page_url.clone(), sem.clone());
        set.spawn(async move {
            let _permit = sem.acquire().await;
            (idx, super::assets::fetch_image(&app, &url, Some(&referer)).await)
        });
    }
    let mut fetched: Vec<Option<Vec<u8>>> = vec![None; groups.len()];
    while let Some(joined) = set.join_next().await {
        if let Ok((idx, Ok((bytes, _content_type)))) = joined {
            // One over the per-image budget is dropped at the moment it completes rather than after every download finishes — peak memory stays at the scale of the budget
            if bytes.len() > LOCALIZE_MAX_PER_IMAGE {
                partial = true;
                continue;
            }
            fetched[idx] = Some(bytes);
        }
    }

    let mut replace: HashMap<String, String> = HashMap::new();
    let mut total = 0usize;
    for (i, (_, literals)) in groups.iter().enumerate() {
        let Some(bytes) = &fetched[i] else {
            partial = true;
            continue;
        };
        // Only allowlisted image formats land; anything whose magic bytes are not
        // recognised (an HTML error page, a script) stays out of the vault.
        // The total budget is applied in document order, so which images are kept does
        // not depend on the order they finished in.
        let (size_ok, ext) =
            (total + bytes.len() <= LOCALIZE_MAX_TOTAL, vemry_clip::image_ext_of(bytes));
        let (true, Some(ext)) = (size_ok, ext) else {
            partial = true;
            continue;
        };
        let digest = <sha2::Sha256 as sha2::Digest>::digest(bytes);
        let hash: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        let asset_rel = format!("{}{hash}.{ext}", vemry_clip::ASSETS_DIR);
        // Content-addressed: the same name means the same content, so an existing file is reused and no second copy is written
        let saved = with_vault(&state, |v| {
            if v.root().join(&asset_rel).is_file() {
                Ok(asset_rel.clone())
            } else {
                v.save_attachment(&asset_rel, bytes).map_err(|e| e.to_string())
            }
        });
        match saved {
            Ok(path) => {
                total += bytes.len();
                for l in literals {
                    replace.insert(l.clone(), path.clone());
                }
            }
            Err(_) => partial = true,
        }
    }

    if replace.is_empty() && !partial {
        return;
    }
    let written = with_vault_mut(&state, |v| {
        let id = v.note_id(&rel).map_err(|e| e.to_string())?;
        let current = v.read_note(&id).map_err(|e| e.to_string())?;
        let mut next = vemry_clip::rewrite_images(&current, &replace);
        // The partial marker is only set on a clips/ note — a user note a selection was clipped into keeps its frontmatter untouched
        if partial && rel.starts_with(vemry_clip::CLIPS_DIR) {
            next = vemry_clip::mark_images_partial(&next);
        }
        if next == current {
            return Ok(None);
        }
        Ok(Some(v.write_note(&id, &next).map_err(|e| e.to_string())?))
    });
    if let Ok(Some(event)) = written {
        broadcast(&app, &event);
        // Should the editor buffer be dirty (the user kept typing while the download
        // ran), the frontend would skip NoteModified and the next autosave would
        // quietly wipe out the localisation on disk, orphaning every file in
        // clips/assets. So the substitution table is sent to the frontend for the live
        // buffer to apply the same URL→local path rewrite (the editor side only
        // changes an image's target).
        let replace_pairs: Vec<[String; 2]> =
            replace.iter().map(|(k, v)| [k.clone(), v.clone()]).collect();
        let _ = tauri::Emitter::emit_to(
            &app,
            "main",
            "clip://localized",
            serde_json::json!({ "path": rel, "replace": replace_pairs }),
        );
    }
}

#[derive(Serialize)]
pub struct ClipLinkResult {
    pub text: String,
    pub url: String,
    pub title: String,
}

/// Clipping a link: the current page as a `[title](url)` reference entry (read straight from BrowserState, with no eval needed).
#[tauri::command]
pub fn clip_link(state: State<'_, AppState>, label: Option<String>) -> Result<ClipLinkResult, String> {
    let inner = state.browser.0.lock().unwrap();
    let label = inner.target(label.as_deref()).ok_or("no browser window is open")?;
    let entry = inner.tabs.get(&label).ok_or("the browser tab is closed")?;
    if entry.url == super::window::BLANK {
        return Err("the tab has no page yet".into());
    }
    let meta =
        ClipMeta { url: entry.url.clone(), title: entry.title.clone(), clipped_at: chrono::Local::now() };
    Ok(ClipLinkResult { text: vemry_clip::link_entry(&meta), url: meta.url.clone(), title: meta.display_title() })
}

/// How many characters of a page body "add to context" carries at most — context is
/// reference material for the model, not a full-text archive (archiving is what
/// whole-page clipping is for). The same constant as the envelope-assembly side.
pub(crate) use vemry_agent::PAGE_CONTEXT_MAX_CHARS;

#[derive(Serialize)]
pub struct PageContext {
    pub title: String,
    pub url: String,
    pub markdown: String,
    pub truncated: bool,
}

/// "Add to context": the current page's body → Markdown, handed to the frontend as an
/// attachment of the next message.
/// It shares the extraction pipeline with whole-page clipping but writes nothing to
/// disk — the context is handed to the agent explicitly by the user, and the agent
/// still has no tool for fetching a web page. label works as in clip_*: the toolbar's
/// "summon the AI with this page" points straight at the browser window it is paired
/// with, and the panel entry point passes None.
#[tauri::command]
pub async fn browser_page_context(
    app: AppHandle,
    state: State<'_, AppState>,
    label: Option<String>,
) -> Result<PageContext, String> {
    let (meta, page) = grab_page_extracted(&app, &state, label).await?;
    let truncated = page.markdown.chars().count() > PAGE_CONTEXT_MAX_CHARS;
    let markdown = if truncated {
        page.markdown.chars().take(PAGE_CONTEXT_MAX_CHARS).collect()
    } else {
        page.markdown
    };
    Ok(PageContext { title: meta.display_title(), url: meta.url, markdown, truncated })
}

/// One-click clipping from a source card: without loading the page, land the
/// `[title](url)` reference entry as a clips/ note (whose frontmatter carries
/// source:, which is what makes "where a citation landed" findable).
#[tauri::command]
pub fn clip_url(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    title: String,
) -> Result<String, String> {
    let url = super::window::normalize_url(&url)?;
    let meta = ClipMeta { url: url.to_string(), title, clipped_at: chrono::Local::now() };
    let page = ExtractedPage {
        markdown: vemry_clip::link_entry(&meta),
        title: meta.display_title(),
        byline: None,
        fallback: false,
    };
    land_note(&app, &state, &meta, &compose_clip_note(&page, &meta, &crate::i18n::t(crate::i18n::current(), "clip.sourceLabel")))
}

/// The reverse lookup of where a citation landed (url → the clips/ note's path; see
/// `Vault::clips_for_urls` for the matching rule the agent's tool shares).
#[tauri::command]
pub fn clips_lookup(
    state: State<'_, AppState>,
    urls: Vec<String>,
) -> Result<std::collections::HashMap<String, String>, String> {
    crate::commands::with_vault(&state, |v| Ok(v.clips_for_urls(&urls)))
}
