// The block menu's entries (the grip's click, and the "块" group of the text
// context menu). Every action re-resolves the block from its start position
// before acting, since the document may have changed since the menu opened;
// every dispatch carries a `block.*` user event and the transforms come from
// blockops.ts. Labels come from the catalog; the context-menu store itself is
// reached through the host so this module, like the rest of the editor, does
// not import a store.
import { t } from "$lib/i18n/i18n.svelte";
import { k } from "$lib/shortcut";
import { foldedRanges } from "@codemirror/language";
import type { EditorView } from "@codemirror/view";
import type { MenuEntry, MenuItem } from "$lib/stores/contextMenu.svelte";
import { type EditorBlock, blockAt, blockContentStart, extentTo, nextSibling, prevSibling } from "./blocks";
import {
  type TurnTarget,
  canTurnInto,
  deleteBlocks,
  duplicateBlocks,
  indentBlock,
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

export interface BlockMenuHost {
  copyRef(view: EditorView, block: EditorBlock, embed: boolean): void;
  askAgent(view: EditorView, block: EditorBlock): void;
  /** Whether the whole note is in a privacy zone (the AI cannot be asked about it). */
  isPrivate(): boolean;
  /** The block's local id when it is an AI block awaiting review, else null. */
  pendingBlock(view: EditorView, block: EditorBlock): string | null;
  /** How many blocks of this note await review. */
  pendingCount(): number;
  /** Accept one pending block (a local id) or, with null, every one in the note. */
  acceptProvenance(localId: string | null): void;
  /** Put the write behind one pending block (a local id) or, with null, every one in the note back. */
  rejectProvenance(localId: string | null): void;
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

export function blockMenuEntries(view: EditorView, b: EditorBlock, host: BlockMenuHost, opts: { compact: boolean }): MenuEntry[] {
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
      const spec = turnInto(state, now, target);
      if (spec) view.dispatch(spec);
    }),
  }));
  const typeItems: MenuEntry[] = host.blockTypes().map((type) => ({
    label: type.icon ? `${type.icon} ${type.name}` : type.name,
    action: act(view, b, (now) => {
      const spec = applyType(state, now, type);
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
    prov.push({ label: t("prov.acceptBlock"), action: () => host.acceptProvenance(pend) });
    prov.push({ label: t("prov.rejectBlock"), action: () => host.rejectProvenance(pend) });
  }
  if (host.pendingCount() > 0) {
    prov.push({ label: t("prov.acceptAll"), action: () => host.acceptProvenance(null) });
    prov.push({ label: t("prov.rejectAll"), action: () => host.rejectProvenance(null) });
  }
  if (prov.length) prov.unshift({ separator: true });
  if (opts.compact) return [{ title: t("block.title") }, copyLink, copyEmbed, turn, privacy, ask, ...prov];
  return [
    {
      label: t("block.insertBelow"),
      action: act(view, b, (now) => {
        const at = extentTo(state, now);
        view.dispatch({ changes: { from: at, insert: "\n\n" }, selection: { anchor: at + 2 }, userEvent: "input.type" });
      }),
    },
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
        const spec = duplicateBlocks(state, [now]);
        if (spec) view.dispatch(spec);
      }),
    },
    {
      label: t("block.delete"),
      hint: k("Backspace"),
      danger: true,
      action: act(view, b, (now) => {
        const spec = deleteBlocks(state, [now]);
        if (spec) view.dispatch(spec);
      }),
    },
    { separator: true },
    ask,
    ...prov,
  ];
}

