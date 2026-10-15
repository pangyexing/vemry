// The "table" probe: a table in a note driven the way a user would — `/table` and a
// sized `/table2x4` from the slash menu, Tab and Enter in its source, the rendered
// table's add bars, the Table entries of the context menu on a rendered cell, and the
// palette's table commands. It works in a scratch vault and goes back to the user's
// vault before it ends.
import { api } from "$lib/api";
import { executeCommand } from "$lib/commands/registry";
import { t } from "$lib/i18n/i18n.svelte";
import { contextMenu, type MenuEntry, type MenuItem } from "$lib/stores/contextMenu.svelte";
import { vault } from "$lib/stores/vault.svelte";
import { startCompletion } from "@codemirror/autocomplete";
import { EditorView } from "@codemirror/view";
import { Probe, sleep } from "./index";

const NOTE = "Tables.md";

function noteView(): EditorView | null {
  for (const el of document.querySelectorAll<HTMLElement>(".cm-editor")) {
    const v = EditorView.findFromDOM(el);
    if (v?.state.doc.toString().startsWith("# Tables")) return v;
  }
  return null;
}

function key(view: EditorView, k: string, shift = false) {
  view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: k, shiftKey: shift, bubbles: true, cancelable: true }));
}

function type(view: EditorView, text: string) {
  view.dispatch(view.state.replaceSelection(text), { userEvent: "input.type" });
}

/** The doc with the cursor drawn in as ‸ (a selection as «…»). */
function show(view: EditorView): string {
  const s = view.state.doc.toString();
  const r = view.state.selection.main;
  return r.empty ? s.slice(0, r.head) + "‸" + s.slice(r.head) : s.slice(0, r.from) + "«" + s.slice(r.from, r.to) + "»" + s.slice(r.to);
}

/** Put the cursor on the note's last line, out of every table, so they render. */
function park(view: EditorView) {
  view.dispatch({ selection: { anchor: view.state.doc.length } });
}

const isItem = (e: MenuEntry): e is MenuItem => "label" in e;

