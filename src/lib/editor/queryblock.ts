// The ```vemry-query block: a declarative dynamic projection embedded in a note's
// body, and a safe subset of the query language (no nesting, no evaluated
// expressions, no writing to disk or reaching the network).
// With the cursor outside the block the whole fence renders as a live query result
// (the same block-level replace semantics as table.ts, which has to come from a
// StateField); clicking a result row jumps to the note it came from. The YAML body is
// eight flat scalars (all the fields of BlockQuery) plus two presentation keys,
// `group` and `sort`, that never leave the frontend (views/queryrows.ts) — for which
// a 20-line parser is enough and js-yaml is not pulled in. An index update triggers
// a requery through the queryRefresh effect — sharing the refresh signal
// (indexVersion) with the projection views.
// In any standard Markdown renderer it degrades to an ordinary code block, with zero
// lock-in.
import { t } from "$lib/i18n/i18n.svelte";
import { blockRoot } from "./blockwidget";
import { syntaxTree } from "@codemirror/language";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { type EditorState, type Extension, StateEffect, StateField } from "@codemirror/state";
import type { BlockHit, BlockQuery, BlockType, TaskState } from "$lib/api";
import { fencedCodeText } from "./fenced";
import { selectionTouches } from "./reveal";
import { foldRangesOf, insideFold } from "./table";
import { isClosed, dateFieldLabel, dateIconSvg, dueLabel, localDate } from "$lib/views/tasks";
import { stateField } from "$lib/fields";
import { dateChip, fieldChip, stateGlyph, stateGlyphSvg, stateTone, tagChip } from "$lib/chips";
import { type BlockRow, blockRowOf, rowStep, stateSummary } from "$lib/views/blockrow";
import { type GroupKey, type SortSpec, groupHits, parseGroup, parseSort, sortHits } from "$lib/views/queryrows";

/** Where a row's chip was clicked, for the menu the host raises. */
export interface RowChipClick {
  hit: BlockHit;
  key: string;
  value: string;
  at: { x: number; y: number };
}

export interface QueryHost {
  runQuery(q: BlockQuery): Promise<BlockHit[]>;
  /** Jump to a row's block (its byte offset in the note), or to the note when the host cannot land on a block. */
  openNote(path: string, byteStart?: number, content?: string): void;
  /** The enabled block types: what makes a field a state field (fields.ts's stateField).
   *  Absent, every field is a plain chip. */
  blockTypes?: () => BlockType[];
  /** A click on a row's field chip: the host raises the value menu and writes the pick
   *  back to the block's note. Absent, the chips are labels. */
  onRowChip?: (click: RowChipClick) => void;
  /** A click on a task row's state glyph: flip the checkbox on disk (⌥ for "in
   *  progress", as the editor's own checkbox). Absent, the glyph is a label. */
  onRowState?: (hit: BlockHit, state: TaskState) => void;
}

/** An index version change → every query block requeries (a widget's eq key carries the version) */
export const queryRefresh = StateEffect.define<number>();

// The types come from api.ts's BlockQuery mirror: when the Rust side adds a kind or a state, tsc catches the drift
const KINDS: NonNullable<BlockQuery["kind"]>[] = ["paragraph", "heading", "list_item", "task", "claim", "evidence", "code_block", "private"];
const STATES: NonNullable<BlockQuery["task_state"]>[] = ["todo", "doing", "done", "cancelled"];
const FIELDS = "kind/note/tag/text/task_state/due_before/field/limit/group/sort";
// Mirrors vemry-core's parse_field_filter grammar for a localized error only; Rust is the authority
const FIELD_FLAT_RE = /^(?!\d+(?![\p{L}\p{N}_-]))[\p{L}\p{N}_-]+\s*(?:(?:!=|=|<|>|~)\s*\S.*)?$/u;

/** What a fence parses to: the query for Rust, and the two presentation keys the widget applies itself. */
export interface ParsedQuery {
  query?: BlockQuery;
  error?: string;
  group?: GroupKey;
  sort?: SortSpec;
}

/** Flat `key: value` lines → BlockQuery fields, validated field by field (exported for testing) */
export function parseQuerySource(src: string): ParsedQuery {
  const q: BlockQuery = {};
  let group: GroupKey | undefined;
  let sort: SortSpec | undefined;
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
      // Presentation keys: applied by the widget, never sent to Rust
      case "group": {
        const g = parseGroup(value);
        if (!g) return { error: t("queryBlock.groupFormat") };
        group = g;
        break;
      }
      case "sort": {
        const s = parseSort(value);
        if (!s) return { error: t("queryBlock.sortFormat") };
        sort = s;
        break;
      }
      default:
        return { error: t("queryBlock.unknownField", { key, fields: FIELDS }) };
    }
  }
  return { query: q, group, sort };
}

