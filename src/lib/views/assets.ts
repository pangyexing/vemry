// A note's own images inside a rendered view (the translation's cells, a review card):
// renderNoteMarkdown emits them as <img data-asset="path"> with no src, and this fills
// them from the vault, one blob URL per path for the loader's life.
// An image with no size is 0px tall until it loads and then pushes everything under it
// down, so every image gets width/height attributes before its src — from the editor's
// size cache (livepreview imageDimsOf, shared with the editor and embeds), else from its
// file header (commands.rs image_sizes) — and with CSS's `height: auto` the attributes
// only reserve the aspect ratio: the text under the image does not move when it arrives.
import { api } from "$lib/api";
import { fileKind, mimeOf } from "$lib/stores/vault.svelte";
import { imageDimsOf, noteImageDims } from "$lib/editor/livepreview";

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

  function applyDims(img: HTMLImageElement, target: string): boolean {
    const d = imageDimsOf(target);
    if (d && !img.hasAttribute("width")) {
      img.width = d.w;
      img.height = d.h;
    }
    return !!d;
  }

  /** Read the headers of the targets no surface has sized yet, then size the images
   *  that are still waiting on them. */
  function prime(waiting: [HTMLImageElement, string][]) {
    const paths = [...new Set(waiting.map(([, t]) => t))];
    void api
      .imageSizes(paths)
      .then((sizes) => {
        sizes.forEach((s, i) => s && noteImageDims(paths[i], s[0], s[1]));
        for (const [img, t] of waiting) applyDims(img, t);
      })
      .catch(() => {});
  }

  function fill(node: HTMLElement) {
    // each image element is handled once: a mutation elsewhere in the view does not
    // re-read a failed file or stack another load listener on a pending one
    const seen = new WeakSet<HTMLImageElement>();
    const run = () => {
      const unsized: [HTMLImageElement, string][] = [];
      for (const img of node.querySelectorAll<HTMLImageElement>("img[data-asset]:not([src])")) {
        if (seen.has(img)) continue;
        seen.add(img);
        const target = img.dataset.asset ?? "";
        if (!applyDims(img, target) && target.includes("/") && fileKind(target) === "image") unsized.push([img, target]);
        img.addEventListener("load", () => noteImageDims(target, img.naturalWidth, img.naturalHeight), { once: true });
        void load(target).then((u) => {
          if (u && img.isConnected) img.src = u;
        });
      }
      if (unsized.length) prime(unsized);
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
