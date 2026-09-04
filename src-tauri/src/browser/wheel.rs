//! Smoothing a discrete scroll wheel (macOS). WKWebView jumps instantly for a
//! non-precise wheel — one slow notch often moves only ~4px, both tiny and juddery,
//! where a tweened animation per notch is what a scroll wheel is expected to feel
//! like. So the browser window injects [`SMOOTH_WHEEL_JS`], which takes over the
//! wheel events of a discrete wheel and animates them with rAF; "is this a discrete
//! wheel" is not guessed at by a JS heuristic (a trackpad and a wheel are all but
//! indistinguishable on the DOM side) but classified authoritatively by a local
//! NSEvent monitor reading `hasPreciseScrollingDeltas` and pushed into each browser
//! window by eval — for a trackpad or a precise mouse (a Magic Mouse, an MX-series
//! with driver smoothing on) the script does not intervene at all and the native
//! feel is kept exactly.

use std::cell::Cell;
use std::ptr::NonNull;
use std::time::Instant;

use block2::RcBlock;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventPhase};
use tauri::{AppHandle, Manager};

/// The script injected into a browser window. Dormant by default
/// (`__vemryWheelDiscrete=false`), it only takes over once the native side pushes
/// "the current input device is a discrete wheel"; losing the flag on a navigation
/// does no harm either — while scrolling it is re-pushed every ~300ms, so a new page
/// is smooth again within a notch.
/// Where it hands back to the native behaviour (always): the page already called
/// preventDefault, a modifier is held (zoom or horizontal scroll), the scroll is
/// mostly horizontal, or no scrollable container is found (the page is at the top or
/// the bottom, preserving the rubber-band).
pub(crate) const SMOOTH_WHEEL_JS: &str = r#"
(() => {
  'use strict';
  if (window.__vemrySmoothWheel) return;
  window.__vemrySmoothWheel = true;
  if (window.__vemryWheelDiscrete === undefined) window.__vemryWheelDiscrete = false;

  const TAU = 90;       // the exponential approach time constant (ms): small = responsive, large = floaty
  const MULT = 2.5;     // deltaY amplification (WebKit already applies the macOS acceleration curve; this is a linear lift)
  const MIN_STEP = 45;  // the floor movement of one slow notch (px): the native ~4px is what makes it judder
  const MAX_STEP = 200; // the per-event movement cap: fast scrolling gets its speed from events accumulating, not from one huge step

  let anim = null; // { el, rest, last, prev } — only one scroll container is driven at a time
  let raf = 0;

  const stopAnim = () => {
    if (anim) {
      if (anim.prev) anim.el.style.scrollBehavior = anim.prev;
      else anim.el.style.removeProperty('scroll-behavior');
    }
    anim = null;
    if (raf) { cancelAnimationFrame(raf); raf = 0; }
  };

  const canScroll = (el, dir) => {
    const room = el.scrollHeight - el.clientHeight;
    if (room < 2) return false;
    return dir < 0 ? el.scrollTop > 0 : el.scrollTop < room - 1;
  };

  // The nearest scrollable container on the event path (composedPath goes through
  // the shadow DOM); the root scroller is judged separately, and returns null when
  // it cannot scroll (at the top or the bottom) to hand back to the native behaviour
  const findScroller = (e, dir) => {
    const root = document.scrollingElement || document.documentElement;
    for (const n of e.composedPath()) {
      if (!(n instanceof Element) || n === document.body || n === document.documentElement) continue;
      const cs = getComputedStyle(n);
      if (!/(auto|scroll|overlay)/.test(cs.overflowY)) continue;
      if (canScroll(n, dir)) return n;
    }
    return canScroll(root, dir) ? root : null;
  };

  const stepFrame = (now) => {
    if (!anim) { raf = 0; return; }
    const dt = Math.min(now - anim.last, 50);
    anim.last = now;
    let move = anim.rest * (1 - Math.exp(-dt / TAU));
    if (Math.abs(anim.rest) <= 1) move = anim.rest;
    const before = anim.el.scrollTop;
    anim.el.scrollTop = before + move;
    const actual = anim.el.scrollTop - before;
    anim.rest -= move;
    // Finished, or hit a boundary (asked for ≥1px of movement and nothing moved): stop
    if (Math.abs(anim.rest) < 1 || (Math.abs(move) >= 1 && actual === 0)) { stopAnim(); return; }
    raf = requestAnimationFrame(stepFrame);
  };

  const onWheel = (e) => {
    if (!window.__vemryWheelDiscrete) { stopAnim(); return; }
    if (e.defaultPrevented) return;
    if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
    let dy = e.deltaY;
    if (e.deltaMode === 1) dy *= 16; else if (e.deltaMode === 2) dy *= innerHeight;
    if (!dy || Math.abs(e.deltaX) > Math.abs(dy)) return;
    const dir = dy > 0 ? 1 : -1;
    // An animation is running and can still scroll: stay with the same container
    // (the normal case for a burst of scrolling, saving a composedPath +
    // getComputedStyle DOM climb per event; it also approximates a browser's gesture
    // lock — the container does not change until the tween has finished)
    const el = anim && canScroll(anim.el, dir) ? anim.el : findScroller(e, dir);
    if (!el) { stopAnim(); return; }
    e.preventDefault();
    if (!anim || anim.el !== el) {
      stopAnim();
      // Force instant scrolling for the duration of the animation: with the page
      // setting scroll-behavior:smooth, a per-frame scrollTop would be tweened a
      // second time and freeze
      anim = { el, rest: 0, last: performance.now(), prev: el.style.scrollBehavior };
      el.style.scrollBehavior = 'auto';
    }
    if (anim.rest && Math.sign(anim.rest) !== dir) anim.rest = 0; // an immediate stop when the direction reverses
    anim.rest += dir * Math.min(Math.max(Math.abs(dy) * MULT, MIN_STEP), MAX_STEP);
    if (!raf) { anim.last = performance.now(); raf = requestAnimationFrame(stepFrame); }
  };

  // Attached to window in the bubble phase (the end of the path): the page's own
  // wheel handler runs first, and its preventDefault/stopPropagation both mean "the
  // page wants to handle this itself", so we keep out
  window.addEventListener('wheel', onWheel, { passive: false });
  window.addEventListener('mousedown', stopAnim, true);
})();
"#;

