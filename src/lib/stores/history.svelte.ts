// The note-history dialog's one piece of shared state: which note it is open on.
// Everything else (the kept states, the selected one, its diff) is the dialog's own,
// fetched when it opens — the history lives on disk under `.vemry/history/`, and
// the dialog is a window onto it, not a cache of it.

class HistoryStore {
  /** The note the dialog is open on; null = closed. */
  path = $state<string | null>(null);

  open(path: string) {
    this.path = path;
  }

  close() {
    this.path = null;
  }
}

export const history = new HistoryStore();
