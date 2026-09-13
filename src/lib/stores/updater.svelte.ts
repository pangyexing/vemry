// The single state machine of in-app updates, and the one surface every phase shows
// on: the update dialog (UpdateDialog.svelte). The settings page only mirrors it.
// Rule 5 of the controlled network egress: a check or a download is only ever started
// by the two explicit actions here — check() (a user click, or one silent check at
// startup once updater_auto_check is on) and downloadAndInstall() (always a user
// click). There is no background path at all.
import { api, type SettingsView } from "$lib/api";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { toast } from "$lib/stores/toast.svelte";
import { vault } from "$lib/stores/vault.svelte";
import { t } from "$lib/i18n/i18n.svelte";

export type UpdatePhase =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "ready"
  | "uptodate"
  | "error";

class UpdaterStore {
  phase = $state<UpdatePhase>("idle");
  /** The update dialog is showing. A check the user asked for opens it at once (so the
   *  wait is visible); the silent startup check opens it only on a new version that was
   *  not skipped. Closing it never stops anything — a download sent to the background
   *  announces its outcome as a toast, and the dialog comes back through the settings
   *  page or the check command (never on its own: an Enter typed in the editor must not
   *  land on a restart button that appeared under the keys). */
  open = $state(false);
  /** The version running now (from tauri.conf, read once at startup). */
  current = $state("");
  /** The new version found and its release notes. */
  version = $state("");
  notes = $state("");
  /** The download progress (in bytes); a null total means the server gave no Content-Length. */
  progress = $state(0);
  total = $state<number | null>(null);
  error = $state("");
  /** Which step failed: the dialog titles a failed check and a failed download differently. */
  errorStage = $state<"check" | "download">("check");
  /** The mirror of updater_auto_check behind the dialog's checkbox: seeded from the
   *  startup settings fetch, written straight through by setAutoCheck, and kept in step
   *  by SettingsModal after its own save. */
  autoCheck = $state(false);
  /** The version "skip this version" muted (settings.updater_skipped_version): the
   *  silent check does not open the dialog for it; a check the user asks for still does. */
  skipped = $state<string | null>(null);
  #update: Update | null = null;

  constructor() {
    void getVersion().then((v) => (this.current = v)).catch(() => {});
  }

  /** The startup path hands over the settings it fetched anyway (no second IPC). */
  applySettings(s: SettingsView) {
    this.autoCheck = s.updater_auto_check;
    this.skipped = s.updater_skipped_version;
  }

  /** interactive=false is the silent check at startup: it fails silently and only opens the dialog on a new version. */
  async check(interactive = true) {
    // A download in flight or an update waiting for its restart: a new check would only
    // find the same version again, so an ask just shows where things stand
    if (this.phase === "checking" || this.phase === "downloading" || this.phase === "ready") {
      if (interactive) this.open = true;
      return;
    }
    this.phase = "checking";
    this.error = "";
    // Opened before the answer, so the wait is visible; closing it meanwhile discards
    // the answer from view (the settings page still reflects it)
    if (interactive) this.open = true;
    try {
      const u = await check();
      if (u) {
        this.#update = u;
        this.version = u.version;
        this.notes = u.body ?? "";
        this.phase = "available";
        if (!interactive && u.version !== this.skipped) this.open = true;
      } else {
        this.#update = null;
        this.phase = "uptodate";
      }
    } catch (e) {
      this.phase = "error";
      this.errorStage = "check";
      this.error = String(e);
    }
  }

  /** Show the dialog for the state already in hand (the settings page's "install" button). */
  show() {
    this.open = true;
  }

  /** Close the dialog and leave everything as it is. */
  close() {
    this.open = false;
  }

  /** "Skip this version": the silent startup check stays quiet about this one; a
   *  later version, or a check the user asks for, shows it again. */
  async skip() {
    this.skipped = this.version;
    this.open = false;
    try {
      await api.settingsSet({ updater_skipped_version: this.version });
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  /** The dialog's checkbox writes the setting at once (no save button to reach). */
  async setAutoCheck(on: boolean) {
    this.autoCheck = on;
    try {
      await api.settingsSet({ updater_auto_check: on });
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  /** Download and install (the minisign signature check happens inside the plugin), then wait for the user to press restart. */
  async downloadAndInstall() {
    const u = this.#update;
    if (!u || this.phase === "downloading") return;
    this.phase = "downloading";
    this.progress = 0;
    this.total = null;
    try {
      await u.downloadAndInstall((ev) => {
        if (ev.event === "Started") this.total = ev.data.contentLength ?? null;
        else if (ev.event === "Progress") this.progress += ev.data.chunkLength;
      });
      this.phase = "ready";
    } catch (e) {
      this.phase = "error";
      this.errorStage = "download";
      this.error = String(e);
    }
    // The plugin has no way to abort a download, so "continue in background" only
    // hides the dialog; the outcome then arrives as a toast (the dialog itself is
    // reopened by the user, from the settings page or the check command), while a
    // dialog left open just moves on to its next phase
    if (!this.open) {
      if (this.phase === "ready") toast.push(t("updater.readyBody", { version: this.version }));
      else toast.push(`${t("updater.installFailedTitle")}: ${this.error}`, "error");
    }
  }

  /** A relaunch exits with a code the quit handshake lets pass, so the buffer is written out here first. */
  async restart() {
    await vault.flushEditor?.();
    await relaunch();
  }
}

export const updater = new UpdaterStore();
