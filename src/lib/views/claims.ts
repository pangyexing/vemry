// Claim ↔ evidence join. `supports` targets are raw wikilink text
// ("Note#^c1"); the note part resolves through the same resolver the index
// uses (aliases, shortest-path) via the resolve_link IPC, so this join is
// always consistent with what the outline panel's links and the agent see.
import { api, type BlockHit } from "$lib/api";
import { plainText } from "$lib/markdown";

export interface ClaimWithEvidence {
  claim: BlockHit;
  /** The evidence supporting it (the same objects as in the evidence array, each carrying its own text) */
  evidence: EvidenceWithClaims[];
  /** The card's text: calloutText is stripped once here rather than recomputed in
   *  the template and the filter key every time (the same rule as TaskRow.text) */
  text: string;
}

export interface EvidenceLink {
  /** raw wikilink target as written, e.g. "Sleep#^c1" */
  target: string;
  /** note id the note part resolves to, null when dangling */
  resolvedNote: string | null;
  /** matched claim block — needs the target note to hold a claim with that ^id */
  claim: BlockHit | null;
}

export interface EvidenceWithClaims {
  evidence: BlockHit;
  links: EvidenceLink[];
  /** The card's text: stripped once, like ClaimWithEvidence.text */
  text: string;
}

export async function loadClaimGraph(): Promise<{ claims: ClaimWithEvidence[]; evidence: EvidenceWithClaims[] }> {
  const [claimHits, evidenceHits] = await Promise.all([
    api.queryBlocks({ kind: "claim", limit: 2000 }),
    api.queryBlocks({ kind: "evidence", limit: 2000 }),
  ]);

  // only explicitly-^id'd claims are addressable by evidence
  const claimByKey = new Map<string, BlockHit>();
  for (const c of claimHits) {
    if (c.block.explicit_id) claimByKey.set(`${c.note}::${c.block.local_id}`, c);
  }

  const cache = new Map<string, Promise<string | null>>();
  const resolve = (target: string, from: string) => {
    const key = `${from}|${target}`;
    let p = cache.get(key);
    if (!p) {
      p = api.resolveLink(target, from);
      cache.set(key, p);
    }
    return p;
  };

  const evidence: EvidenceWithClaims[] = await Promise.all(
    evidenceHits.map(async (ev) => {
      const k = ev.block.kind;
      // The parser does not deduplicate: writing the same target twice produces duplicate links (which a keyed each would blow up on)
      const supports = k.type === "evidence" ? [...new Set(k.supports)] : [];
      const links: EvidenceLink[] = await Promise.all(
        supports.map(async (target) => {
          // The note part stops at the first # (resolve_target does not understand
          // "Note#Heading"), and a block reference is only recognised as a trailing
          // #^id; an empty note part is a reference within the same note, which Rust
          // resolves as from
          const hash = target.indexOf("#");
          const notePart = hash >= 0 ? target.slice(0, hash) : target;
          const refIdx = target.lastIndexOf("#^");
          const ref = refIdx >= 0 ? target.slice(refIdx + 2) : null;
          const resolvedNote = await resolve(notePart, ev.note);
          const claim = resolvedNote && ref ? (claimByKey.get(`${resolvedNote}::${ref}`) ?? null) : null;
          return { target, resolvedNote, claim };
        }),
      );
      return { evidence: ev, links, text: calloutText(ev.block.content) };
    }),
  );

  const evidenceOfClaim = new Map<BlockHit, EvidenceWithClaims[]>();
  for (const ev of evidence) {
    for (const l of ev.links) {
      if (!l.claim) continue;
      const arr = evidenceOfClaim.get(l.claim) ?? [];
      // Two differently written targets (an alias and a path) may resolve to the same claim, so one piece of evidence is attached once
      if (!arr.includes(ev)) arr.push(ev);
      evidenceOfClaim.set(l.claim, arr);
    }
  }
  const claims: ClaimWithEvidence[] = claimHits.map((c) => ({
    claim: c,
    evidence: evidenceOfClaim.get(c) ?? [],
    text: calloutText(c.block.content),
  }));
  return { claims, evidence };
}

/** Callout body for display: `> [!claim]` marker, trailing `^id` and the
 *  `supports:[[...]]` machine-readable tail stripped, then folded to plain
 *  text (links/emphasis) via plainText — a projection card should not show the
 *  source's markers.
 *  supports is matched on the original `[[...]]` shape and therefore has to be
 *  stripped before plainText folds the wikilinks; the order cannot be swapped. */
export function calloutText(content: string): string {
  return plainText(
    content
      .replace(/^(?:>\s*)?\[!\w+\]\s*/, "")
      .replace(/\s*\^[A-Za-z0-9_-]+\s*$/, "")
      .replace(/\bsupports:\s*(?:\[\[[^\]]*\]\][,，\s]*)+/g, ""),
  );
}

/** Group hits by their note, preserving hit order. */
export function groupByNote<T>(items: T[], noteOf: (item: T) => { note: string; note_title: string }): {
  note: string;
  title: string;
  items: T[];
}[] {
  const out: { note: string; title: string; items: T[] }[] = [];
  const byNote = new Map<string, { note: string; title: string; items: T[] }>();
  for (const item of items) {
    const { note, note_title } = noteOf(item);
    let g = byNote.get(note);
    if (!g) {
      g = { note, title: note_title, items: [] };
      byNote.set(note, g);
      out.push(g);
    }
    g.items.push(item);
  }
  return out;
}
