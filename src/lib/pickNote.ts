// The two-stage "pick a note, then optionally pick a destination section" selector
// now has two hosts: ⌘P's pick-note mode (CommandPalette) and the clip toolbar's ▾
// (ClipBar). Their appearance differs entirely (a full-screen overlay versus a 320px
// embedded panel), so the components cannot be shared; but the three things below are
// one contract, and writing them a second time would let the two drift — so they are
// collected here.
import { api, type NoteSummary } from "$lib/api";
import { vault } from "$lib/stores/vault.svelte";

/** A note's fuzzy match key: title + path + aliases. The aliases have to be in the
 *  key, or a row found by alias would lead to a different note than the list shows. */
export const noteKey = (n: NoteSummary) => `${n.title} ${n.id} ${n.aliases.join(" ")}`;

export interface PickHeading {
  text: string;
  level: number;
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
