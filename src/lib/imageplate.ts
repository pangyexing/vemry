// A light plate behind an image drawn in dark ink on a transparent ground — a chart
// exported without a background, a formula, a line diagram. On the dark theme's ground
// such an image all but disappears (black lines on the editor measured 1.26:1, and
// 1.04:1 on the lightbox's black veil); with the plate it reads as it does in the light
// theme. The plate is the light theme's own ground, 8px of it round the picture, with
// 6px corners, and it is drawn in the dark theme only — except in the lightbox, whose
// veil is dark in both. An opaque picture (a photo, a screenshot) and one drawn in light
// ink are left exactly as they are.
//
// One mechanism for every surface that shows a note's or a page's picture — the editor's
// image widget, an embed's body, the reading projection, the translation view, the file
// preview, the lightbox: a capture-phase `load` listener on the document looks at each
// such image once, when it has loaded, and marks the ones that need a plate
// (`img-plate`, drawn by app.css). The look is a 64px copy of the picture on a canvas:
// enough transparent pixels, and enough of the visible ink dark. An image from another
// origin cannot be read back from a canvas (the read throws, and the canvas would be
// tainted), so one is never sampled: the vault's images are blob URLs of this origin,
// and the reading projection's come through the image proxy, another origin — those
// keep their look.
//
// The plate changes nothing in the layout: the image's box keeps its size (border-box
// sizing, the picture's own aspect ratio on the box) and the plate's padding comes out
// of the picture, so nothing around an image moves when it is marked.

/** The images the plate applies to: pictures that are content, never the interface's own. */
const CONTENT_IMAGES = [
  ".cm-lp-image img", // the editor's image widget
  "img[data-asset]", // a vault image in rendered Markdown: an embed's body, the translation view
  "img.md-imgload", // the reading projection
  ".md-body img",
  ".imgbox img", // the lightbox
  ".image-wrap > img", // the file preview
].join(", ");

/** The side of the sampled copy: 64 × 64 at most, 4096 pixels to look at. */
const SAMPLE = 64;
/** A pixel this transparent counts as ground. */
const CLEAR_ALPHA = 8;
/** At least this share of the picture must be clear ground for it to be a drawing on transparency. */
const MIN_CLEAR = 0.25;
/** Ink darker than this relative luminance reads at under 2.4:1 on the dark ground. */
const DARK_LUMA = 0.1;
/** At least this share of the visible ink (weighted by its opacity) must be that dark. */
const MIN_DARK_INK = 0.3;

/** Verdicts by image URL: a picture shown twice (the lightbox, a remounted widget) is looked at once. */
const verdicts = new Map<string, boolean>();
let canvas: HTMLCanvasElement | null = null;

/** sRGB channel → linear light, as a table (the sample reads four thousand pixels). */
const LINEAR = Array.from({ length: 256 }, (_, i) => {
  const c = i / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
});

function sameOrigin(src: string): boolean {
  if (src.startsWith("blob:") || src.startsWith("data:")) return true;
  try {
    return new URL(src, location.href).origin === location.origin;
  } catch {
    return false;
  }
}

/** The picture's own size; an SVG sized by its viewBox alone has none, and its box on
 *  the page stands in (the ratio is what the plate needs). */
function sizeOf(img: HTMLImageElement): [number, number] {
  return img.naturalWidth && img.naturalHeight ? [img.naturalWidth, img.naturalHeight] : [img.clientWidth, img.clientHeight];
}

/** Whether a loaded image needs the plate; null when it cannot be looked at. */
function needsPlate(img: HTMLImageElement): boolean | null {
  const [w, h] = sizeOf(img);
  if (!w || !h) return null;
  const scale = Math.min(1, SAMPLE / Math.max(w, h));
  const cw = Math.max(1, Math.round(w * scale));
  const ch = Math.max(1, Math.round(h * scale));
  canvas ??= document.createElement("canvas");
  canvas.width = cw;
  canvas.height = ch;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) return null;
  let data: Uint8ClampedArray;
  try {
    ctx.clearRect(0, 0, cw, ch);
    ctx.drawImage(img, 0, 0, cw, ch);
    data = ctx.getImageData(0, 0, cw, ch).data;
  } catch {
    return null; // not readable after all (an origin the URL did not give away)
  }
  let clear = 0;
  let ink = 0;
  let dark = 0;
  for (let i = 0; i < data.length; i += 4) {
    const a = data[i + 3];
    if (a < CLEAR_ALPHA) {
      clear++;
      continue;
    }
    ink += a;
    const luma = 0.2126 * LINEAR[data[i]] + 0.7152 * LINEAR[data[i + 1]] + 0.0722 * LINEAR[data[i + 2]];
    if (luma < DARK_LUMA) dark += a;
  }
  const pixels = cw * ch;
  return clear / pixels >= MIN_CLEAR && ink > 0 && dark / ink >= MIN_DARK_INK;
}

function mark(img: HTMLImageElement, plate: boolean) {
  img.classList.toggle("img-plate", plate);
  // the box's ratio, so the plate's padding comes out of the picture and not the layout
  const [w, h] = sizeOf(img);
  if (plate && w && h) img.style.setProperty("--plate-ratio", `${w} / ${h}`);
  else img.style.removeProperty("--plate-ratio");
}

function onLoad(e: Event) {
  const img = e.target;
  if (!(img instanceof HTMLImageElement) || !img.matches(CONTENT_IMAGES)) return;
  const src = img.currentSrc || img.src;
  if (!src || !sameOrigin(src)) return;
  let plate = verdicts.get(src);
  if (plate === undefined) {
    const v = needsPlate(img);
    if (v === null) return;
    plate = v;
    if (verdicts.size > 1000) verdicts.clear();
    verdicts.set(src, plate);
  }
  mark(img, plate);
}

/** Install the listener once, before anything is mounted (main.ts). */
export function installImagePlate(): void {
  document.addEventListener("load", onLoad, true);
}