/// Install the local NSEvent wheel monitor (which sees this process's events only),
/// classify the input device by `hasPreciseScrollingDeltas` and eval the result into
/// every browser window.
/// It is pushed whenever the classification changes, and re-pushed every 300ms while
/// a discrete scroll is running — a new page after a navigation has the flag back at
/// zero, and the re-push catches up within a notch. Called on the main thread
/// (setup); the monitor lives for the whole process, so removeMonitor is never
/// needed.
pub fn install_monitor(app: &AppHandle) {
    let app = app.clone();
    let last_class = Cell::new(false);
    let last_push: Cell<Option<Instant>> = Cell::new(None);
    let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // With no browser window at all (both the registry and the warm-up shell
        // empty) there is nothing to do: a user with a discrete mouse scrolling the
        // editor in the main window does not pay a webview_windows clone per event
        // for a browser feature; once a browser window is open, the 300ms re-push
        // while scrolling catches the classification up (the same recovery path as
        // losing the flag on a navigation)
        let has_browser = app.try_state::<crate::AppState>().is_some_and(|s| {
            let inner = s.browser.0.lock().unwrap();
            !inner.tabs.is_empty() || inner.pool.is_some()
        });
        if !has_browser {
            return event.as_ptr();
        }
        let ev = unsafe { event.as_ref() };
        // A discrete wheel means non-precise and with no gesture phase (a trackpad carries a phase even on the momentum tail)
        let discrete = !ev.hasPreciseScrollingDeltas()
            && ev.phase() == NSEventPhase::None
            && ev.momentumPhase() == NSEventPhase::None;
        let due = discrete && last_push.get().is_none_or(|t| t.elapsed().as_millis() >= 300);
        if last_class.get() != discrete || due {
            last_class.set(discrete);
            last_push.set(Some(Instant::now()));
            let js = if discrete {
                "window.__vemryWheelDiscrete=true"
            } else {
                "window.__vemryWheelDiscrete=false"
            };
            for (label, w) in app.webviews() {
                if label.starts_with("browser-") {
                    let _ = w.eval(js);
                }
            }
        }
        // The event is passed through untouched: the monitor only observes to classify, and the scrolling itself is the injected script's job
        event.as_ptr()
    });
    unsafe {
        // By the AppKit contract the returned monitor token must be held strongly
        // (it is only ever used for removeMonitor); the monitor lives for the whole
        // process, so the token is leaked deliberately (the same as HoverKeyOwner) —
        // releasing it by dropping the Retained would rely on an undocumented
        // internal strong reference in the system
        if let Some(token) =
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::ScrollWheel, &block)
        {
            std::mem::forget(token);
        }
    }
}
