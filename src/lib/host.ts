// The single exit for actions that "have to happen in the main window". The editor,
// the clip destination and the command table live in the main window alone; when
// "open the note / insert / pin / # convention / a page-card command" is pressed in
// the detached chat window, the action is relayed through Rust
// (agent_window_to_main → agent-window://action) back to the main window to run,
// while the main window calling it runs it directly — both paths share
// runHostAction, so the semantics exist once.
import { t } from "$lib/i18n/i18n.svelte";
import { api, type HostAction } from "$lib/api";
import { isAgentWindow } from "$lib/window";
import { vault } from "$lib/stores/vault.svelte";
import { clipTarget } from "$lib/stores/clipTarget.svelte";
import { toast } from "$lib/stores/toast.svelte";
import { executeCommand } from "$lib/commands/registry";

/** For a navigation or command action the user wants to see the result: relaying it also brings the main window to the front. */
const FOCUS_MAIN = new Set<HostAction["kind"]>(["open-note", "follow-link", "command", "review-start"]);

/** Run the action in the right window. From the chat window it is a one-way delivery
 *  (the result and any error toast in the main window); in the main window it runs
 *  synchronously and throws its error to the caller. */
export async function hostDispatch(a: HostAction): Promise<void> {
  if (isAgentWindow) {
    try {
      await api.agentWindowToMain(a, FOCUS_MAIN.has(a.kind));
    } catch (e) {
      toast.push(String(e), "error");
    }
    return;
  }
  await runHostAction(a);
}

/** Text the user placed from the chat is the AI's, accepted by the placing itself:
 *  the ledger line names the session and keeps the blocks out of "pending". Failing to
 *  record must not undo an insert that already landed, so errors are swallowed. */
function recordAgentText(path: string | null, text: string, session?: string) {
  if (!path) return;
  void api.provenanceRecord(path, text, "agent", session).catch(() => undefined);
}

/** The main-window side (App's listener for agent-window://action comes through here too). */
export async function runHostAction(a: HostAction): Promise<void> {
  switch (a.kind) {
    case "open-note":
      await vault.openNote(a.path);
      return;
    case "review-start":
      await vault.reviewStart(a.paths);
      return;
    case "review-accept":
      await vault.acceptProvenanceNotes(a.paths);
      return;
    case "follow-link":
      await vault.followLink(a.target, { guard: a.guard });
      return;
    case "insert":
      // It has to pass the canInsertIntoVisibleNote guard — under a projection the
      // editor is only hidden by CSS, and a bare insertAtCursor cannot tell a visible
      // note from a hidden one
      if (vault.canInsertIntoVisibleNote && vault.insertAtCursor?.(a.payload, true)) {
        toast.push(t("app.insertedInto", { path: vault.currentPath ?? "" }));
        recordAgentText(vault.currentPath, a.payload, a.session);
      } else {
        toast.push(t("app.noVisibleEditor"), "error");
      }
      return;
    case "pin": {
      // Shares clipTarget.deliver's pipeline and destination contract with a clipped
      // selection (fresh: a note just created and pinned from the toolbar may not have
      // reached this window's notes mirror yet, and must not be killed by mistake)
      if (!clipTarget.path || !(await clipTarget.ensureLiveFresh())) {
        toast.push(t("chat.pinFirst"));
        return;
      }
      const target = clipTarget.path;
      const ok = await clipTarget.deliver(target, a.payload, undefined, clipTarget.heading);
      if (ok) {
        recordAgentText(target, a.payload, a.session);
        await vault.refreshNotes();
      }
      return;
    }
    case "memo":
      // With VEMRY.md open and holding unsaved edits, flush first: an append on disk is
      // not reloaded into the editor for a dirty note (vault.handleEvent), and the
      // autosave that follows would overwrite the convention just recorded with a stale
      // buffer
      try {
        if (vault.currentPath === "VEMRY.md" && vault.dirty) await vault.flushEditor?.();
        await api.memoryAppend(a.text);
        toast.push(t("app.memoRecorded"));
      } catch (e) {
        toast.push(String(e), "error");
        throw e; // the main-window path: the caller gives the input back to the user from it
      }
      return;
    case "command":
      await executeCommand(a.id);
      return;
  }
}
