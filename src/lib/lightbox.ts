// Zoomed image viewing: an overlay in the current window (a lightbox), with no second
// window opened. Wheel and trackpad pinch zooming (the zoom's fixed point follows the
// pointer), drag panning, a double click toggling between "fit to window" and "actual
// pixels", and Esc, a click on the background or the close button to leave.
// Plain framework-agnostic DOM, shared by the editor widget (livepreview) and the
// reading projection (ReadingView); the styling lives in the .imgbox section of
// app.css.

import { t } from "$lib/i18n/i18n.svelte";
import { browser } from "$lib/stores/browser.svelte";

let closeCurrent: (() => void) | null = null;

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function openImageLightbox(src: string, alt = ""): void {
  closeCurrent?.();
  const overlay = document.createElement("div");
  overlay.className = "imgbox";
  overlay.tabIndex = -1; // focus the overlay, so keys no longer land in the editor behind it
  const img = document.createElement("img");
  img.src = src;
  img.alt = alt;
  img.draggable = false;
  const closeBtn = document.createElement("button");
  closeBtn.className = "imgbox-close";
  closeBtn.title = t("editor.closeEsc");
  closeBtn.textContent = "✕";
  const tag = document.createElement("div");
  tag.className = "imgbox-zoom";
  overlay.append(img, closeBtn, tag);

  // The transform model: the image is centred by flex and the transform origin is its
  // centre, translating first and scaling second — so the screen coordinate of an
  // offset u inside the image = the window's centre + (tx,ty) + scale·u
  let scale = 1;
  let tx = 0;
  let ty = 0;
  const apply = () => {
    img.style.transform = `translate(${tx}px, ${ty}px) scale(${scale})`;
  };

  // The zoom indicator: with a natural size it is computed against the original
  // image's pixels (100% = pixel for pixel), and something like an SVG with no natural
  // size falls back to a relative factor; it flashes briefly and hides itself
  let tagTimer = 0;
  const showZoom = () => {
    const pct = img.naturalWidth
      ? Math.round((img.clientWidth * scale * 100) / img.naturalWidth)
      : Math.round(scale * 100);
    tag.textContent = `${pct}%`;
    tag.classList.add("show");
    clearTimeout(tagTimer);
    tagTimer = window.setTimeout(() => tag.classList.remove("show"), 900);
  };

  /** Change the factor to scale·k with the screen point (x,y) as the fixed point */
  const zoomAt = (k: number, x: number, y: number) => {
    const next = clamp(scale * k, 0.15, 12);
    const cx = window.innerWidth / 2;
    const cy = window.innerHeight / 2;
    tx += (1 - next / scale) * (x - cx - tx);
    ty += (1 - next / scale) * (y - cy - ty);
    scale = next;
    apply();
    showZoom();
  };

  // Wheel and pinch zooming. A trackpad pinch reports as ctrlKey plus a small deltaY
  // and gets a large gain; an ordinary wheel notch is ±100 or so and gets a small gain
  // (about 1.25× per notch). preventDefault also stops the editor behind from
  // scrolling and the WebView from zooming the page
  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const k = Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0022));
    zoomAt(clamp(k, 0.4, 2.5), e.clientX, e.clientY);
  };
  overlay.addEventListener("wheel", onWheel, { passive: false });

  // Drag panning (starting on the image or on the background); moving past a threshold
  // means it no longer counts as "click the background to close".
  // Only a start on the image or the background — pointer capture redirects the click
  // to the overlay, and the close button's click must not be swallowed
  let dragging = false;
  let moved = 0;
  let lastX = 0;
  let lastY = 0;
  overlay.onpointerdown = (e) => {
    if (e.button !== 0 || (e.target !== img && e.target !== overlay)) return;
    dragging = true;
    moved = 0;
    lastX = e.clientX;
    lastY = e.clientY;
    overlay.setPointerCapture(e.pointerId);
    overlay.classList.add("dragging");
    img.style.transition = "none"; // panning has to track the finger; easing is left to wheel zooming
  };
  overlay.onpointermove = (e) => {
    if (!dragging) return;
    moved += Math.abs(e.clientX - lastX) + Math.abs(e.clientY - lastY);
    tx += e.clientX - lastX;
    ty += e.clientY - lastY;
    lastX = e.clientX;
    lastY = e.clientY;
    apply();
  };
  overlay.onpointerup = overlay.onpointercancel = () => {
    dragging = false;
    overlay.classList.remove("dragging");
    img.style.transition = "";
  };

  // A click on the background closes; a click on the image does not (the double click
  // has to stay available for toggling the factor, and closing on a single click would
  // make it unreachable)
  overlay.onclick = (e) => {
    if (e.target === overlay && moved <= 4) close();
  };
  closeBtn.onclick = (e) => {
    e.stopPropagation();
    close();
  };
  img.ondblclick = (e) => {
    e.preventDefault();
    // Fit to window ↔ actual pixels; when the original is smaller than the fitted state it goes to 2×
    const pixel = img.naturalWidth && img.clientWidth ? img.naturalWidth / img.clientWidth : 1;
    if (Math.abs(scale - 1) < 0.01) {
      zoomAt(Math.max(pixel, 2) / scale, e.clientX, e.clientY);
    } else {
      scale = 1;
      tx = 0;
      ty = 0;
      apply();
      showZoom();
    }
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  };
  window.addEventListener("keydown", onKey, true);

  const prevFocus = document.activeElement;
  document.body.appendChild(overlay);
  browser.lightboxOpen = true; // a docked browser tab is veiled while the overlay is up
  overlay.focus();
  requestAnimationFrame(() => overlay.classList.add("show"));

  function close() {
    browser.lightboxOpen = false;
    window.removeEventListener("keydown", onKey, true);
    overlay.classList.remove("show");
    window.setTimeout(() => overlay.remove(), 180); // wait for the fade-out to finish
    if (closeCurrent === close) closeCurrent = null;
    if (prevFocus instanceof HTMLElement) prevFocus.focus();
  }
  closeCurrent = close;
}
