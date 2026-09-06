// A scaffold command (a block type, a new or customised skill) writes its file once and opens it
// in the editor. A file already there is the end state the command exists to reach, so it
// is opened too — with a toast saying so — rather than reported as a failure. Shared by
// App.svelte's commands and the palette's "block type…" rows.
import type { Scaffolded } from "$lib/api";
import { t } from "$lib/i18n/i18n.svelte";
import { toast } from "$lib/stores/toast.svelte";
import { vault } from "$lib/stores/vault.svelte";

export async function scaffold(write: () => Promise<Scaffolded>): Promise<void> {
  try {
    const { path, created } = await write();
    toast.push(t(created ? "app.wroteFile" : "app.existsOpened", { path }));
    await vault.openFile(path);
  } catch (e) {
    toast.push(String(e), "error");
  }
}
