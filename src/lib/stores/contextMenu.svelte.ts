// The application's one context-menu state. A menu item is data rather than layout:
// entries are computed for the context at the moment of open (following FileTree's
// undoLabel and paste wording), and rendering belongs to the single <ContextMenu />
// mounted in App — the same shape as toast / PermissionDialog / ScriptGrantDialog.
// What making the mount point a singleton actually buys: the window-level pointerdown
// and keydown capture listeners collapse from "a pair per right-click host" to one
// pair, and a new host no longer needs pos/entries state of its own.

export interface MenuItem {
  label: string;
  /** The right-aligned shortcut hint column; the caller passes what k() returned. */
  hint?: string;
  danger?: boolean;
  disabled?: boolean;
  /** A second confirmation: the first click turns the item's wording into this string and only another click runs it; a click outside or Esc resets it. */
  confirm?: string;
  action: () => void;
}
export type MenuEntry = MenuItem | { separator: true } | { title: string };

class ContextMenuStore {
  pos = $state<{ x: number; y: number } | null>(null);
  entries = $state<MenuEntry[]>([]);

  /** Open it at the right-click. preventDefault is collected here, and the caller only
   *  computes entries (a host that needs stopPropagation adds it itself — that is a
   *  matter for nested hosts, not for the menu). */
  open(e: MouseEvent, entries: MenuEntry[]) {
    e.preventDefault();
    this.entries = entries;
    this.pos = { x: e.clientX, y: e.clientY };
  }

  close() {
    this.pos = null;
    // Clear entries along the way: an action closure captures the host's scope (an
    // EditorView, a session object and so on), and as a singleton that never goes away
    // it would keep the previous menu's whole captured environment until the next
    // right-click.
    this.entries = [];
  }
}

export const contextMenu = new ContextMenuStore();