export async function runMain() {
  const p = new Probe();
  p.log("table probe: main window");
  await sleep(1500);
  const originalVault = vault.root;
  let view: EditorView | null = null;

  await p.step("scratch vault with an empty note", async () => {
    const path = await api.probeScratchPath();
    await vault.createVault(path, "probe");
    await api.noteWrite(NOTE, "# Tables\n\n");
    await vault.refreshNotes();
    await vault.openNote(NOTE);
    p.check("the note is open", await p.until(() => (view = noteView()) !== null, 8000));
  });
  if (!view) {
    await p.done();
    return;
  }
  const v: EditorView = view;
  v.focus();

  await p.step("/table inserts a 3×3 table with the cursor in its first cell", async () => {
    park(v);
    type(v, "/table");
    startCompletion(v);
    p.check("the slash menu offers Table", await p.until(() => document.querySelector(".cm-tooltip-autocomplete li[aria-selected]")?.textContent?.startsWith("Table") === true));
    await sleep(150); // the popup takes no Enter in its first moments (interactionDelay)
    key(v, "Enter");
    p.check("a 3×3 table", await p.until(() => show(v) === "# Tables\n\n| ‸ |  |  |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |"), show(v));
  });

  await p.step("Tab steps through the cells, Enter adds a row and ends the table", async () => {
    type(v, "Fruit");
    key(v, "Tab");
    type(v, "Kg");
    key(v, "Tab");
    type(v, "Note");
    key(v, "Tab");
    p.check("Tab passes the delimiter row", show(v).includes("| Note |\n| --- | --- | --- |\n| ‸ |"), show(v));
    type(v, "Apple");
    key(v, "Tab");
    key(v, "Tab", true);
    p.check("Shift-Tab selects the cell before", show(v).includes("| «Apple» |"), show(v));
    // the last body row, then Tab past its last cell: a new row
    v.dispatch({ selection: { anchor: v.state.doc.length - 1 } });
    key(v, "Tab");
    p.check("Tab past the last cell opens a row", show(v).endsWith("|  |  |  |\n| ‸ |  |  |"), show(v));
    type(v, "Pear");
    key(v, "Enter");
    p.check("Enter opens a row below", show(v).endsWith("| Pear |  |  |\n| ‸ |  |  |"), show(v));
    key(v, "Enter");
    p.check("Enter on the empty last row ends the table", show(v).endsWith("| Pear |  |  |\n\n‸"), show(v));
  });

  await p.step("/table2x4 inserts two rows by four columns", async () => {
    type(v, "/table2x4");
    startCompletion(v);
    p.check("the menu names the size", await p.until(() => document.querySelector(".cm-tooltip-autocomplete li[aria-selected]")?.textContent?.startsWith("Table 2×4") === true));
    await sleep(150);
    key(v, "Enter");
    p.check("a 2×4 table", await p.until(() => show(v).endsWith("\n\n| ‸ |  |  |  |\n| --- | --- | --- | --- |\n|  |  |  |  |")), show(v));
    type(v, "A");
    v.dispatch({ changes: { from: v.state.doc.length, insert: "\n\nEnd" } });
    park(v);
  });

  const tables = () => [...document.querySelectorAll<HTMLElement>(".cm-lp-table")];
  await p.step("the tables render, with their add bars", async () => {
    p.check("two rendered tables", await p.until(() => tables().length === 2), tables().length);
    const bars = tables()[0]?.querySelectorAll(".cm-lp-table-add") ?? [];
    p.check("a row bar and a column bar", bars.length === 2);
    const style = document.createElement("style");
    style.textContent = ".cm-lp-table-add { opacity: 1 !important }";
    document.head.appendChild(style);
    await sleep(200);
    await p.shot("table-rendered");
    style.remove();
  });

  await p.step("the row bar adds a row and puts the cursor in it", async () => {
    const before = v.state.doc.lines;
    tables()[0].querySelector<HTMLElement>(".cm-lp-table-add-row")!.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true }));
    p.check("one more line", v.state.doc.lines === before + 1, v.state.doc.lines - before);
    p.check("the cursor is in the new row", show(v).includes("| Pear |  |  |\n| ‸ |  |  |"), show(v));
    park(v);
  });

  await p.step("the context menu on a rendered cell acts on that cell", async () => {
    await p.until(() => tables().length === 2);
    // the Kg cell of the header
    const cell = tables()[0].querySelectorAll<HTMLElement>("th")[1];
    const r = cell.getBoundingClientRect();
    cell.dispatchEvent(new MouseEvent("mousedown", { button: 2, bubbles: true, cancelable: true, clientX: r.x + 4, clientY: r.y + 4 }));
    p.check("a right press leaves the table rendered", tables().length === 2, tables().length);
    cell.dispatchEvent(new MouseEvent("contextmenu", { button: 2, bubbles: true, cancelable: true, clientX: r.x + 4, clientY: r.y + 4 }));
    const table = contextMenu.entries.filter(isItem).find((e) => e.label === t("table.menu"));
    p.check("the menu has Table ›", !!table?.submenu, contextMenu.entries.filter(isItem).map((e) => e.label));
    const sub = (table?.submenu ?? []).filter(isItem);
    p.check("delete row is off on the header", sub.find((e) => e.label === t("table.deleteRow"))?.disabled === true);
    await p.shot("table-menu");
    sub.find((e) => e.label === t("table.colRight"))?.action?.();
    contextMenu.close();
    p.check("a column right of Kg, the cursor in it", show(v).includes("| Fruit | Kg | ‸ | Note |\n| --- | --- | --- | --- |"), show(v));
  });

  await p.step("the palette's table commands act on the cursor's cell", async () => {
    await executeCommand("table-align-center");
    p.check("the column is centered", show(v).includes("| --- | --- | :---: | --- |"), show(v));
    await executeCommand("table-delete-col");
    p.check("the column is gone", show(v).includes("| Fruit | Kg | Note‸ |\n| --- | --- | --- |"), show(v));
    await p.shot("table-source");
    park(v);
    const before = v.state.doc.toString();
    await executeCommand("table-delete-row");
    p.check("outside a table nothing changes", v.state.doc.toString() === before);
  });

  await p.step("restore", async () => {
    if (originalVault) await vault.openVault(originalVault);
  });

  await p.done();
}
