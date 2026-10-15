// The block menu's entries (the grip's click, and the "Block" group of the text
// context menu). Every action re-resolves the block from its start position
// before acting, since the document may have changed since the menu opened;
// every dispatch carries a `block.*` user event and the transforms come from
// blockops.ts. Labels come from the catalog; the context-menu store itself is
// reached through the host so this module, like the rest of the editor, does
// not import a store.
import { t, type MessageKey } from "$lib/i18n/i18n.svelte";
import { k } from "$lib/shortcut";
import { foldedRanges } from "@codemirror/language";
import type { EditorView } from "@codemirror/view";
import type { MenuEntry, MenuItem } from "$lib/stores/contextMenu.svelte";
import { type EditorBlock, blockAt, blockContentStart, nextSibling, prevSibling } from "./blocks";
import {
  type TurnTarget,
  canTurnInto,
  deleteBlocks,
  duplicateBlocks,
  indentBlock,
  insertBelow,
  isPrivateBlock,
  moveBlocks,
  outdentBlock,
  turnInto,
} from "./blockops";
import { selectBlock } from "./blocksel";
import { focusEditor } from "./clickguard";
import { applyType } from "./blocktype";
import type { BlockType } from "$lib/api";
import { foldRangeAt, toggleFoldAt } from "./fold";
import { type TableOp, canTableOp, cellAt, tableAppend, tableAt, tableEdit } from "./tableedit";
import type { TransactionSpec } from "@codemirror/state";

export interface BlockMenuHost {
  copyRef(view: EditorView, block: EditorBlock, embed: boolean): void;
  askAgent(view: EditorView, block: EditorBlock): void;
  /** Whether the whole note is in a privacy zone (the AI cannot be asked about it). */
  isPrivate(): boolean;
  /** The block's local id when it is an AI block awaiting review, else null. */
  pendingBlock(view: EditorView, block: EditorBlock): string | null;
  /** How many blocks of this note await review. */
  pendingCount(): number;
  /** Accept pending blocks (local ids) or, with null, every one in the note. */
  acceptProvenance(ids: string[] | null): void;
  /** Put the write behind pending blocks (local ids) or, with null, every one in the note back. */
  rejectProvenance(ids: string[] | null): void;
  /** The vault's enabled block types ("turn into ›" lists them after the kinds). */
  blockTypes(): BlockType[];
}

export const TURN_TARGETS: TurnTarget[] = [
  "paragraph",
  "heading1",
  "heading2",
  "heading3",
  "bullet",
  "numbered",
  "task",
  "quote",
  "callout",
  "code",
];

const ANCHORABLE: EditorBlock["kind"][] = ["paragraph", "heading", "list_item", "task", "quote", "callout"];

function fresh(view: EditorView, b: EditorBlock): EditorBlock | null {
  const now = blockAt(view.state, b.from);
  return now && now.kind === b.kind ? now : null;
}

function act(view: EditorView, b: EditorBlock, f: (now: EditorBlock) => void) {
  return () => {
    const now = fresh(view, b);
    if (!now) return;
    f(now);
    focusEditor(view);
  };
}

/** Whether `[[Note#^id]]` could point at this block. */
export function isAnchorable(b: EditorBlock): boolean {
  return ANCHORABLE.includes(b.kind);
}

/** A table's entries: the row, column and alignment operations on the cell at `at`
 *  (the context menu's point, or the cursor when it is in the table); with no cell to
 *  act on, adding a row or a column at the table's end. `at` is kept as an offset from
 *  the block's start, so it follows the block if the note changes while the menu is open. */
