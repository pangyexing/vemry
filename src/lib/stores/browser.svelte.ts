// The built-in browser's state (Svelte 5 runes). The source of truth is Rust's
// BrowserState, and this is the mirror assembled from browser://event pushes plus the
// browser_state backstop snapshot.
import {
  api,
  type AdblockStatusView,
  type BookmarkView,
  type BrowserEvent,
  type BrowserWindowView,
} from "$lib/api";
import { vault } from "$lib/stores/vault.svelte";

/** Only the docked shape brings the research panel out: the page card (clip, add to
 *  context) sits in the main window's right column, and it only means something beside
 *  a docked window; a detached window (batch D's default) has its clipping entry point
 *  in its own bottom toolbar and does not move the main window's layout — looking
 *  something up must not interrupt the writing in progress.
 *
 *  `label` = which window this is going to land in; when it is not in the registry yet
 *  (a newly opened window's Opened event may not have arrived) or the caller knowingly
 *  passed nothing for a new window, it falls back to settings.browser_docked, the same
 *  source the window was built from. Both callers (⌘P's "open the browser" and ⌘L's
 *  return) share this one verdict, so the fallback rule is not written twice. */
export async function revealPanelIfDocked(label?: string | null): Promise<void> {
  const known = label ? browser.windows.find((w) => w.label === label) : undefined;
  const docked =
    known?.docked ?? (await api.settingsGet().then((s) => s.browser_docked).catch(() => false));
  if (!docked) return;
  vault.rightOpen = true;
  vault.rightTab = "agent";
}

class BrowserStore {
  windows = $state<BrowserWindowView[]>([]);
  active = $state<string | null>(null);
  /** The ad-blocking pipeline's state (broadcast at startup; null = nothing received yet). */
  adblock = $state<AdblockStatusView | null>(null);
  /** The bookmarks mirror (the source of truth is bookmarks.json, and a change replaces it wholesale through bookmarks_changed). */
  bookmarks = $state<BookmarkView[]>([]);

  /** The window the control panel shows and acts on: the one focused last, or null when there are no windows left. */
  get current(): BrowserWindowView | null {
    return this.windows.find((w) => w.label === this.active) ?? this.windows[0] ?? null;
  }

  /** How much of the main window's right edge a docked browser window covers (logical
   *  px). With the main window full-screen or against the screen's right edge, the
   *  child window can only lie over it, and App's layout gives way with that much
   *  padding-right. */
  get overlayInset(): number {
    return this.windows.reduce((m, w) => (w.docked && w.inset > m ? w.inset : m), 0);
  }

  /** Patch fields onto the window matching label (an immutable update; a no-op when the label does not match). */
  private patch(label: string, p: Partial<BrowserWindowView>) {
    this.windows = this.windows.map((w) => (w.label === label ? { ...w, ...p } : w));
  }

  handle(e: BrowserEvent) {
    if (e.type === "opened") {
      if (!this.windows.some((w) => w.label === e.label)) {
        this.windows = [
          ...this.windows,
          { label: e.label, url: e.url, title: "", docked: e.docked, inset: 0 },
        ];
      }
      this.active = e.label;
    } else if (e.type === "navigated") {
      this.patch(e.label, { url: e.url });
    } else if (e.type === "title_changed") {
      this.patch(e.label, { title: e.title });
    } else if (e.type === "focused") {
      this.active = e.label;
    } else if (e.type === "dock_changed") {
      this.patch(e.label, { docked: e.docked });
    } else if (e.type === "dock_layout") {
      this.patch(e.label, { inset: e.inset });
    } else if (e.type === "closed") {
      this.windows = this.windows.filter((w) => w.label !== e.label);
      if (this.active === e.label) this.active = this.windows[0]?.label ?? null;
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
      this.windows = s.windows;
      this.active = s.active;
      this.bookmarks = bookmarks;
    } catch {
      /* a failure to fetch the browser state blocks no local feature */
    }
  }
}

export const browser = new BrowserStore();
