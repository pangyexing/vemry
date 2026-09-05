// What the active projection view shows right now, as something the composer can
// attach: the Agenda, the board and the Review view register their live query and row count here on
// every change (and clear it when they unmount), and the @ picker's "current view"
// candidate and the view header's "Ask AI" button read it. The rows themselves are
// not carried — Rust re-runs the query through the privacy-filtered query API, so
// what the model gets is exactly what the view would show minus the private blocks.
import type { BlockQuery, ContextRef } from "$lib/api";

export interface ViewContext {
  /** The view's id: `agenda` / `kanban` / `review` — the audit records it as `view:<id>`. */
  view: "agenda" | "kanban" | "review";
  /** The chip's wording (the view's title, plus the grouping or the filter when set). */
  label: string;
  query: BlockQuery;
  rows: number;
}

class ViewContextStore {
  current = $state<ViewContext | null>(null);

  set(ctx: ViewContext | null) {
    this.current = ctx;
  }

  /** The reference the composer sends (null when no view is registered). */
  ref(): Extract<ContextRef, { kind: "view" }> | null {
    const c = this.current;
    if (!c) return null;
    return { kind: "view", path: `view:${c.view}`, label: c.label, query: c.query, rows: c.rows };
  }
}

export const viewContext = new ViewContextStore();
