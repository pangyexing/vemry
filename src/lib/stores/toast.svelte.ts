// Transient notification stack (the app's own notices, scripts' vemry.toast, script
// errors). An info toast dismisses itself after a reading time scaled to its length; an
// error toast stays until dismissed, so failures in background workflows are never missed.

export interface ToastItem {
  id: number;
  message: string;
  level: "info" | "error";
  /** How many times the same text repeated (a workflow that fails on every autosave, say) — collapsed rather than flooding the screen. */
  count: number;
}

/** A base reading time plus an allowance per character, capped: a four-word confirmation is
 *  gone before it nags, a path with a sentence around it stays long enough to be read. */
const DISMISS_BASE_MS = 3500;
const DISMISS_PER_CHAR_MS = 40;
const DISMISS_MAX_MS = 9000;
/** A toast the pointer has just left gets a short grace, not its whole time again. */
const RELEASE_GRACE_MS = 1500;
/** Errors stick around, and without a cap a script that fails on every event would paper over the viewport; past this the oldest is dropped. */
const MAX_ITEMS = 6;

function readingTime(message: string): number {
  return Math.min(DISMISS_MAX_MS, DISMISS_BASE_MS + DISMISS_PER_CHAR_MS * message.length);
}

class ToastStore {
  items = $state<ToastItem[]>([]);
  #next = 1;
  #timers = new Map<number, ReturnType<typeof setTimeout>>();
  /** The toasts the pointer is over: their clock stays stopped, a repeat included. */
  #held = new Set<number>();

  push(message: string, level: "info" | "error" = "info") {
    const dup = this.items.find((t) => t.message === message && t.level === level);
    if (dup) {
      dup.count += 1;
      this.#arm(dup.id, readingTime(message)); // a repeat is news too: its clock restarts
      return;
    }
    const id = this.#next++;
    this.items = [...this.items, { id, message, level, count: 1 }].slice(-MAX_ITEMS);
    this.#arm(id, readingTime(message));
  }

  /** The pointer over a toast holds it: it is being read, or its text is being selected to copy. */
  hold(id: number) {
    this.#held.add(id);
    this.#disarm(id);
  }

  release(id: number) {
    this.#held.delete(id);
    this.#arm(id, RELEASE_GRACE_MS);
  }

  dismiss(id: number) {
    this.#held.delete(id);
    this.#disarm(id);
    this.items = this.items.filter((t) => t.id !== id);
  }

  /** Start (or restart) the clock of the item with `id`; a held toast, an error toast, or an id no longer shown gets none. */
  #arm(id: number, ms: number) {
    if (this.#held.has(id)) return;
    this.#disarm(id);
    if (this.items.find((t) => t.id === id)?.level === "info") this.#timers.set(id, setTimeout(() => this.dismiss(id), ms));
  }

  #disarm(id: number) {
    const t = this.#timers.get(id);
    if (t !== undefined) clearTimeout(t);
    this.#timers.delete(id);
  }
}

export const toast = new ToastStore();
