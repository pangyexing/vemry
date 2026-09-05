// The built-in browser's state (Svelte 5 runes). The source of truth for the tabs,
// the active tab and the shape is Rust's BrowserState, and this is the mirror
// assembled from browser://event pushes plus the browser_state backstop snapshot.
// The panel's width belongs to the main window's frontend alone and is persisted with
// the sidebar widths (vault.browserWidth). Whether the panel is on screen is not
// state at all: it is there exactly while it has tabs — closing the last tab closes
// it, and opening a page brings it back (a hidden-but-alive set of tabs would need a
// switch of its own, which twice turned out to be a button nobody could place).
import {
  api,
  type AdblockStatusView,
  type BookmarkView,
  type BrowserEvent,
  type BrowserShape,
  type BrowserTabView,
} from "$lib/api";
import { isMainWindow } from "$lib/window";
import { vault } from "$lib/stores/vault.svelte";

/** The docked panel column's width floor (logical px): narrower and most sites render their mobile layout in a strip. */
export const PANEL_MIN = 360;
/** The panel column's width until the user drags its sash. */
export const PANEL_DEFAULT = 440;
/** How much of the main window the panel may take at most: the rest keeps the file tree, the content area and the research panel usable (matches Rust's former MIN_MAIN_VISIBLE). */
export const MAIN_MIN_BESIDE_PANEL = 804;

class BrowserStore {
  tabs = $state<BrowserTabView[]>([]);
  active = $state<string | null>(null);
  shape = $state<BrowserShape>("docked");
  /** The ad-blocking pipeline's state (broadcast at startup; null = nothing received yet). */
  adblock = $state<AdblockStatusView | null>(null);
  /** The bookmarks mirror (the source of truth is bookmarks.json, and a change replaces it wholesale through bookmarks_changed). */
  bookmarks = $state<BookmarkView[]>([]);

  /** The docked panel column's width (logical px): the persisted value on its floor, or the default. */
  get panelWidth(): number {
    return Math.max(PANEL_MIN, vault.browserWidth ?? PANEL_DEFAULT);
  }
  set panelWidth(w: number) {
    vault.browserWidth = w;
  }
  /** A full-window overlay is open (the palette, a dialog, the lightbox, a sash drag): the
   *  native tab is hidden by Rust meanwhile, and the panel paints a placeholder. Written by
   *  App from its overlay states. */
  veiled = $state(false);
  /** The image lightbox is open (an overlay outside any component, so it reports here). */
  lightboxOpen = $state(false);

  /** The panel column is on screen (docked shape): exactly while there are tabs. */
  get panelOpen(): boolean {
    return this.tabs.length > 0;
  }

  /** The panel column is on screen in the main window: the one predicate every layout decision keys on. */
  get docked(): boolean {
    return this.panelOpen && this.shape === "docked";
  }

  /** The tab shown: the chrome shows and acts on it; null when there are no tabs. */
  get current(): BrowserTabView | null {
    return this.tabs.find((w) => w.label === this.active) ?? this.tabs[0] ?? null;
  }

  /** How much of the main window's right edge the native panel covers (logical px):
   *  the popups, the toasts and the sash budget keep out of it, since a native view
   *  covers any DOM underneath. */
  get panelInset(): number {
    return isMainWindow && this.docked ? this.panelWidth : 0;
  }

  /** Patch fields onto the tab matching label (an immutable update; a no-op when the label does not match). */
  private patch(label: string, p: Partial<BrowserTabView>) {
    this.tabs = this.tabs.map((w) => (w.label === label ? { ...w, ...p } : w));
  }

  handle(e: BrowserEvent) {
    if (e.type === "opened") {
      if (!this.tabs.some((w) => w.label === e.label)) {
        this.tabs = [...this.tabs, { label: e.label, url: e.url, title: "" }];
      }
      this.active = e.label;
    } else if (e.type === "navigated") {
      this.patch(e.label, { url: e.url });
    } else if (e.type === "title_changed") {
      this.patch(e.label, { title: e.title });
    } else if (e.type === "activated") {
      this.active = e.label;
    } else if (e.type === "closed") {
      this.tabs = this.tabs.filter((w) => w.label !== e.label);
      if (this.active === e.label) this.active = null;
    } else if (e.type === "shape_changed") {
      this.shape = e.shape;
    } else if (e.type === "adblock_status") {
      this.adblock = e.status;
    } else if (e.type === "bookmarks_changed") {
      this.bookmarks = e.bookmarks;
    }
  }

  /** Whether the current URL is bookmarked (an exact string match — both sides store the registry's entry.url, so the rule is the same). */
  isBookmarked(url: string): boolean {
    return this.bookmarks.some((b) => b.url === url);
  }

  /** The startup backstop: events emitted before listen was registered cannot be recovered, so the full state is fetched once. */
  async refresh() {
    try {
      const [s, bookmarks] = await Promise.all([api.browserState(), api.bookmarkList()]);
      this.tabs = s.tabs;
      this.active = s.active;
      this.shape = s.shape;
      this.bookmarks = bookmarks;
    } catch {
      /* a failure to fetch the browser state blocks no local feature */
    }
  }
}

export const browser = new BrowserStore();