function tableMenu(view: EditorView, b: EditorBlock, at: number | null): MenuItem {
  const model = tableAt(view.state, b.from);
  const cell = model && at != null && at >= model.from && at <= model.to ? cellAt(model, at) : null;
  // with no cell, the appends need only a position in the table: its first line
  const offset = cell && at != null ? at - b.from : 0;
  const run = (f: (pos: number) => TransactionSpec | null) =>
    act(view, b, (now) => {
      const spec = f(Math.min(now.from + offset, view.state.doc.length));
      if (spec) view.dispatch(spec);
    });
  const label = t("table.menu");
  if (!model || !cell) {
    return {
      label,
      submenu: [
        { label: t("table.addRow"), action: run((pos) => tableAppend(view.state, pos, "row")) },
        { label: t("table.addColumn"), action: run((pos) => tableAppend(view.state, pos, "col")) },
      ],
    };
  }
  const item = (op: TableOp, key: MessageKey, extra: Partial<MenuItem> = {}): MenuItem => ({
    label: t(key),
    disabled: !canTableOp(model, op, cell),
    action: run((pos) => {
      const now = tableAt(view.state, pos);
      return now ? tableEdit(view.state, now, op, cellAt(now, pos)) : null;
    }),
    ...extra,
  });
  return {
    label,
    submenu: [
      item("row-above", "table.rowAbove"),
      item("row-below", "table.rowBelow", { hint: k("Enter") }),
      item("col-left", "table.colLeft"),
      item("col-right", "table.colRight"),
      { separator: true },
      item("align-left", "table.alignLeft"),
      item("align-center", "table.alignCenter"),
      item("align-right", "table.alignRight"),
      { separator: true },
      item("delete-row", "table.deleteRow", { danger: true }),
      item("delete-col", "table.deleteCol", { danger: true }),
    ],
  };
}

/** `opts.at`: the position the menu was opened on (the context menu's point); the grip's
 *  menu leaves it out and a table then reads the cursor, when the cursor is in it. */
