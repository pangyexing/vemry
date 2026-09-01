// Transient notification stack (scripts' vemry.toast, script errors). Info
// toasts auto-dismiss; error toasts stay until clicked so failures in
// background workflows are never missed.

export interface ToastItem {
  id: number;
  message: string;
  level: "info" | "error";
  /** How many times the same text repeated (a workflow that fails on every autosave, say) — collapsed rather than flooding the screen. */
  count: number;
}

const AUTO_DISMISS_MS = 4000;
/** Errors stick around, and without a cap a script that fails on every event would paper over the viewport; past this the oldest is dropped. */
const MAX_ITEMS = 6;

class ToastStore {
  items = $state<ToastItem[]>([]);
  #next = 1;

  push(message: string, level: "info" | "error" = "info") {
    const dup = this.items.find((t) => t.message === message && t.level === level);
    if (dup) {
      dup.count += 1;
      return;
    }
    const id = this.#next++;
    this.items = [...this.items, { id, message, level, count: 1 }].slice(-MAX_ITEMS);
    if (level === "info") {
      setTimeout(() => this.dismiss(id), AUTO_DISMISS_MS);
    }
  }

  dismiss(id: number) {
    this.items = this.items.filter((t) => t.id !== id);
  }
}

export const toast = new ToastStore();
