// The one definition of the context menu's privacy item, shared by the file tree's row
// menu and the editor's menu (a plain module, following pickNote.ts): "mark" when the
// path is not private, "unmark" when it is a zone root, and a disabled explanation when
// it is inherited — Rust removes only the exact rule line, so "unmark" on an inherited
// row would be a silent no-op that redraws the same state.
import { t } from "$lib/i18n/i18n.svelte";
import { vault } from "$lib/stores/vault.svelte";
import type { MenuItem } from "$lib/stores/contextMenu.svelte";

export function privacyMenuItem(path: string, run: (op: "mark" | "unmark") => void): MenuItem {
  const st = vault.privacyStatus(path);
  if (st.kind === "inherited") {
    return { label: t("fileTree.privacyCovered", { zone: `${st.zone}/` }), disabled: true, action: () => {} };
  }
  return st.kind === "root"
    ? { label: t("fileTree.unmarkPrivate"), action: () => run("unmark") }
    : { label: t("fileTree.markPrivate"), action: () => run("mark") };
}
