// The "where a citation landed" cache: URL → the relative path of a clips/ note. The
// clipped badge on a source card, and a citation marker click preferring to open the
// local note, both rely on it; the source of truth is the `source:` frontmatter of a
// clips/ note, and this is only a mirror looked up on demand.
import { api } from "$lib/api";

class ClipsStore {
  private map = $state<Map<string, string>>(new Map());

  pathFor(url: string): string | null {
    return this.map.get(url) ?? null;
  }

  /** Look several up at once and merge the results. A URL that misses takes no placeholder — clipping it later can still be found. */
  async lookup(urls: string[]) {
    const todo = [...new Set(urls.filter((u) => u && !this.map.has(u)))];
    if (!todo.length) return;
    try {
      const found = await api.clipsLookup(todo);
      const entries = Object.entries(found);
      if (entries.length) {
        const next = new Map(this.map);
        for (const [url, path] of entries) next.set(url, path);
        this.map = next;
      }
    } catch {
      /* no vault open and the like — a missing badge is fine and nothing bothers the user */
    }
  }

  /** Registered immediately after a successful clip (without waiting for the next lookup). */
  markClipped(url: string, path: string) {
    const next = new Map(this.map);
    next.set(url, path);
    this.map = next;
  }

  /** Switching vaults: the paths belong to the old one, so everything is void. */
  reset() {
    this.map = new Map();
  }
}

export const clips = new ClipsStore();
