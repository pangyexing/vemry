// A note's own images inside a rendered view (the translation's cells, a review card):
// renderNoteMarkdown emits them as <img data-asset="path"> with no src, and this fills
// them from the vault, one blob URL per path for the loader's life.
// An image with no size is 0px tall until it loads and then pushes everything under it
// down, so the intrinsic size of every image seen is kept, and a later mount (another
// cell showing the same figure, the view reopened, a re-render) gets width/height
// attributes before its src: with CSS's `height: auto` the attributes only reserve the
// aspect ratio, and the text under the image does not move when it arrives.
import { api } from "$lib/api";
import { fileKind, mimeOf } from "$lib/stores/vault.svelte";

/** Path → intrinsic size, for the session (a figure's size does not change under a view). */
const dims = new Map<string, { w: number; h: number }>();

export interface VaultImages {
  /** The Svelte action for a rendered container: `use:images.fill`. */
  fill: (node: HTMLElement) => { destroy: () => void };
  /** Revoke the blob URLs (the owning component's teardown). */
  dispose: () => void;
}

export function vaultImages(): VaultImages {
  const urls = new Map<string, Promise<string | null>>();

  function load(target: string): Promise<string | null> {
    // As written, when it names a path; a bare file name has nothing to resolve against here
    if (!target.includes("/") || fileKind(target) !== "image") return Promise.resolve(null);
    let p = urls.get(target);
    if (!p) {
      p = api
        .fileReadBinary(target)
        .then((buf) => URL.createObjectURL(new Blob([buf], { type: mimeOf(target) })))
        .catch(() => {
          // one transient read failure should not nail this image as "missing" for the loader's life
          urls.delete(target);
          return null;
        });
      urls.set(target, p);
    }
    return p;
  }

  function sized(img: HTMLImageElement, target: string) {
    const d = dims.get(target);
    if (d) {
      img.width = d.w;
      img.height = d.h;
      return;
    }
    img.addEventListener(
      "load",
      () => {
        if (img.naturalWidth > 0 && img.naturalHeight > 0) dims.set(target, { w: img.naturalWidth, h: img.naturalHeight });
      },
      { once: true },
    );
  }

  function fill(node: HTMLElement) {
    const run = () => {
      for (const img of node.querySelectorAll<HTMLImageElement>("img[data-asset]:not([src])")) {
        const target = img.dataset.asset ?? "";
        sized(img, target);
        void load(target).then((u) => {
          if (u && img.isConnected) img.src = u;
        });
      }
    };
    run();
    const mo = new MutationObserver(run);
    mo.observe(node, { childList: true, subtree: true });
    return { destroy: () => mo.disconnect() };
  }

  function dispose() {
    for (const p of urls.values()) void p.then((u) => u && URL.revokeObjectURL(u));
    urls.clear();
  }

  return { fill, dispose };
}