export function blockMenuEntries(
  view: EditorView,
  b: EditorBlock,
  host: BlockMenuHost,
  opts: { compact: boolean; at?: number },
): MenuEntry[] {
  // The state the entries are built against: the enabled/disabled decisions only.
  // An action reads `view.state` when it runs — the document may have changed
  // while the menu was open, and `now` is resolved against that one.
  const state = view.state;
  // The block the caller holds may belong to an earlier state (the grip's hover
  // resolved it, and selecting it on the click made a new one): the enabled/disabled
  // decisions below compare block objects by identity, so resolve it against the
  // state the menu is built for first
  b = fresh(view, b) ?? b;
  const anchorable = isAnchorable(b);
  const isList = b.kind === "list_item" || b.kind === "task";
  const line = state.doc.lineAt(b.from);
  const foldRange = foldRangeAt(state, line);
  let folded = false;
  if (foldRange) foldedRanges(state).between(foldRange.from, foldRange.from, (f) => void (folded = folded || f === foldRange.from));
  const copyLink: MenuItem = { label: t("block.copyLink"), disabled: !anchorable, action: act(view, b, (now) => host.copyRef(view, now, false)) };
  const copyEmbed: MenuItem = { label: t("block.copyEmbed"), disabled: !anchorable, action: act(view, b, (now) => host.copyRef(view, now, true)) };
  const turnItems: MenuEntry[] = TURN_TARGETS.map((target) => ({
    label: t(`block.kind.${target}`),
    disabled: !canTurnInto(b, target),
    action: act(view, b, (now) => {
      const spec = turnInto(view.state, now, target);
      if (spec) view.dispatch(spec);
    }),
  }));
  const typeItems: MenuEntry[] = host.blockTypes().map((type) => ({
    label: type.icon ? `${type.icon} ${type.name}` : type.name,
    action: act(view, b, (now) => {
      const spec = applyType(view.state, now, type);
      if (spec) view.dispatch(spec);
    }),
  }));
  if (typeItems.length) turnItems.push({ separator: true }, ...typeItems);
  const turn: MenuItem = { label: t("block.turnInto"), disabled: ["table", "hr", "html", "other"].includes(b.kind), submenu: turnItems };
  // The block-level privacy zone as one toggle: a plain block becomes a `[!private]`
  // callout, a private block goes back to a paragraph
  const privateNow = isPrivateBlock(b);
  const privacy: MenuItem = {
    label: t(privateNow ? "block.unmakePrivate" : "block.makePrivate"),
    disabled: !privateNow && !canTurnInto(b, "private"),
    action: act(view, b, (now) => {
      const spec = turnInto(view.state, now, privateNow ? "paragraph" : "private");
      if (spec) view.dispatch(spec);
    }),
  };
  const ask: MenuItem = { label: t("block.askAgent"), action: act(view, b, (now) => host.askAgent(view, now)) };
  // Provenance review: shown only when there is something to accept
  const prov: MenuEntry[] = [];
  const pend = host.pendingBlock(view, b);
  if (pend) {
    prov.push({ label: t("prov.acceptBlock"), action: () => host.acceptProvenance([pend]) });
    prov.push({ label: t("prov.rejectBlock"), action: () => host.rejectProvenance([pend]) });
  }
  if (host.pendingCount() > 0) {
    prov.push({ label: t("prov.acceptAll"), action: () => host.acceptProvenance(null) });
    prov.push({ label: t("prov.rejectAll"), action: () => host.rejectProvenance(null) });
  }
  if (prov.length) prov.unshift({ separator: true });
  const table: MenuEntry[] = b.kind === "table" ? [tableMenu(view, b, opts.at ?? state.selection.main.head)] : [];
  if (opts.compact) return [{ title: t("block.title") }, ...table, copyLink, copyEmbed, turn, privacy, ask, ...prov];
  return [
    {
      label: t("block.insertBelow"),
      action: act(view, b, (now) => view.dispatch(insertBelow(view.state, now))),
    },
    ...table,
    { separator: true },
    copyLink,
    copyEmbed,
    turn,
    privacy,
    { separator: true },
    {
      label: t("block.moveUp"),
      hint: k("Mod+Shift+↑"),
      disabled: !prevSibling(state, b),
      action: act(view, b, (now) => {
        const spec = moveBlocks(view.state, [now], -1);
        if (spec) {
          view.dispatch(spec);
          const moved = blockAt(view.state, view.state.selection.main.head);
          if (moved) selectBlock(view, moved);
        }
      }),
    },
    {
      label: t("block.moveDown"),
      hint: k("Mod+Shift+↓"),
      disabled: !nextSibling(state, b),
      action: act(view, b, (now) => {
        const spec = moveBlocks(view.state, [now], 1);
        if (spec) {
          view.dispatch(spec);
          const moved = blockAt(view.state, view.state.selection.main.head);
          if (moved) selectBlock(view, moved);
        }
      }),
    },
    ...(isList
      ? [
          {
            label: t("block.indent"),
            disabled: !indentBlock(state, b),
            action: act(view, b, (now) => {
              const spec = indentBlock(view.state, now);
              if (spec) view.dispatch(spec);
            }),
          },
          {
            label: t("block.outdent"),
            disabled: !outdentBlock(state, b),
            action: act(view, b, (now) => {
              const spec = outdentBlock(view.state, now);
              if (spec) view.dispatch(spec);
            }),
          },
        ]
      : []),
    ...(foldRange
      ? [
          {
            label: t(folded ? "block.unfold" : "block.fold"),
            action: act(view, b, (now) => void toggleFoldAt(view, now.from)),
          },
        ]
      : []),
    { separator: true },
    {
      label: t("block.duplicate"),
      // The key that works in both modes (block mode also takes a bare ⌘D, text mode does not)
      hint: k("Mod+Shift+D"),
      action: act(view, b, (now) => {
        const spec = duplicateBlocks(view.state, [now]);
        if (spec) view.dispatch(spec);
      }),
    },
    {
      label: t("block.delete"),
      hint: k("Backspace"),
      danger: true,
      action: act(view, b, (now) => {
        const spec = deleteBlocks(view.state, [now]);
        if (spec) view.dispatch(spec);
      }),
    },
    { separator: true },
    ask,
    ...prov,
  ];
}

