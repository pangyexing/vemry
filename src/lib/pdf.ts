// pdf.js, loaded once: the document cache (a PDF is parsed once for the preview
// and the translation view alike) and the page renderer both preview and the
// translation rows draw through. The worker is the bundled module worker, served
// from the app's own origin — the CSP's script-src 'self' covers it.
import * as pdfjs from "pdfjs-dist";
import type { PDFDocumentProxy, PDFPageProxy } from "pdfjs-dist";
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
