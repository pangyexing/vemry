// "Ask the AI about this view": the active projection view's live query becomes a
// context chip (one per view — asking again replaces the earlier chip) and the composer
// is summoned, so the question is typed with the table already attached.
import { agent } from "$lib/stores/agent.svelte";
import { viewContext } from "$lib/stores/viewContext.svelte";
import { executeCommand } from "$lib/commands/registry";

export function askAboutView() {
  const ref = viewContext.ref();
  if (!ref) return;
  agent.contextDraft = [...agent.contextDraft.filter((d) => !(d.kind === "view" && d.path === ref.path)), ref];
  void executeCommand("focus-agent");
}
