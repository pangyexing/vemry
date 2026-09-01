// The ```vemry-query block: a declarative dynamic projection embedded in a note's
// body, and a safe subset of the query language (no nesting, no evaluated
// expressions, no writing to disk or reaching the network).
// With the cursor outside the block the whole fence renders as a live query result
// (the same block-level replace semantics as table.ts, which has to come from a
// StateField); clicking a result row jumps to the note it came from. The YAML body is
// eight flat scalars (all the fields of BlockQuery), for which a 20-line parser is
// enough and js-yaml is not pulled in. An index update triggers a requery through the
// queryRefresh effect — sharing the refresh signal (indexVersion) with the projection
// views.
// In any standard Markdown renderer it degrades to an ordinary code block, with zero
// lock-in.
import { t } from "$lib/i18n/i18n.svelte";
import { syntaxTree } from "@codemirror/language";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { type EditorState, type Extension, StateEffect, StateField } from "@codemirror/state";
import type { BlockHit, BlockQuery } from "$lib/api";
import { fencedCodeText } from "./fenced";
import { selectionTouches } from "./reveal";
import { localDate } from "$lib/views/tasks";

export interface QueryHost {
  runQuery(q: BlockQuery): Promise<BlockHit[]>;
  openNote(path: string): void;
}

/** An index version change → every query block requeries (a widget's eq key carries the version) */
export const queryRefresh = StateEffect.define<number>();

// The types come from api.ts's BlockQuery mirror: when the Rust side adds a kind or a state, tsc catches the drift
const KINDS: NonNullable<BlockQuery["kind"]>[] = ["paragraph", "heading", "list_item", "task", "claim", "evidence", "code_block", "private"];
const STATES: NonNullable<BlockQuery["task_state"]>[] = ["todo", "doing", "done"];
const FIELDS = "kind/note/tag/text/task_state/due_before/field/limit";
// Mirrors vemry-core's parse_field_filter grammar for a localized error only; Rust is the authority
const FIELD_FLAT_RE = /^[\p{L}\p{N}_-]+\s*(?:(?:!=|=|<|>|~)\s*\S.*)?$/u;

