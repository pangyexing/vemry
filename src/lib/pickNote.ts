// The two-stage "pick a note, then optionally pick a destination section" selector
// has three hosts: ⌘P's pick-note mode (CommandPalette), the browser panel's clip
// target (BrowserPanel) and the quick capture window's ⌘Enter (CaptureComposer).
// Their appearance differs entirely (a full-screen overlay, a 320px embedded panel,
// a strip under a text box), so the components cannot be shared; but the things below
// are one contract, and writing them a second time would let the hosts drift — so
// they are collected here.
import { api, type NoteSummary } from "$lib/api";
import { fuzzyFilter } from "$lib/commands/registry";
import { vault } from "$lib/stores/vault.svelte";

/** A note's fuzzy match key: title + path + aliases. The aliases have to be in the
 *  key, or a row found by alias would lead to a different note than the list shows. */
export const noteKey = (n: NoteSummary) => `${n.title} ${n.id} ${n.aliases.join(" ")}`;

/** The first stage: the notes matching the query, and the title of the "create" row
 *  that follows them — offered unless a note with exactly that title is among the
 *  matches (all of them, not the slice a host shows: a note ranked past the cut-off
 *  is still a note the user has). */
export function noteMatches(query: string): { notes: NoteSummary[]; create: string | null } {
  const notes = fuzzyFilter(vault.notes, query, noteKey);
  const q = query.trim();
  const create = q && !notes.some((n) => n.title.toLowerCase() === q.toLowerCase()) ? q : null;
  return { notes, create };
}

export interface PickHeading {
  text: string;
  level: number;
}

/** The notes as ⌘O lists them before anything is typed: the ones opened this session,
 *  most recent first, then the open tabs, then the rest by modification time. In notes
 *  mode the note on screen goes after the other recent ones (Enter then returns to the
 *  previous note); when picking a target it stays first — it is the likeliest target. */
export function notesByRecency(keepCurrentFirst: boolean): NoteSummary[] {
  const seen = new Set<string>();
  const out: NoteSummary[] = [];
  const cur = vault.currentPath;
  const push = (p: string | null) => {
    const n = p ? vault.noteById.get(p) : undefined;
    if (n && !seen.has(n.id)) {
      seen.add(n.id);
      out.push(n);
    }
  };
  if (keepCurrentFirst) push(cur);
  for (const p of vault.recentOpened) if (p !== cur) push(p);
  for (const tab of vault.tabs) if (tab.path !== cur) push(tab.path);
  push(cur);
  const rest = vault.notes.filter((n) => !seen.has(n.id)).sort((a, b) => b.mtime_ms - a.mtime_ms);
  return [...out, ...rest];
}

/** The second stage's section list is another projection of query(); it opens no new read path. */
export async function fetchHeadings(path: string): Promise<PickHeading[]> {
  const hits = await api.queryBlocks({ kind: "heading", note: path });
  return hits.map((h) => ({
    text: h.block.content,
    level: h.block.kind.type === "heading" ? h.block.kind.level : 1,
  }));
}

/** Where the "create 'q'" row lands: an existing file of that name first, otherwise a
 *  note is really created. A note whose title has come apart from its file name cannot
 *  be caught by the caller's suppression list, and a bare noteCreate would conjure up a
 *  "title 1.md" — this same-name fallback is what plugs that hole. The sanitising and
 *  deduplication rules of creating a file still live in Rust alone. */
export async function resolveOrCreateNote(title: string): Promise<string> {
  const fname = `${title.toLowerCase()}.md`;
  const existing = vault.notes.find((n) => n.id.toLowerCase() === fname);
  return existing ? existing.id : await api.noteCreate(title);
}
