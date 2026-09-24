//! Translating a browser tab where it is: every text block's translation appears
//! beside the block — in a column next to the page's text where there is room, under
//! the block itself where there is not — so the reader stays in the tab, docked or
//! detached, with no second window and no other view. The page is a native webview
//! with zero IPC, so everything goes through `eval`: a script installs the layer and
//! collects the page's text blocks (each tagged with an id), the job in vemry-agent
//! translates them, and every finished segment is written into its entry as text.
//! The side column lives in the document at the blocks' coordinates, so it scrolls
//! with the page by itself; it is laid out again when something moved a block,
//! pushing an entry down when the one above it is taller than its block (see
//! `COLLECT_JS`). A poll every second picks up blocks that appeared since (an endless
//! feed, a page rendering as it scrolls). Moving to another page — a navigation, a
//! reload, a single-page app's route change — ends the translation: the layer is
//! removed and the button goes back to off, and the next page is translated only
//! when asked. Pressing the button again removes the layer and stops.
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

/// Install the translation layer (once per document) and collect the text blocks
/// not yet tagged. `gen` is minted when the script first runs in a document, so a
/// different value on the next poll means a different document. `TARGET` (the
/// translation's target language) and `KNOWN` (how many blocks the job already
/// holds) are filled in by `collect_js`: a block already in the target language is
/// not collected, and a block the job never received — a poll whose answer was
/// lost on the way back — is sent again, so the two sides keep numbering alike.
///
/// Where the translations go is settled from the page as it is laid out, and again
/// whenever the viewport's width changes (a docked browser's sash, a resized window):
/// - `stack`: narrower than 900px there is no room beside the page, so each
///   translation goes into the page right under its block, and the page keeps its width;
/// - `beside`: a column 24px to the right of the translated blocks' right edge, over
///   space the page leaves empty, with the page's layout untouched;
/// - `edge`: when that does not fit (or the page has something there), a column at
///   the viewport's right edge, with the page given a right margin as wide.
///
/// The side column sits in the document: it is absolutely positioned at the blocks'
/// document coordinates, so it scrolls with the page natively and no script runs on
/// scroll. It is laid out again only when something can have moved a block — a
/// resize of the body or a block, a change to the page's DOM, a scroll inside an
/// inner scroller — plus a slow safety tick that writes nothing when nothing moved.
/// A page scrolling inside its own container (an app shell) keeps a viewport-fixed
/// column laid out on that container's scroll, as the document would not carry it.
const COLLECT_JS: &str = r#"
(() => {
  const TARGET = "__TARGET__";
  const KNOWN = __KNOWN__;
  const primary = TARGET.split('-')[0].toLowerCase();
  const CJK = primary === 'zh' || primary === 'ja' || primary === 'ko';
  let S = window.__vemryTr;
  if (!S) {
    S = window.__vemryTr = { n: 0, gen: Math.random().toString(36).slice(2), items: new Map(), mode: null, fixed: false, x: 0, w: 0, vw: innerWidth, raf: 0, rz: 0, timer: 0 };
    const STACK_BELOW = 900;
    const BESIDE = 24, EDGE = 16, GAP = 6;
    const de = document.documentElement;
    const meta = document.querySelector('meta[name="color-scheme"]');
    const declared = getComputedStyle(de).colorScheme;
    const scheme = declared && declared !== 'normal' ? declared : (meta && meta.content) || 'normal';

    // Any CSS colour — rgb(), oklch(), color(), a named one — as [r, g, b, a]: the
    // browser paints it on a 1px canvas and it is read back, so no notation is missed
    const cv = document.createElement('canvas');
    cv.width = cv.height = 1;
    const cx = cv.getContext('2d', { willReadFrequently: true });
    const rgba = (c) => {
      if (!cx || !c) return [0, 0, 0, 0];
      cx.clearRect(0, 0, 1, 1);
      cx.fillStyle = 'rgba(0,0,0,0)';
      cx.fillStyle = c;
      cx.fillRect(0, 0, 1, 1);
      const d = cx.getImageData(0, 0, 1, 1).data;
      return [d[0], d[1], d[2], d[3] / 255];
    };
    const lum = (c) => {
      const f = (v) => { v /= 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
      return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]);
    };
    const contrast = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
    const rgb = (c) => 'rgb(' + c[0] + ',' + c[1] + ',' + c[2] + ')';
    // The page's text colour taken as far towards its ground as `ratio` still allows:
    // the secondary ink at 4.5:1, the ink of rules and marks at 3:1
    const inkAt = (fg, bg, ratio) => {
      for (let t = 0.8; t > 0; t -= 0.05) {
        const c = [0, 1, 2].map((i) => Math.round(fg[i] + (bg[i] - fg[i]) * t));
        if (contrast(c, bg) >= ratio) return c;
      }
      return fg;
    };

    const layer = document.createElement('div');
    layer.setAttribute('data-vemry-tr-out', 'pane');
    // `all:initial` first: the page's own rules for div (a border, a margin, a font) must
    // not reach the layer. `all` resets color-scheme too, which the system colours
    // below resolve against, so the page's own is put back
    layer.style.cssText = 'all:initial;display:none;top:0;left:0;z-index:2147483646;box-sizing:border-box;color-scheme:' + scheme + ';';
    // the side column's ground and rule, from its first entry to its last
    const band = document.createElement('div');
    band.style.cssText = 'all:initial;display:none;position:absolute;left:0;right:0;box-sizing:border-box;';
    layer.appendChild(band);
    const style = document.createElement('style');
    style.textContent = '@media (prefers-reduced-motion: reduce){[data-vemry-tr-out]{transition:none!important}}';
    layer.appendChild(style);
    // the progress line: the translation's one running indicator, at the top of its column
    const track = document.createElement('div');
    track.setAttribute('data-vemry-tr-out', 'progress');
    track.style.cssText = 'all:initial;display:block;position:fixed;top:0;left:0;width:100%;height:2px;z-index:2147483647;pointer-events:none;color-scheme:' + scheme + ';';
    const bar = document.createElement('div');
    bar.setAttribute('data-vemry-tr-out', 'bar');
    bar.style.cssText = 'all:initial;display:block;height:100%;width:0;transition:width .3s,opacity .6s;';
    track.appendChild(bar);
    de.appendChild(layer);
    de.appendChild(track);
    S.layer = layer;
    S.progress = (done, total) => {
      const pct = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0;
      bar.style.width = pct + '%';
      bar.style.opacity = total > 0 && done >= total ? '0' : '1';
    };
    // A system colour as the page's colour scheme resolves it (dark under a dark scheme;
    // `all:initial` would reset the probe's own scheme to the light default)
    const system = (v) => {
      const p = document.createElement('i');
      p.style.cssText = 'all:initial;color-scheme:' + scheme + ';color:' + v + ';';
      track.appendChild(p);
      const c = rgba(getComputedStyle(p).color);
      p.remove();
      return c;
    };
    S.ink = { set: false, bg: 'Canvas', fg: 'CanvasText', muted: 'GrayText', rule: 'GrayText', accent: '#1f6f74', tint: 'rgba(250,204,21,.4)' };
    // The page's real colours are the ones behind its text, not the body's (an app shell
    // often paints a dark body under a light content column): taken once, from a body
    // paragraph's ancestors. No opaque ground anywhere means the page's colour scheme's
    // own canvas — dark under a dark scheme, never a white assumed
    S.recolor = (el) => {
      if (S.ink.set) return;
      let bg = null;
      for (let e = el; e; e = e.parentElement) {
        const c = rgba(getComputedStyle(e).backgroundColor);
        if (c[3] >= 0.5) { bg = c; break; }
      }
      if (!bg) bg = system('Canvas');
      let fg = rgba(getComputedStyle(el).color);
      if (fg[3] < 0.5) fg = system('CanvasText');
      const dark = lum(bg) < 0.2;
      let accent = dark ? [94, 176, 181] : [31, 111, 116];
      if (contrast(accent, bg) < 3) accent = inkAt(fg, bg, 3);
      S.ink = {
        set: true, bg: rgb(bg), fg: rgb(fg), muted: rgb(inkAt(fg, bg, 4.5)), rule: rgb(inkAt(fg, bg, 3)), accent: rgb(accent),
        // a picked block's tint; the accent line beside it is what shows on a white page
        tint: dark ? 'rgba(250,204,21,.28)' : 'rgba(250,204,21,.4)',
      };
      bar.style.background = S.ink.accent;
      S.frame();
    };

    // The text column: the left edge carrying the most text, and the right edge of the
    // blocks starting there — the one reached by half their text, so a stray wide block
    // does not move it
    S.column = (list) => {
      const rows = [];
      for (const b of list) {
        if (!b.el.isConnected) continue;
        const r = b.el.getBoundingClientRect();
        if (r.width < 120 || r.height === 0) continue;
        rows.push({ el: b.el, r, n: b.text.length, k: Math.round(r.left / 8) });
      }
      if (!rows.length) return null;
      const sum = new Map();
      for (const x of rows) sum.set(x.k, (sum.get(x.k) || 0) + x.n);
      let k = 0, most = -1;
      for (const [key, n] of sum) if (n > most) { most = n; k = key; }
      const main = rows.filter((x) => Math.abs(x.k - k) <= 1).sort((a, b) => a.r.right - b.r.right);
      let half = main.reduce((s, x) => s + x.n, 0) / 2, right = main[main.length - 1].r.right;
      for (const x of main) { half -= x.n; if (half <= 0) { right = x.r.right; break; } }
      let left = Infinity, top = Infinity, bottom = -Infinity, heavy = main[0];
      for (const x of main) {
        left = Math.min(left, x.r.left); top = Math.min(top, x.r.top); bottom = Math.max(bottom, x.r.bottom);
        if (x.n > heavy.n) heavy = x;
      }
      return { left, right, top, bottom, el: heavy.el };
    };
    // Is the space beside the column the page's own (a right-hand column, a table of
    // contents)? Probed at a few points down the column's height: anything there but the
    // column's own containers, or a floating bar, and the column goes to the edge instead
    const occupied = (col, x, w) => {
      const y0 = Math.max(0, col.top), y1 = Math.min(innerHeight, col.bottom);
      for (let i = 1; i <= 4 && y1 > y0; i++) {
        for (let j = 0; j < 3; j++) {
          const hit = document.elementFromPoint(x + (w * (2 * j + 1)) / 6, y0 + ((y1 - y0) * i) / 5);
          if (!hit || hit === de || hit === document.body || hit.contains(col.el)) continue;
          let floating = false;
          for (let e = hit; e && e !== document.body; e = e.parentElement) {
            const p = getComputedStyle(e).position;
            if (p === 'fixed' || p === 'sticky') { floating = true; break; }
          }
          if (!floating) return true;
        }
      }
      return false;
    };
    // The blocks scroll inside a container of the page's rather than with the document
    const innerScroll = (el) => {
      const scrolls = (e) => { const o = getComputedStyle(e).overflowY; return (o === 'auto' || o === 'scroll' || o === 'overlay') && e.scrollHeight > e.clientHeight + 1; };
      for (let e = el.parentElement; e && e !== document.body && e !== de; e = e.parentElement) if (scrolls(e)) return true;
      // the body's overflow is the viewport's unless the root has one of its own
      return getComputedStyle(de).overflowY !== 'visible' && scrolls(document.body);
    };
    S.prevMargin = de.style.marginRight;
    S.decide = (list) => {
      if (S.mode === 'edge') de.style.marginRight = S.prevMargin || '';
      layer.style.display = 'none';
      const vw = de.clientWidth;
      S.vw = innerWidth;
      let mode = 'stack', x = 0, w = 0;
      if (innerWidth >= STACK_BELOW) {
        const col = S.column(list);
        // about three quarters of the column for a CJK translation of an alphabetic
        // paragraph, which then ends near where the paragraph ends; nearly all of it
        // for an alphabetic one; never a line longer than a readable one
        const want = Math.round(Math.max(300, col ? Math.min(720, (col.right - col.left) * (CJK ? 0.75 : 0.95) + 26) : vw * 0.36));
        if (col) {
          S.fixed = innerScroll(col.el);
          x = Math.round(col.right) + BESIDE;
          w = Math.min(want, Math.floor(vw - EDGE - x));
          if (w >= Math.max(300, want * 0.85) && !occupied(col, x, w)) mode = 'beside';
        }
        if (mode !== 'beside') {
          mode = 'edge';
          w = Math.round(Math.min(vw * 0.48, want));
          x = vw - w;
          de.style.marginRight = w + 'px';
        }
      }
      const moved = !S.mode || (S.mode === 'stack') !== (mode === 'stack');
      S.mode = mode; S.x = x; S.w = w;
      S.frame();
      for (const it of S.items.values()) { if (moved) S.place(it); it.top = null; }
      S.schedule();
    };
    // The layer and the progress line for the mode
    S.frame = () => {
      const T = track.style, L = layer.style;
      if (S.mode !== 'beside' && S.mode !== 'edge') { L.display = 'none'; T.left = '0'; T.width = '100%'; return; }
      T.left = S.x + 'px'; T.width = S.w + 'px';
      L.display = 'block';
      L.position = S.fixed ? 'fixed' : 'absolute';
      L.width = S.w + 'px';
      L.height = S.fixed ? '100%' : '0';
      L.overflow = S.fixed ? 'hidden' : 'visible';
      L.background = S.fixed ? S.ink.bg : 'transparent';
      L.borderLeft = S.fixed ? '1px solid ' + S.ink.rule : '0';
      L.color = S.ink.fg;
      band.style.display = S.fixed ? 'none' : 'block';
      band.style.background = S.ink.bg;
      band.style.borderLeft = '1px solid ' + S.ink.rule;
      // where left:0 lands is the containing block's origin (a page may position its root)
      L.left = '0';
      L.left = Math.round(S.x - layer.getBoundingClientRect().left) + 'px';
    };
    // An entry into the page under its block (stack), or into the side column
    S.place = (it) => {
      const o = it.out.style;
      if (S.mode === 'stack') {
        o.position = 'static'; o.top = ''; o.left = ''; o.right = ''; o.color = it.failed ? S.ink.muted : it.ink;
        o.margin = it.inside ? '.3em 0' : it.mt + 'px 0 ' + it.mb + 'px';
        if (it.inside) it.el.appendChild(it.out); else it.el.after(it.out);
      } else {
        o.position = 'absolute'; o.left = '0'; o.right = '0'; o.margin = '0'; o.color = it.failed ? S.ink.muted : '';
        layer.appendChild(it.out);
      }
      it.top = null;
    };
    // Picking a block: it stays lit on the page, its entry lit, until another is picked or it is picked again
    S.selected = null;
    S.select = (id, scroll) => {
      const prev = S.selected == null ? null : S.items.get(S.selected);
      if (prev) {
        prev.el.style.background = prev.saved.bg; prev.el.style.boxShadow = prev.saved.sh; prev.el.style.borderRadius = prev.saved.br;
        prev.out.style.background = ''; prev.out.style.borderLeftColor = S.ink.rule;
      }
      if (S.selected === id) { S.selected = null; return; }
      const it = S.items.get(id);
      if (!it) { S.selected = null; return; }
      it.saved = { bg: it.el.style.background, sh: it.el.style.boxShadow, br: it.el.style.borderRadius };
      // "this one" is drawn the one way it is everywhere: an accent line along the text's
      // start edge (the shadow's inset keeps it outside the block); the tint behind it
      // barely shows on a white page
      it.el.style.background = S.ink.tint;
      it.el.style.boxShadow = '-7px 0 0 -3px ' + S.ink.accent + ',0 0 0 4px ' + S.ink.tint;
      it.el.style.borderRadius = '3px';
      it.out.style.background = 'rgba(127,127,127,.10)'; it.out.style.borderLeftColor = S.ink.accent;
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
    S.layout = () => {
      S.raf = 0;
      if (!S.mode) return;
      const stack = S.mode === 'stack';
      // Every measurement first, then every style change: a rect read between two
      // writes makes the browser lay the page out again, once per entry
      const lr = stack ? null : layer.getBoundingClientRect();
      const rows = [];
      const gone = [];
      const hidden = [];
      for (const it of S.items.values()) {
        if (!it.el.isConnected) { gone.push(it); continue; }
        const r = it.el.getBoundingClientRect();
        // what sits under the column (a page's right-hand column) is not what the reader reads
        if ((r.width === 0 && r.height === 0) || (!stack && r.left + r.width / 2 >= lr.left)) { hidden.push(it); continue; }
        rows.push({ it, t: stack ? 0 : r.top - lr.top, hb: r.height, h: 0, y: 0 });
      }
      for (const it of gone) if (it.out.isConnected) it.out.remove();
      for (const it of hidden) if (it.out.style.display !== 'none') it.out.style.display = 'none';
      for (const x of rows) {
        // a page that rendered its block again may have dropped the entry beside it
        if (!x.it.out.isConnected) S.place(x.it);
        if (x.it.out.style.display === 'none') x.it.out.style.display = '';
      }
      // under its block, an entry is laid out by the page itself
      if (stack) return;
      for (const x of rows) x.h = x.it.out.offsetHeight;
      rows.sort((a, b) => a.t - b.t);
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
      // only what moved is written: a pass where nothing moved leaves the page alone
      for (const x of rows) {
        const y = Math.round(x.y);
        if (x.it.top !== y) { x.it.top = y; x.it.out.style.top = y + 'px'; }
      }
      if (!S.fixed && rows.length) {
        let lo = Infinity, hi = -Infinity;
        for (const x of rows) { lo = Math.min(lo, x.y); hi = Math.max(hi, x.y + x.h); }
        const top = Math.round(lo - 12) + 'px', height = Math.round(hi - lo + 24) + 'px';
        if (band.style.top !== top) band.style.top = top;
        if (band.style.height !== height) band.style.height = height;
      }
    };
    S.schedule = () => { if (!S.raf) S.raf = requestAnimationFrame(S.layout); };
    // The page's own DOM changes lay out at most every 300ms: a live page (a ticker, a
    // clock, a feed) changes every frame, and each pass measures every entry
    S.scheduleSoon = () => {
      if (S.moTimer) return;
      S.moTimer = setTimeout(() => { S.moTimer = 0; S.schedule(); }, 300);
    };
    // What can move a block: the body or a block changing size (an image loading, a font
    // arriving, a section opening), the page changing its DOM — the entries' own changes
    // aside — and, only for a page scrolling in a container of its own, that scroll
    S.ro = new ResizeObserver(() => S.schedule());
    S.ro.observe(document.body);
    const ours = (n) => n.nodeType === 1 && n.hasAttribute('data-vemry-tr-out');
    S.mo = new MutationObserver((records) => {
      for (const r of records) {
        const at = r.target.nodeType === 1 ? r.target : r.target.parentElement;
        if (at && at.closest('[data-vemry-tr-out]')) continue;
        const nodes = [...r.addedNodes, ...r.removedNodes];
        if (nodes.length && nodes.every(ours)) continue;
        S.scheduleSoon();
        return;
      }
    });
    S.mo.observe(document.body, { childList: true, subtree: true, characterData: true });
    S.onScroll = (e) => { if (S.mode && S.mode !== 'stack' && (S.fixed || e.target !== document)) S.schedule(); };
    document.addEventListener('scroll', S.onScroll, true);
    S.onResize = () => {
      if (S.rz) return;
      S.rz = requestAnimationFrame(() => {
        S.rz = 0;
        if (innerWidth !== S.vw) S.decide([...S.items.values()]);
        else S.schedule();
      });
    };
    window.addEventListener('resize', S.onResize);
    // a safety net for a move nothing reports (a transform, an animation's end): it
    // writes only what moved
    S.timer = setInterval(S.schedule, 2000);
    // An entry's state is its left rule: dotted while waiting, dashed while the text
    // streams in, solid when done, double when it will not come. The text itself is
    // never faded; the one that says a translation failed is in the secondary ink
    S.apply = (id, text, done) => {
      const it = S.items.get(id);
      if (!it) return;
      if (done) { it.pending = false; it.out.style.minHeight = ''; }
      it.failed = false;
      it.out.style.borderLeftStyle = done ? 'solid' : 'dashed';
      it.out.style.color = S.mode === 'stack' ? it.ink : '';
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
          // a displayed formula wider than the column is shrunk to fit rather than scrolled
          if (c instanceof HTMLElement && c.scrollWidth > c.clientWidth && c.clientWidth > 0) { c.style.fontSize = Math.floor((c.clientWidth / c.scrollWidth) * 96) + '%'; c.style.overflow = 'hidden'; }
        }
      }
      S.schedule();
    };
    // The segment will not arrive (the request failed, or the model left it out twice):
    // the placeholder goes and the entry says so, rather than waiting for ever
    S.fail = (id, label) => {
      const it = S.items.get(id);
      if (!it || !it.pending) return;
      it.pending = false;
      it.failed = true;
      it.out.textContent = label;
      it.out.style.color = S.ink.muted;
      it.out.style.minHeight = '';
      it.out.style.borderLeftStyle = 'double';
      S.schedule();
    };
    // The placeholder: as many still bars as the block has lines (the shape of what is
    // coming; nothing loops beside the text — the progress line is what runs)
    S.skeleton = (o, lines) => {
      o.textContent = '';
      for (let i = 0; i < lines; i++) {
        const b = document.createElement('div');
        b.style.cssText = 'all:initial;display:block;height:.65em;margin:.5em 0;border-radius:4px;background:rgba(127,127,127,.2);' + (i === lines - 1 && lines > 1 ? 'width:60%;' : '');
        o.appendChild(b);
      }
    };
    // The background stopped at its budget before block `at`: that block's entry says so and offers to go on
    S.wants = null;
    S.pausedAt = null;
    S.pause = (at, labels) => {
      const it = S.items.get(at);
      if (!it || !it.pending) return;
      S.pausedAt = at;
      it.out.textContent = '';
      const note = document.createElement('span');
      note.style.cssText = 'all:initial;font:inherit;font-size:max(12px,.85em);color:' + S.ink.muted + ';';
      note.textContent = labels.here + ' · ';
      it.out.appendChild(note);
      for (const [key, label] of [['more', labels.more], ['all', labels.all]]) {
        const b = document.createElement('span');
        b.style.cssText = 'all:initial;font:inherit;font-size:max(12px,.85em);color:inherit;cursor:pointer;text-decoration:underline;margin-right:.8em;';
        b.textContent = label;
        b.addEventListener('click', (e) => { e.stopPropagation(); S.wants = key; S.resume(); });
        it.out.appendChild(b);
      }
      S.schedule();
    };
    S.resume = () => {
      const it = S.pausedAt == null ? null : S.items.get(S.pausedAt);
      S.pausedAt = null;
      if (it && it.pending) S.skeleton(it.out, it.lines);
      S.schedule();
    };
    S.clear = () => {
      clearInterval(S.timer);
      if (S.raf) cancelAnimationFrame(S.raf);
      if (S.rz) cancelAnimationFrame(S.rz);
      clearTimeout(S.moTimer);
      S.ro.disconnect();
      S.mo.disconnect();
      document.removeEventListener('scroll', S.onScroll, true);
      document.removeEventListener('click', S.onPageClick, true);
      window.removeEventListener('resize', S.onResize);
      if (S.selected != null) S.select(S.selected, false);
      for (const it of S.items.values()) { it.out.remove(); it.el.removeAttribute('data-vemry-tr'); }
      layer.remove();
      track.remove();
      de.style.marginRight = S.prevMargin || '';
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
  const inTarget = (el, text) => {
    const tagged = el.closest('[lang]');
    if (tagged && tagged !== document.documentElement && tagged !== document.body) {
      const l = (tagged.getAttribute('lang') || '').split('-')[0].toLowerCase();
      if (l) return l === primary;
    }
    if (CJK) {
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
  // A translation into Chinese, Japanese or Korean is set in the app's CJK faces for its
  // language, after the page's own faces (which still draw its Latin letters). A generic
  // family of the page's is dropped from in front of them: `serif` would draw the CJK in
  // the language's serif face (Song for Chinese), whose strokes vanish at small sizes
  const HANT = /^zh-(tw|hk|mo|hant)/i.test(TARGET);
  const CJK_FACES = primary === 'ja'
    ? '"Hiragino Sans", "Hiragino Kaku Gothic ProN", "Yu Gothic UI", "Yu Gothic", "Meiryo", "Noto Sans JP", "Noto Sans CJK JP"'
    : primary === 'ko'
      ? '"Apple SD Gothic Neo", "Malgun Gothic", "Noto Sans KR", "Noto Sans CJK KR"'
      : HANT
        ? '"PingFang TC", "Noto Sans TC", "Microsoft JhengHei"'
        : '"PingFang SC", "Hiragino Sans GB", "Noto Sans SC", "Microsoft YaHei"';
  const GENERIC = /^(serif|sans-serif|monospace|cursive|fantasy|ui-serif|ui-monospace|ui-rounded|math|fangsong|emoji)$/i;
  const facesFor = (family) => {
    if (!CJK) return family;
    const own = family.split(',').map((f) => f.trim()).filter((f) => f && !GENERIC.test(f.replace(/^["']|["']$/g, '')));
    return [...own, CJK_FACES, 'sans-serif'].join(', ');
  };
  const BLOCK = new Set(['DIV','P','UL','OL','LI','TABLE','SECTION','ARTICLE','HEADER','FOOTER','ASIDE','NAV','BLOCKQUOTE','PRE','H1','H2','H3','H4','H5','H6','FIGURE','FORM','FIELDSET','DL','DD','DT','HR','TR','TD','TH','MAIN','DETAILS','SUMMARY']);
  const TEXTY = new Set(['P','H1','H2','H3','H4','H5','H6','LI','BLOCKQUOTE','DD','DT','DIV','SPAN','SECTION','ARTICLE','SUMMARY','MAIN']);
  const out = [];
  // Tagged here but never received on the other side: sent again, ahead of anything new,
  // so the ids the job hands out (its segment count, counting up) stay the layer's ids
  for (const [id, it] of S.items) if (id >= KNOWN) out.push({ id, text: it.text });
  const found = [];
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
    found.push({ el, text, maths });
  }
  if (found.length) {
    // the colours come from a body paragraph where there is one: a heading is often in a colour of its own
    S.recolor((found.find((f) => f.el.tagName === 'P') || found[0]).el);
    // the column is settled from the first blocks, and again while there were too few to
    // go by (a page still rendering); after that only a change of the viewport's width moves it
    if (!S.mode || S.items.size < 3) S.decide([...S.items.values(), ...found]);
  }
  const cut = S.mode === 'beside' || S.mode === 'edge' ? S.layer.getBoundingClientRect().left : Infinity;
  // Everything the entries need is read from the page first and only then are they
  // placed: an entry going into the page's flow moves what comes after it, so a read
  // after each write would lay the whole page out again once per block
  const blocks = [];
  for (const { el, text, maths } of found) {
    const r = el.getBoundingClientRect();
    // What sits under the column — a page's right-hand column, its promoted items — is not
    // what the reader is reading, and its translation would float beside nothing
    if (r.left + r.width / 2 >= cut) continue;
    const cs = getComputedStyle(el);
    blocks.push({
      el, text, maths, height: r.height,
      fontSize: parseFloat(cs.fontSize), fontFamily: cs.fontFamily, fontWeight: cs.fontWeight,
      lineHeight: parseFloat(cs.lineHeight), display: cs.display, marginBottom: parseFloat(cs.marginBottom),
      color: cs.color, parentDisplay: el.parentElement ? getComputedStyle(el.parentElement).display : '',
    });
  }
  for (const b of blocks) {
    const { el, text, maths } = b;
    const id = S.n++;
    el.setAttribute('data-vemry-tr', String(id));
    const o = document.createElement('div');
    o.setAttribute('data-vemry-tr-out', String(id));
    o.setAttribute('lang', TARGET);
    // A touch smaller than the block: a CJK translation of an English paragraph then ends
    // about where the paragraph does, and the rows below it stay aligned
    const fs = Math.max(12, Math.round(b.fontSize * 0.94));
    o.style.cssText = 'all:initial;display:block;box-sizing:border-box;padding:0 12px 0 10px;white-space:pre-wrap;overflow-wrap:anywhere;cursor:pointer;color:inherit;border-left:3px dotted ' + S.ink.rule + ';line-height:' + (CJK ? '1.7' : '1.55') + ';' + (primary === 'ko' ? 'word-break:keep-all;' : CJK ? 'line-break:strict;' : '');
    o.style.fontFamily = facesFor(b.fontFamily);
    o.style.fontSize = fs + 'px';
    o.style.fontWeight = b.fontWeight;
    // the placeholder: the height the translation will most likely have (the block's), so a
    // translation landing above the reader does not push the rows below it
    const lh = b.lineHeight || b.fontSize * 1.5;
    const lines = Math.max(1, Math.min(4, Math.round(b.height / lh)));
    o.style.minHeight = Math.round(b.height) + 'px';
    S.skeleton(o, lines);
    o.addEventListener('click', () => S.select(id, true));
    // Under its block: inside it where a sibling would not sit below it (a list item, a
    // row of a flex or grid parent), else right after it, pulled up into half the
    // block's bottom margin so the pair reads as one, and keeping that margin below
    const inside = /^(LI|DT|DD|SUMMARY)$/.test(el.tagName) || (/flex|grid/.test(b.parentDisplay) && !/flex|grid/.test(b.display));
    const mb = Math.max(0, b.marginBottom || 0);
    const it = { el, out: o, math: maths, pending: true, lines, text, ink: b.color, inside, mt: -Math.round(Math.min(mb * 0.5, fs * 0.4)), mb: Math.round(mb > 0 ? mb : fs * 0.6), top: null };
    S.items.set(id, it);
    S.place(it);
    S.ro.observe(el);
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

/// Move the translation's progress line.
fn progress_js(done: usize, total: usize) -> String {
    format!("(() => {{ const S = window.__vemryTr; if (S) S.progress({done}, {total}); }})()")
}

/// The collect script for a target language; `known` is how many blocks the job
/// already holds, which is where the page's own numbering has to pick up.
fn collect_js(target: &str, known: usize) -> String {
    COLLECT_JS.replace("__TARGET__", &target.replace(['"', '\\'], "")).replace("__KNOWN__", &known.to_string())
}

/// Mark the entries of segments that will not arrive.
fn failed_js(ids: &[u32], label: &str) -> String {
    let list = serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
    let label = serde_json::to_string(label).unwrap_or_else(|_| "\"\"".into());
    format!("(() => {{ const S = window.__vemryTr; if (S) for (const id of {list}) S.fail(id, {label}); }})()")
}

/// What page the tab is on, without touching it: the layer's document token (null
/// when this document has no layer — the tab navigated or reloaded) and the URL.
const PROBE_JS: &str = "(() => { const S = window.__vemryTr; return { gen: S ? S.gen : null, url: location.href }; })()";

/// The page's identity for the poll: the URL without its fragment (an anchor jump
/// stays on the page; a single-page app's route change does not).
fn page_key(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

/// Remove the layer and the entries, and untag the blocks.
const CLEAR_JS: &str = "(() => { if (window.__vemryTr) window.__vemryTr.clear(); return true; })()";

/// Put a translation into its entry — the text so far while it streams, final when done.
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
            // These will not arrive: the entries stop waiting, and they count towards
            // the progress line, which would otherwise never fill
            TranslateEvent::Failed { ids, .. } => {
                let label = crate::i18n::t(crate::i18n::current(), "translate.untranslated");
                let _ = ev_webview.eval(failed_js(&ids, &label));
                let mut counted = counted.lock().unwrap();
                for id in ids {
                    if counted.insert(id) {
                        ev_done.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            TranslateEvent::Idle => {}
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
            let known = poll_t.job.len();
            let Ok(data) = eval_json(&webview, &collect_js(&poll_t.job.target, known)).await else { continue };
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
            if !c.blocks.is_empty() {
                poll_t.job.extend(segments_of(&c.blocks));
                poll_total.store(poll_t.job.len(), Ordering::Relaxed);
            }
            // after `extend`, so a block that came in on this very poll and is already on
            // screen goes to the front of the queue rather than waiting a second for the next one
            if !c.visible.is_empty() {
                poll_t.job.request(&c.visible, false);
            }
            if c.blocks.is_empty() {
                continue;
            }
            // blocks that came from the cache landed through `extend` without an event
            for (id, text) in poll_t.job.translations() {
                if c.blocks.iter().any(|b| b.id == id) {
                    let _ = webview.eval(apply_js(id, &text, true));
                    poll_t.done.fetch_add(1, Ordering::Relaxed);
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
    let data = eval_json(&webview, &collect_js(&target, 0)).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_js_fills_its_placeholders_and_cannot_be_broken_out_of() {
        let js = collect_js("zh-CN", 7);
        assert!(js.contains(r#"const TARGET = "zh-CN";"#));
        assert!(js.contains("const KNOWN = 7;"));
        assert!(!js.contains("__TARGET__") && !js.contains("__KNOWN__"));
        // a target is a setting the user typed: a quote or a backslash in it cannot end the string
        let js = collect_js(r#"x"; alert(1); ""#, 0);
        assert!(js.contains(r#"const TARGET = "x; alert(1); ";"#));
    }

    #[test]
    fn page_key_drops_only_the_fragment() {
        assert_eq!(page_key("https://a.b/c?d=1#e"), "https://a.b/c?d=1");
        assert_eq!(page_key("https://a.b/c"), "https://a.b/c");
    }
}