/** Flat `key: value` lines → BlockQuery fields, validated field by field (exported for testing) */
export function parseQuerySource(src: string): { query?: BlockQuery; error?: string } {
  const q: BlockQuery = {};
  for (const rawLine of src.split("\n")) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const m = /^([A-Za-z_]+)\s*:\s*(.*)$/.exec(line);
    if (!m) return { error: t("queryBlock.cannotParse", { line }) };
    const key = m[1];
    let value = m[2].trim();
    if (
      (value.startsWith('"') && value.endsWith('"') && value.length >= 2) ||
      (value.startsWith("'") && value.endsWith("'") && value.length >= 2)
    ) {
      value = value.slice(1, -1);
    } else {
      // An unquoted value: under YAML rules a trailing ` # …` is a comment, not part of the value
      value = value.replace(/\s+#.*$/, "");
    }
    switch (key) {
      case "kind":
        if (!(KINDS as string[]).includes(value)) return { error: t("queryBlock.kindMustBe", { list: KINDS.join("/") }) };
        q.kind = value as BlockQuery["kind"];
        break;
      case "task_state":
        if (!(STATES as string[]).includes(value)) return { error: t("queryBlock.stateMustBe", { list: STATES.join("/") }) };
        q.task_state = value as BlockQuery["task_state"];
        break;
      case "note":
      case "text":
        if (!value) return { error: t("queryBlock.fieldEmpty", { key }) };
        q[key] = value;
        break;
      case "tag":
        if (!value) return { error: t("queryBlock.tagEmpty") };
        q.tag = value.replace(/^#/, "");
        break;
      case "due_before":
        if (value === "today") {
          q.due_before = localDate(); // the same "local today" as Agenda and Timeline
          break;
        }
        if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return { error: t("queryBlock.dueBeforeFormat") };
        q.due_before = value;
        break;
      case "field": {
        if (!FIELD_FLAT_RE.test(value)) return { error: t("queryBlock.fieldFormat") };
        const prev = q.field == null ? [] : Array.isArray(q.field) ? q.field : [q.field];
        q.field = [...prev, value];
        break;
      }
      case "limit": {
        const n = Number(value);
        if (!Number.isInteger(n) || n <= 0) return { error: t("queryBlock.limitPositive") };
        q.limit = n;
        break;
      }
      default:
        return { error: t("queryBlock.unknownField", { key, fields: FIELDS }) };
    }
  }
  return { query: q };
}

function truncate(s: string, max: number): string {
  const t = s.trim().replace(/\s+/g, " ");
  return t.length <= max ? t : t.slice(0, max) + "…";
}

class QueryWidget extends WidgetType {
  constructor(
    readonly src: string,
    readonly version: number,
    readonly host: QueryHost,
  ) {
    super();
  }
  eq(other: QueryWidget) {
    return other.src === this.src && other.version === this.version;
  }
  get estimatedHeight() {
    return 80;
  }
  ignoreEvent() {
    return true;
  }
  toDOM(view: EditorView) {
    const wrap = document.createElement("div");
    wrap.className = "cm-queryblock";
    const parsed = parseQuerySource(this.src);
    if (parsed.error) {
      wrap.classList.add("cm-queryblock-error");
      wrap.textContent = `vemry-query: ${parsed.error}`;
      return wrap;
    }
    wrap.textContent = t("queryBlock.querying");
    // As in livepreview's ImageWidget: requestMeasure corrects the line height once the asynchronous load finishes
    void this.host
      .runQuery(parsed.query!)
      .then((hits) => {
        if (!wrap.isConnected) return;
        wrap.textContent = "";
        const head = document.createElement("div");
        head.className = "cm-queryblock-head";
        head.textContent = t("queryBlock.hits", { n: hits.length });
        wrap.appendChild(head);
        for (const hit of hits) {
          const row = document.createElement("button");
          row.className = "cm-queryblock-row";
          const text = document.createElement("span");
          text.className = "cm-queryblock-text";
          text.textContent = truncate(hit.block.content, 120);
          const note = document.createElement("span");
          note.className = "cm-queryblock-note";
          note.textContent = hit.note_title;
          row.append(text, note);
          row.onmousedown = (e) => {
            e.preventDefault();
            this.host.openNote(hit.note);
          };
          wrap.appendChild(row);
        }
        if (!hits.length) {
          const empty = document.createElement("div");
          empty.className = "cm-queryblock-empty";
          empty.textContent = t("queryBlock.noHits");
          wrap.appendChild(empty);
        }
        view.requestMeasure();
      })
      .catch((e) => {
        if (!wrap.isConnected) return;
        wrap.classList.add("cm-queryblock-error");
        wrap.textContent = String(e);
      });
    return wrap;
  }
}

const HEAD_RE = /^\s*(`{3,}|~{3,})\s*vemry-query\s*$/;

interface BuildResult {
  decos: DecorationSet;
  /** Whether the document holds a vemry-query fence at all (even one whose widget is not rendered because the cursor is inside it). */
  hasFence: boolean;
}

function build(state: EditorState, version: number, host: QueryHost): BuildResult {
  const doc = state.doc;
  const decos: ReturnType<Decoration["range"]>[] = [];
  let hasFence = false;
  syntaxTree(state).iterate({
    enter(node) {
      if (node.name !== "FencedCode") return;
      const firstLine = doc.lineAt(node.from);
      if (!HEAD_RE.test(firstLine.text)) return false;
      hasFence = true;
      const from = firstLine.from;
      const to = doc.lineAt(Math.min(node.to, doc.length)).to;
      // A selection touching the block expands the whole source for editing (as in
      // table.ts; block level uses selectionTouches, so a range selection expands too —
      // deliberately different from the empty-only rule of inline elements, see
      // reveal.ts)
      if (selectionTouches(state, from, to)) return false;
      const src = fencedCodeText(doc, node.node);
      decos.push(Decoration.replace({ widget: new QueryWidget(src, version, host), block: true }).range(from, to));
      return false;
    },
  });
  return { decos: Decoration.set(decos, true), hasFence };
}

export function queryBlockExtension(host: QueryHost): Extension {
  return StateField.define<{ version: number } & BuildResult>({
    create: (state) => ({ version: 0, ...build(state, 0, host) }),
    update(value, tr) {
      let version = value.version;
      let refreshed = false;
      for (const e of tr.effects) {
        if (e.is(queryRefresh)) {
          version = e.value;
          refreshed = true;
        }
      }
      const treeChanged = syntaxTree(tr.state) !== syntaxTree(tr.startState);
      if (!tr.docChanged && !tr.selection && !treeChanged && !refreshed) return value;
      // The common case — a note with no query block: neither a cursor move nor an
      // index refresh needs a full rescan (a document or syntax-tree change still does,
      // since a fence may have just been typed)
      if (!value.hasFence && !tr.docChanged && !treeChanged) {
        return version === value.version ? value : { ...value, version };
      }
      return { version, ...build(tr.state, version, host) };
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });
}
