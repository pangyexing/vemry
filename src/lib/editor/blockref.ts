// The second and third stages of the `[[` completion: after `[[Note#` the
// headings and blocks of that note (after `[[Note#^` the blocks alone). Picking
// a heading inserts `[[Note#Heading]]`; picking a block inserts `[[Note#^id]]`,
// minting the id when the block has none — in the editor buffer when the note
// is the one open here (a disk write would be clobbered by autosave; Rust still
// decides where the id goes, through `block_id_plan`), on disk otherwise
// (`block_ensure_id`). Both paths guard on the block's content, so a note edited
// since the candidates were fetched refuses instead of tagging the wrong block.
import { t } from "$lib/i18n/i18n.svelte";
import type { Completion, CompletionContext, CompletionResult, CompletionSource } from "@codemirror/autocomplete";
import type { EditorView } from "@codemirror/view";
import type { BlockHit, BlockIdPlan, BlockSelector } from "$lib/api";
import { byteToChar } from "$lib/views/tasks";

export interface RefHost {
  currentPath(): string | null;
  /** Resolve a wikilink note part to a note id (""/whitespace = the current note); null when unknown or private. */
  resolveNote(target: string): Promise<string | null>;
  /** All blocks of a note in document order (cached per index version by the caller). */
  blocksOf(note: string): Promise<BlockHit[]>;
  /** Buffer path: the one insert that gives a block its id. */
  planId(source: string, selector: BlockSelector): Promise<BlockIdPlan>;
  /** Disk path: mint or return the block's id, writing the note. */
  ensureId(note: string, localId: string, guard: string): Promise<string>;
  toast(message: string, level?: "info" | "error"): void;
}

/** `[[Note#…` — headings and blocks; `[[Note#^…` — blocks only. The stage regexes
 *  exclude `#`, `|` and `^` from validFor so the keystroke that changes stage forces
 *  a fresh query (CodeMirror would otherwise reuse the previous result). */
