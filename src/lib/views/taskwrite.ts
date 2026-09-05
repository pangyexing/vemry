// The write funnel every task view shares (the agenda's rows, the kanban's cards): the
// note being edited is saved first, so unsaved changes are not overwritten (with several
// tabs only the active one can be dirty, so the meaning holds); a stale span (the note
// changed after the query) writes nothing and requeries, so the user retries on the new
// data; an error lands in the vault's error slot. It lives beside tasks.ts rather than in
// it: tasks.ts cannot import the vault store (the store imports ymd from it), and this is
// the one piece of the task views' shared code that needs the store.
import { t } from "$lib/i18n/i18n.svelte";
import { k } from "$lib/shortcut";
import { vault } from "$lib/stores/vault.svelte";
import type { BlockHit } from "$lib/api";

/** `act` is the wording of the action for the dirty guard's message. Returns true only
 *  when the write went to disk: a drop's provisional card waits for that write's reload,
 *  and every other outcome has no reload coming. */
export async function commitTaskWrite(
  hit: BlockHit,
  act: string,
  write: () => Promise<boolean>,
  requery: () => Promise<void>,
): Promise<boolean> {
  if (hit.note === vault.currentPath && vault.dirty) {
    vault.error = t("task.dirtyGuard", { key: k("Mod+S"), action: act });
    return false;
  }
  try {
    if (await write()) return true;
    await requery();
  } catch (err) {
    vault.error = String(err);
  }
  return false;
}
