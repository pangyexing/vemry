import { t } from "$lib/i18n/i18n.svelte";
import { untrack } from "svelte";
import { api, emitClipTargetChanged, onClipTargetChanged } from "$lib/api";
import { windowLabel } from "$lib/window";
import { vault } from "./vault.svelte";
import { toast } from "./toast.svelte";
import { loadBucketMap, saveBucket } from "$lib/persist";

/** The "clip destination": one note pinned for clipping a selection, optionally with a
 *  destination section. Choose the target once and later a single click goes straight
 *  there — the destination is written on the button's label, so the behaviour is
 *  predictable. Persisted to localStorage per vault (a vault-relative path means nothing
 *  in another vault, so a vault switch restores that vault's own pin).
 *  AgentPanel (a browser-window selection) and ReadingView (a projection selection)
 *  share one destination. */
const CLIP_KEY = "vemry-clip-target";
type SavedTarget = { path: string; heading: string | null };

class ClipTargetStore {
  path = $state<string | null>(null);
  /** The destination section (the heading as plain text, without the #); null = the end of the note or the cursor. */
  heading = $state<string | null>(null);

  /** The button's short label: the note's name (without its directory or extension), and `note › section` when it has one. */
  get name(): string | null {
    if (!this.path) return null;
    const base = this.path.replace(/\.md$/, "").split("/").pop() || this.path;
    if (!this.heading) return base;
    const h = this.heading.length > 12 ? `${this.heading.slice(0, 12)}…` : this.heading;
    return `${base} › ${h}`;
  }

  clear() {
    this.path = null;
    this.heading = null;
  }

  /** Check the pinned destination is still alive before going straight there. A pin
   *  restored from localStorage (or deleted from outside) may point at a note that no
   *  longer exists — note_insert means "create and write" for a missing note, and
   *  without the check it would silently rebuild a phantom file and report success. When
   *  it is gone the pin is released with a message and the caller continues with the
   *  unpinned default destination. */
  ensureLive(): boolean {
    if (!this.path) return false;
    if (vault.notes.some((n) => n.id === this.path)) return true;
    toast.push(t("vaultStore.pinnedTargetGone", { path: this.path }), "error");
    this.clear();
    return false;
  }

  /** The no-false-kill version of ensureLive: the main window's notes mirror lags the
   *  real disk (a note just created by "create and clip into" in the toolbar's ▾ has to
   *  wait for the watcher broadcast plus a refreshNotes round trip), and pronouncing it
   *  dead from a stale list would release a pin just made and send the quote block to
   *  the default destination. When it is not in the list, the list is refreshed once
   *  before judging — the extra IPC only happens on the miss path. */
  async ensureLiveFresh(): Promise<boolean> {
    if (!this.path) return false;
    if (vault.notes.some((n) => n.id === this.path)) return true;
    await vault.refreshNotes().catch(() => {});
    return this.ensureLive();
  }

  /** Land a quote block in a given note; returns whether it landed (false = not one
   *  character was written, and the error has been reported).
   *  The dispatch and the section-fallback message live in vault.insertBlock (one
   *  contract shared with the quick-memo capture); this only adds what clipping itself
   *  needs: the wording of a successful receipt, and — with a sourceUrl — localising the
   *  images once it has landed. The backend rewrites literals against what is on disk,
   *  so the buffer path flushes first and localisation can only find the images once the
   *  quote block has reached disk. */
  async deliver(path: string, quote: string, sourceUrl?: string, heading: string | null = null): Promise<boolean> {
    const spot = heading ? `${path} › ${heading}` : path;
    let intoEditor = false;
    try {
      const r = await vault.insertBlock(path, quote, heading);
      intoEditor = r.intoEditor;
      if (!(heading && r.landed === "tail")) toast.push(t(intoEditor ? "app.insertedInto" : "app.clippedInto", intoEditor ? { path: spot } : { spot }));
      if (sourceUrl) {
        if (intoEditor) await vault.flushEditor?.();
        await api.clipLocalizeNote(path, sourceUrl);
      }
      return true;
    } catch (e) {
      toast.push(e instanceof Error ? e.message : String(e), "error");
      // A throw from the insertion means nothing landed; a throw from the flush or the localisation after a buffer landing does not affect the quote block that did land
      return intoEditor;
    }
  }
}

export const clipTarget = new ClipTargetStore();

// The pinned destination is written through and restored per vault (a module-level
// $effect needs its own root, as in vault.svelte.ts).
// On the first beat of a vault switch the vault's own record is restored before the
// write-through begins — otherwise the (new root, old path) combination would write the
// old vault's leftovers into the new vault's bucket (the same timing trap as
// vault-tree). The restore branch rewrites the effect's own dependency, so it reruns at
// once and the second beat writes through exactly what was restored, which is stable.
let restoredFor: string | null = null;
$effect.root(() => {
  $effect(() => {
    const root = vault.root;
    const path = clipTarget.path;
    const heading = clipTarget.heading;
    if (!root) return;
    if (restoredFor !== root) {
      restoredFor = root;
      const saved = loadBucketMap<SavedTarget>(CLIP_KEY)[root];
      clipTarget.path = saved?.path ?? null;
      clipTarget.heading = saved?.heading ?? null;
      return;
    }
    // Writing to disk and broadcasting belong to the main window alone: the chat window
    // and the toolbar are read-only mirrors, and the restore branch above gives them an
    // initial value after which they follow the main window's broadcast; they must not
    // become a second writer of the shared bucket (a bucket knows a root and not a
    // window, and if anyone could write it there would be no answer to "who set this
    // value")
    if (windowLabel !== "main") return;
    // recent is only the trimming allowlist: untrack keeps loadRecent from triggering a rewrite, and passing null while it is unloaded trims nothing
    saveBucket(CLIP_KEY, root, path ? { path, heading } : null, untrack(() => (vault.recentLoaded ? vault.recent : null)));
    // Cross-window sync: broadcast globally after writing through (the restore branch
    // returns above, so a stale value is never broadcast; once the restore rewrites the
    // dependency the effect reruns, and the second beat broadcasts exactly what was
    // restored)
    void emitClipTargetChanged({ path, heading }).catch(() => {});
  });
});

// A non-main window (the chat window, the clip toolbar) is a read-only mirror: a change
// to the pin is always relayed to the main window to write (a single writer), and the
// main window's persistence effect broadcasts it back — so both windows' labels agree
// in real time and no longer depend on the localStorage snapshot taken at attach time.
if (windowLabel !== "main") {
  void onClipTargetChanged((t) => {
    clipTarget.path = t.path;
    clipTarget.heading = t.heading;
  });
}

// Deleting invalidates it and renaming follows it (ensureLive only covers "when it is
// used", and a label still pointing at a deleted target would mislead — measured by
// hand: after deleting test3 the toolbar still said selection→test3). The hooks are
// only called in the main window, where file operations happen; clearing or rewriting it
// goes through the main window's persistence effect to disk and is broadcast, so the
// toolbar's label recovers immediately. A deletion from outside the application (rm in a
// terminal) is still covered by ensureLive.
vault.pathGone = (path) => {
  const p = clipTarget.path;
  if (p && (p === path || p.startsWith(path + "/"))) clipTarget.clear();
};
vault.pathMoved = (from, to) => {
  const p = clipTarget.path;
  if (!p) return;
  if (p === from) clipTarget.path = to;
  else if (p.startsWith(from + "/")) clipTarget.path = to + p.slice(from.length);
  // The heading is left alone: renaming or moving does not change a section's title
};