export const STAGE2_RE = /\[\[([^\]\n#|]*)#([^\]\n#|^]*)$/;
export const STAGE3_RE = /\[\[([^\]\n#|]*)#\^([^\]\n#|]*)$/;

const KIND_GLYPH: Record<string, string> = {
  paragraph: "¶",
  list_item: "•",
  task: "☐",
  claim: "◆",
  evidence: "≡",
  heading: "›",
};

/** Whether `[[Note#^id]]` could point at this hit (mirrors Rust's blockid::is_anchorable). */
export function isAnchorableHit(h: BlockHit): boolean {
  const k = h.block.kind.type;
  if (k === "code_block" || k === "private") return false;
  const c = h.block.content;
  if (k === "paragraph" && (c.trimStart().startsWith("|") || /^\s*([-*_])(\s*\1){2,}\s*$/.test(c))) return false;
  return true;
}

/** The first line of a block, markers stripped, ≤ 80 characters. */
export function blockLabel(content: string): string {
  const first = (content.split("\n")[0] ?? "")
    .replace(/^[\s>]*(?:[-*+]\s+(?:\[[ xX/]\]\s*)?|\d+[.)]\s+)?(?:\[![\w-]+\][+-]?\s*)?/, "")
    // the trailing ` ^id` is shown by the caller, next to the text, not inside it
    .replace(/\s*\^[A-Za-z0-9-]+$/, "")
    .replace(/\s+/g, " ")
    .trim();
  return first.length > 80 ? first.slice(0, 80) + "…" : first;
}

/** Insert the link's tail, closing with `]]` unless closeBrackets already did. */
function finishLink(view: EditorView, applyFrom: number, applyTo: number, tail: string, extra?: { from: number; insert: string }) {
  const closed = view.state.sliceDoc(applyTo, applyTo + 2) === "]]";
  const changes = [{ from: applyFrom, to: applyTo, insert: closed ? tail : `${tail}]]` }];
  let shift = 0;
  if (extra) {
    changes.push({ from: extra.from, to: extra.from, insert: extra.insert });
    if (extra.from <= applyFrom) shift = extra.insert.length;
  }
  view.dispatch({ changes, selection: { anchor: applyFrom + shift + tail.length + 2 } });
}

export function refCompletions(host: RefHost): CompletionSource {
  return async (context: CompletionContext): Promise<CompletionResult | null> => {
    const s3 = context.matchBefore(STAGE3_RE);
    const s2 = s3 ? null : context.matchBefore(STAGE2_RE);
    const before = s3 ?? s2;
    if (!before) return null;
    const m = (s3 ? STAGE3_RE : STAGE2_RE).exec(before.text)!;
    const notePart = m[1].trim();
    const blocksOnly = !!s3;
    const from = before.from + m[0].length - m[2].length;
    const note = notePart ? await host.resolveNote(notePart) : host.currentPath();
    if (!note) return null;
    const hits = await host.blocksOf(note);
    const options: Completion[] = [];
    if (!blocksOnly) {
      options.push({
        label: t("wikilink.wholeNote"),
        type: "text",
        boost: 99,
        apply: (view, _c, applyFrom, applyTo) => {
          // drop the `#` too: `[[Note]]`
          const closed = view.state.sliceDoc(applyTo, applyTo + 2) === "]]";
          view.dispatch({
            changes: { from: applyFrom - 1, to: applyTo, insert: closed ? "" : "]]" },
            selection: { anchor: applyFrom - 1 + 2 },
          });
        },
      });
    }
    let i = 0;
    for (const h of hits) {
      const kind = h.block.kind.type;
      const sortText = String(i++).padStart(5, "0");
      if (kind === "heading") {
        if (blocksOnly) continue;
        const text = h.block.content.replace(/\s+\^[A-Za-z0-9-]+$/, "").trim();
        options.push({
          label: text,
          detail: KIND_GLYPH.heading,
          type: "text",
          sortText,
          section: { name: t("wikilink.sectionHeadings"), rank: 0 },
          apply: (view, _c, applyFrom, applyTo) => finishLink(view, applyFrom, applyTo, text),
        });
        continue;
      }
      if (!isAnchorableHit(h)) continue;
      const label = blockLabel(h.block.content) + (h.block.explicit_id ? `  ^${h.block.local_id}` : "");
      options.push({
        label: label || h.block.local_id,
        detail: KIND_GLYPH[kind] ?? "¶",
        type: "text",
        sortText,
        section: { name: t("wikilink.sectionBlocks"), rank: 1 },
        apply: (view, _c, applyFrom, applyTo) => void pickBlock(view, host, note, h, applyFrom, applyTo, before.from),
      });
    }
    if (!options.length) return null;
    return { from, options, validFor: blocksOnly ? /^[^\]\n#|]*$/ : /^[^\]\n#|^]*$/ };
  };
}

async function pickBlock(view: EditorView, host: RefHost, note: string, h: BlockHit, applyFrom: number, applyTo: number, tokenFrom: number) {
  if (h.block.explicit_id) {
    finishLink(view, applyFrom, applyTo, `^${h.block.local_id}`);
    return;
  }
  const token = view.state.sliceDoc(tokenFrom, applyTo);
  if (note === host.currentPath()) {
    // buffer path: Rust plans against the text this editor holds
    const source = view.state.doc.toString();
    let plan: BlockIdPlan;
    try {
      plan = await host.planId(source, { by: "id", local_id: h.block.local_id, guard: h.block.content });
    } catch (e) {
      host.toast(String(e), "error");
      return;
    }
    if (view.state.doc.toString() !== source || view.state.sliceDoc(tokenFrom, applyTo) !== token) {
      host.toast(t("wikilink.blockChanged"), "error");
      return;
    }
    if (plan.target.kind !== "block") return;
    const id = plan.target.id;
    const extra = plan.insert ? { from: byteToChar(source, plan.insert.offset), insert: plan.insert.text } : undefined;
    finishLink(view, applyFrom, applyTo, `^${id}`, extra);
    return;
  }
  let id: string;
  try {
    id = await host.ensureId(note, h.block.local_id, h.block.content);
  } catch (e) {
    host.toast(String(e), "error");
    return;
  }
  // the disk write already happened: only insert when the token is still where it was
  if (view.state.sliceDoc(tokenFrom, applyTo) !== token || view.state.selection.main.head !== applyTo) {
    host.toast(t("wikilink.mintedNotInserted", { id }), "error");
    return;
  }
  finishLink(view, applyFrom, applyTo, `^${id}`);
}
