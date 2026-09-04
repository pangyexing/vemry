// Block citations in an AI answer: `[[path#^id]]` (stable, lands on disk) and
// `[[path#bN]]` (positional — the index's temporary id, valid only while the note
// has not changed and never written to a note). This module holds the shared
// grammar, the per-index-version block cache the hover card, the click and the
// export all read, and the export funnel that turns positional citations into
// stable ids (`block_ensure_id` writes ` ^id` into the cited note; a citation
// that cannot be pinned degrades to the note alone).
import { api, type BlockHit } from "$lib/api";
import { stripAnchor } from "$lib/fields";

export const POSITIONAL_RE = /^b\d+$/;
/** `[[note#bN]]` / `[[note#bN|alias]]` — the transient form, with the note as the model wrote it. */
export const TRANSIENT_CITE_RE = /\[\[([^\]\n#|]+)#(b\d+)((?:\|[^\]\n]*)?)\]\]/g;
/** `[[note#^id]]` / `[[note#Heading]]` — a stable citation, whose note part the export normalizes. */
const STABLE_CITE_RE = /\[\[([^\]\n#|]+)#((?!b\d+[\]|])[^\]\n|]+)((?:\|[^\]\n]*)?)\]\]/g;

export function parseRefTarget(target: string): { note: string; frag: string } {
  const h = target.indexOf("#");
  return h < 0 ? { note: target.trim(), frag: "" } : { note: target.slice(0, h).trim(), frag: target.slice(h + 1).trim() };
}

const cache = new Map<string, { version: number; promise: Promise<BlockHit[]> }>();

/** The blocks of one note (a path), cached per index version. */
export function blocksOf(path: string, version: number): Promise<BlockHit[]> {
  const hit = cache.get(path);
  if (hit && hit.version === version) return hit.promise;
  const promise = api.queryBlocks({ note: path, limit: 5000 }).catch(() => [] as BlockHit[]);
  cache.set(path, { version, promise });
  return promise;
}

/** The block a fragment names: `^id` or `bN` by local id. With a guard (the block's
 *  content when it was cited) a mismatch falls back to the one block that still has
 *  that content — the paragraph moved — and otherwise to the id's current holder, which
 *  is still the most useful landing point; null only when the id is gone. */
export function findBlock(hits: BlockHit[], frag: string, guard?: string): BlockHit | null {
  const id = frag.startsWith("^") ? frag.slice(1) : frag;
  const byId = hits.find((h) => h.block.local_id === id) ?? null;
  if (byId && (!guard || byId.block.content === guard)) return byId;
  if (guard) {
    const same = hits.filter((h) => h.block.content === guard);
    if (same.length === 1) return same[0];
  }
  return byId;
}

export interface Materialized {
  text: string;
  /** Citations that received a stable id. */
  minted: number;
  /** Citations that could not be pinned and now point to the note alone. */
  failed: number;
}

/** Turn every `[[note#bN]]` in text into `[[path#^id]]`, minting ids in the cited
 *  notes. guards supplies the content captured when the answer arrived (so a block that
 *  moved since is still the right one); from is the note the links are resolved from. */
export async function materializeCitations(
  text: string,
  guards: (note: string, frag: string) => string | undefined,
  from?: string,
  version?: number,
): Promise<Materialized> {
  // The link's note part: the note's stem when the stem alone resolves to it (the form
  // the editor's own links take), else the path without its extension
  const names = new Map<string, Promise<string>>();
  const shownName = (path: string) => {
    let name = names.get(path);
    if (!name) {
      name = (async () => {
        const bare = path.replace(/\.md$/i, "");
        const stem = bare.slice(bare.lastIndexOf("/") + 1);
        if (stem === bare) return bare;
        const r = await api.resolveLink(stem, from).catch(() => null);
        return r === path ? stem : bare;
      })();
      names.set(path, name);
    }
    return name;
  };
  const jobs = new Map<string, Promise<{ path: string; frag: string } | null>>();
  const pin = (note: string, bN: string) => {
    const key = `${note}#${bN}`;
    let job = jobs.get(key);
    if (!job) {
      job = (async () => {
        const path = (await api.resolveLink(note, from)) ?? note;
        if (version != null) {
          // A heading is its own anchor: link it by its text instead of minting an
          // id it cannot carry
          const hit = findBlock(await blocksOf(path, version), bN, guards(note, bN));
          if (hit?.block.kind.type === "heading") return { path, frag: stripAnchor(hit.block.content) };
        }
        const id = await api.blockEnsureId(path, bN, guards(note, bN));
        return { path, frag: `^${id.replace(/^\^/, "")}` };
      })().catch(() => null);
      jobs.set(key, job);
    }
    return job;
  };
  const parts: (string | Promise<string>)[] = [];
  let last = 0;
  let minted = 0;
  let failed = 0;
  TRANSIENT_CITE_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = TRANSIENT_CITE_RE.exec(text))) {
    parts.push(text.slice(last, m.index));
    last = m.index + m[0].length;
    const [, rawNote, bN, alias] = m;
    const note = rawNote.trim();
    parts.push(
      pin(note, bN).then(async (r) => {
        if (r) {
          minted++;
          return `[[${await shownName(r.path)}#${r.frag}${alias}]]`;
        }
        failed++;
        return `[[${note}${alias}]]`;
      }),
    );
  }
  parts.push(text.slice(last));
  const resolved = await Promise.all(parts);
  // The stable citations the model wrote itself (`[[path#^id]]`, `[[path#Heading]]`)
  // get the same note part as the minted ones, so one answer does not link the same
  // note two ways
  const stable: (string | Promise<string>)[] = [];
  const joined = resolved.join("");
  last = 0;
  STABLE_CITE_RE.lastIndex = 0;
  while ((m = STABLE_CITE_RE.exec(joined))) {
    stable.push(joined.slice(last, m.index));
    last = m.index + m[0].length;
    const [whole, rawNote, frag, alias] = m;
    const note = rawNote.trim();
    stable.push(
      (async () => {
        const path = await api.resolveLink(note, from).catch(() => null);
        return path ? `[[${await shownName(path)}#${frag}${alias}]]` : whole;
      })(),
    );
  }
  stable.push(joined.slice(last));
  return { text: (await Promise.all(stable)).join(""), minted, failed };
}
