// pdf.js, loaded once: the document cache (a PDF is parsed once for the preview
// and the translation view alike) and the page renderer both preview and the
// translation rows draw through. The worker is the bundled module worker, served
// from the app's own origin — the CSP's script-src 'self' covers it.
import * as pdfjs from "pdfjs-dist";
import type { PDFDocumentProxy, PDFPageProxy, TextItem } from "pdfjs-dist/types/src/display/api";
import workerSrc from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { api } from "$lib/api";

pdfjs.GlobalWorkerOptions.workerSrc = workerSrc;

// WebKit has no async iteration over a ReadableStream, which pdf.js's text
// extraction relies on (`for await` over the content stream); the reader loop is
// what the iterator is elsewhere.
const streamProto = ReadableStream.prototype as unknown as Record<symbol, unknown>;
if (typeof streamProto[Symbol.asyncIterator] !== "function") {
  streamProto[Symbol.asyncIterator] = async function* (this: ReadableStream) {
    const reader = this.getReader();
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) return;
        yield value;
      }
    } finally {
      reader.releaseLock();
    }
  };
}

/** Parsed documents by vault path, a few at a time (a PDF is tens of MB parsed). */
const DOC_CAP = 4;
const docs = new Map<string, Promise<PDFDocumentProxy>>();

export type { PDFDocumentProxy, PDFPageProxy };

/** The parsed document of a vault file; the bytes are read once per path. */
export function loadPdf(path: string): Promise<PDFDocumentProxy> {
  let p = docs.get(path);
  if (!p) {
    p = api.fileReadBinary(path).then((buf) => pdfjs.getDocument({ data: new Uint8Array(buf) }).promise);
    p.catch(() => docs.delete(path));
    docs.set(path, p);
    while (docs.size > DOC_CAP) {
      const oldest = docs.keys().next().value;
      if (oldest == null) break;
      const gone = docs.get(oldest);
      docs.delete(oldest);
      void gone?.then((d) => d.loadingTask.destroy()).catch(() => {});
    }
  }
  return p;
}

/** Forget a document (the file changed or went away). */
export function dropPdf(path: string) {
  const p = docs.get(path);
  docs.delete(path);
  void p?.then((d) => d.loadingTask.destroy()).catch(() => {});
}

/** A rectangle on a page, as fractions of the page's width and height. */
export interface PageRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** Where `needle` sits on `page`: the boxes of the text items it spans, as
 *  fractions of the page. Whitespace is ignored on both sides (extraction and
 *  pdf.js break lines and words differently); when the whole text is not found
 *  as one run, its head and its tail are looked for and everything between them
 *  is taken. Empty when nothing matches. */
export async function textRects(page: PDFPageProxy, needle: string): Promise<PageRect[]> {
  const squash = (s: string) => s.replace(/\s+/g, "");
  const target = squash(needle);
  if (!target) return [];
  const content = await page.getTextContent();
  const base = page.getViewport({ scale: 1 });
  const items = content.items.filter((i): i is TextItem => "str" in i);
  const parts = items.map((i) => squash(i.str));
  const flat = parts.join("");
  let at = flat.indexOf(target);
  let end = at + target.length;
  if (at < 0) {
    const head = target.slice(0, 24);
    const tail = target.slice(-24);
    at = flat.indexOf(head);
    const tailAt = at < 0 ? -1 : flat.indexOf(tail, at);
    if (at < 0 || tailAt < 0) return [];
    end = tailAt + tail.length;
  }
  const rects: PageRect[] = [];
  let pos = 0;
  for (let k = 0; k < items.length; k++) {
    const s = pos;
    const e = pos + parts[k].length;
    pos = e;
    if (e <= s || e <= at || s >= end) continue;
    const it = items[k];
    const [, , c, d, x, y] = it.transform as number[];
    const h = it.height || Math.hypot(c, d);
    rects.push({ left: x / base.width, top: (base.height - y - h) / base.height, width: it.width / base.width, height: h / base.height });
  }
  return rects;
}

/** Draw `page` into `canvas` at `width` CSS pixels (sharp on a retina screen), and
 *  lay a selectable text layer over it when `textLayer` is given. Returns the CSS
 *  height. A render still running for the same canvas is cancelled by the caller
 *  keeping one task per canvas. */
export async function renderPage(
  page: PDFPageProxy,
  canvas: HTMLCanvasElement,
  width: number,
  textLayer?: HTMLElement,
): Promise<number> {
  const base = page.getViewport({ scale: 1 });
  const scale = width / base.width;
  const viewport = page.getViewport({ scale });
  const dpr = Math.max(1, window.devicePixelRatio || 1);
  canvas.width = Math.floor(viewport.width * dpr);
  canvas.height = Math.floor(viewport.height * dpr);
  canvas.style.width = `${Math.floor(viewport.width)}px`;
  canvas.style.height = `${Math.floor(viewport.height)}px`;
  const ctx = canvas.getContext("2d");
  if (!ctx) return viewport.height;
  await page.render({ canvas, canvasContext: ctx, viewport, transform: [dpr, 0, 0, dpr, 0, 0] }).promise;
  if (textLayer) {
    textLayer.replaceChildren();
    textLayer.style.setProperty("--scale-factor", String(scale));
    textLayer.style.width = canvas.style.width;
    textLayer.style.height = canvas.style.height;
    const layer = new pdfjs.TextLayer({ textContentSource: page.streamTextContent(), container: textLayer, viewport });
    await layer.render();
  }
  return viewport.height;
}