/** A block kind's name for a group header. */
function kindLabel(kind: string): string {
  return (KINDS as string[]).includes(kind) ? t(`queryBlock.kind.${kind as (typeof KINDS)[number]}`) : kind;
}

function truncate(s: string, max: number): string {
  const t = s.trim().replace(/\s+/g, " ");
  return t.length <= max ? t : t.slice(0, max) + "…";
}

/** A stopped mousedown: the row's click must not move the editor's selection (which
 *  would reveal the fence's source and replace this widget under the pointer). */
function onPress(el: HTMLElement, handler: (e: MouseEvent) => void) {
  el.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    handler(e);
  });
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

  /** One row: a task's checkbox in the margin, the words, the chips (a state field's
   *  value carries its own glyph on its chip — the margin is the checkbox's place, and a
   *  state is drawn once per row), and the note — unless the words are the note's title
   *  already (a marker line such as `#chapter [status:: unread]`), or the group says
   *  which note. */
  private rowEl(row: BlockRow, today: string, inNoteGroup: boolean): HTMLElement {
    const host = this.host;
    const el = document.createElement("div");
    el.className = "cm-queryblock-row";
    el.setAttribute("role", "button");
    el.tabIndex = 0;
    const step = rowStep(row);
    if (step) el.classList.add(`cm-queryblock-row-${stateTone(step.step, step.of)}`);
    if (row.state) {
      const g = stateGlyph(step!.step, step!.of, t("task.toggleDone"), step!.cancelled);
      g.classList.add("cm-queryblock-glyph");
      if (host.onRowState) {
        g.classList.add("cm-queryblock-clickable");
        onPress(g, (e) => host.onRowState!(row.hit, nextTaskState(row.state!, e.altKey)));
      }
      el.appendChild(g);
    } else {
      el.appendChild(Object.assign(document.createElement("span"), { className: "cm-queryblock-glyph cm-queryblock-glyph-none" }));
    }
    const text = document.createElement("span");
    text.className = "cm-queryblock-text";
    text.textContent = truncate(row.text, 160);
    text.title = row.text;
    el.appendChild(text);
    const chips = document.createElement("span");
    chips.className = "cm-queryblock-chips";
    if (row.due) chips.appendChild(dateChip(dateIconSvg("due"), dueLabel(row.due, row.overdue, "short"), row.overdue, `${dateFieldLabel("due")} ${row.due}`));
    if (row.scheduled) chips.appendChild(dateChip(dateIconSvg("scheduled"), row.scheduled, false, dateFieldLabel("scheduled")));
    for (const c of row.chips) {
      if (c.kind === "tag") {
        chips.appendChild(tagChip(c.tag));
        continue;
      }
      const chip = fieldChip(c.key, c.value, c.kind === "state" ? c : null, c.kind !== "state");
      if (host.onRowChip) {
        chip.classList.add("cm-queryblock-clickable");
        onPress(chip, (e) => host.onRowChip!({ hit: row.hit, key: c.key, value: c.value, at: { x: e.clientX, y: e.clientY } }));
      }
      chips.appendChild(chip);
    }
    if (chips.childElementCount) el.appendChild(chips);
    if (!row.titleAsText && !inNoteGroup) {
      const note = document.createElement("span");
      note.className = "cm-queryblock-note";
      note.textContent = row.hit.note_title;
      note.title = row.hit.note;
      el.appendChild(note);
    }
    const open = () => host.openNote(row.hit.note, row.hit.block.span.start, row.hit.block.content);
    onPress(el, open);
    el.addEventListener("keydown", (e) => {
      if (e.target === el && (e.key === "Enter" || e.key === " ")) {
        e.preventDefault();
        open();
      }
    });
    return el;
  }

  /** The head: the count, and — when the rows carry one state field — how many are at
   *  each step, with a bar of the same shares under it. */
  private headEl(rows: BlockRow[], types: BlockType[]): HTMLElement[] {
    const head = document.createElement("div");
    head.className = "cm-queryblock-head";
    const count = document.createElement("span");
    count.textContent = t("queryBlock.hits", { n: rows.length });
    head.appendChild(count);
    const summary = stateSummary(rows, types);
    if (!summary || !summary.total) return [head];
    const dist = document.createElement("span");
    dist.className = "cm-queryblock-dist";
    dist.title = summary.key;
    summary.values.forEach((v, i) => {
      if (!summary.counts[i]) return;
      const item = document.createElement("span");
      item.className = `cm-queryblock-dist-item vchip-tone-${stateTone(i, summary.values.length)}`;
      item.innerHTML = stateGlyphSvg(i, summary.values.length);
      item.append(`${v} ${summary.counts[i]}`);
      dist.appendChild(item);
    });
    head.appendChild(dist);
    const bar = document.createElement("div");
    bar.className = "cm-queryblock-bar";
    bar.setAttribute("role", "img");
    bar.setAttribute("aria-label", summary.values.map((v, i) => `${v} ${summary.counts[i]}`).join(", "));
    // the done share first, then the active one, so the bar reads left to right as progress
    const last = summary.values.length - 1;
    const seg = (share: number, tone: string) => {
      if (share <= 0) return;
      const s = document.createElement("i");
      s.className = `cm-queryblock-bar-${tone}`;
      s.style.width = `${(share * 100).toFixed(1)}%`;
      bar.appendChild(s);
    };
    seg(summary.counts[last] / summary.total, "done");
    seg(summary.counts.slice(1, last).reduce((a, b) => a + b, 0) / summary.total, "active");
    return [head, bar];
  }

  toDOM(view: EditorView) {
    const wrap = document.createElement("div");
    wrap.className = "cm-queryblock";
    const parsed = parseQuerySource(this.src);
    if (parsed.error) {
      wrap.classList.add("cm-queryblock-error");
      wrap.textContent = `vemry-query: ${parsed.error}`;
      return blockRoot(wrap);
    }
    wrap.textContent = t("queryBlock.querying");
    // As in livepreview's ImageWidget: requestMeasure corrects the line height once the asynchronous load finishes
    void this.host
      .runQuery(parsed.query!)
      .then((hits) => {
        if (!wrap.isConnected) return;
        wrap.textContent = "";
        const types = this.host.blockTypes?.() ?? [];
        const today = localDate();
        // the tag the query filtered by is on every row: not a chip; a group by a
        // field's tag would be the same, but a group is by value, so only the query's
        const hideTags = parsed.query!.tag ? [parsed.query!.tag] : [];
        const rowOf = (hit: BlockHit) => blockRowOf(hit, types, { today, hideTags });
        // a sort or a group by a state field reads the field's declared order
        const declaredOf = (key: { by: string; key?: string }) => (key.by === "field" && key.key ? stateField(types, key.key) : null);
        const ordered = parsed.sort ? sortHits(hits, parsed.sort, declaredOf(parsed.sort.key)) : hits;
        const rows = ordered.map(rowOf);
        for (const el of this.headEl(rows, types)) wrap.appendChild(el);
        if (parsed.group) {
          // One <details> per group, open by default; a group by note hides the
          // per-row note name (the header says it), a missing field value lands last
          const byRow = new Map(rows.map((r) => [r.hit, r]));
          for (const g of groupHits(ordered, parsed.group, t("queryBlock.none"), kindLabel, declaredOf(parsed.group))) {
            const details = document.createElement("details");
            details.className = "cm-queryblock-group";
            details.open = g.hits.length > 0;
            if (!g.hits.length) details.classList.add("cm-queryblock-group-empty");
            const summary = document.createElement("summary");
            const title = document.createElement("span");
            title.className = "cm-queryblock-group-title";
            title.textContent = g.title;
            const count = document.createElement("span");
            count.className = "cm-queryblock-group-count";
            count.textContent = String(g.hits.length);
            summary.append(title, count);
            details.appendChild(summary);
            for (const hit of g.hits) details.appendChild(this.rowEl(byRow.get(hit)!, today, parsed.group.by === "note"));
            wrap.appendChild(details);
          }
        } else {
          for (const row of rows) wrap.appendChild(this.rowEl(row, today, false));
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
    return blockRoot(wrap);
  }
}

/** The state a click on a task row's glyph sets: a plain click flips done, ⌥ flips "in
 *  progress" — the same two gestures as the editor's checkbox. */
export function nextTaskState(state: TaskState, alt: boolean): TaskState {
  if (alt) return state === "doing" ? "todo" : "doing";
  return isClosed(state) ? "todo" : "done";
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
      if (insideFold(state, from, to)) return false;
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
      const foldChanged = foldRangesOf(tr).length > 0;
      if (!tr.docChanged && !tr.selection && !treeChanged && !refreshed && !foldChanged) return value;
      // The common case — a note with no query block: neither a cursor move nor an
      // index refresh needs a full rescan (a document or syntax-tree change still does,
      // since a fence may have just been typed)
      if (!value.hasFence && !tr.docChanged && !treeChanged && !foldChanged) {
        return version === value.version ? value : { ...value, version };
      }
      return { version, ...build(tr.state, version, host) };
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });
}
