// The single state machine of in-app updates. The settings page and the "check for
// updates" command share this logic; rule 5 of the controlled network egress: a check
// or a download is only ever started by the two explicit actions here — check() (a
// user click, or one silent check at startup once updater_auto_check is on) and
// downloadAndInstall() (always a user click). There is no background path at all.
import { t } from "$lib/i18n/i18n.svelte";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { toast } from "$lib/stores/toast.svelte";

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
  /** The version running now (from tauri.conf, read once at startup). */
  current = $state("");
  /** The new version found and its release notes. */
  version = $state("");
  notes = $state("");
  /** The download progress (in bytes); a null total means the server gave no Content-Length. */
  progress = $state(0);
  total = $state<number | null>(null);
  error = $state("");
  #update: Update | null = null;

  constructor() {
    void getVersion().then((v) => (this.current = v)).catch(() => {});
  }

  /** interactive=false is the silent check at startup: it fails silently and only toasts when there really is a new version. */
  async check(interactive = true) {
    if (this.phase === "checking" || this.phase === "downloading") return;
    this.phase = "checking";
    this.error = "";
    try {
      const u = await check();
      if (u) {
        this.#update = u;
        this.version = u.version;
        this.notes = u.body ?? "";
        this.phase = "available";
        if (!interactive) toast.push(t("vaultStore.updateAvailable", { version: u.version }));
      } else {
        this.#update = null;
        this.phase = "uptodate";
      }
    } catch (e) {
      this.phase = "error";
      this.error = String(e);
      if (interactive) toast.push(t("vaultStore.updateCheckFailed", { error: String(e) }), "error");
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
      this.error = String(e);
    }
  }

  async restart() {
    await relaunch();
  }
}

export const updater = new UpdaterStore();
