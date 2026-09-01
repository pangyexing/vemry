// User-script UI state: the loaded-script list (settings page + palette
// sync) and the two blocking dialogs (write grant / vemry.prompt). The
// script thread runs strictly serially, so at most one of each is pending.

import { t } from "$lib/i18n/i18n.svelte";
import { api, type ScriptInfo } from "$lib/api";
import { toast } from "$lib/stores/toast.svelte";

export interface GrantRequest {
  id: string;
  script: string;
  path: string;
}

export interface PromptRequest {
  id: string;
  script: string;
  message: string;
  placeholder: string | null;
}

class ScriptsStore {
  infos = $state<ScriptInfo[]>([]);
  pendingGrant = $state<GrantRequest | null>(null);
  pendingPrompt = $state<PromptRequest | null>(null);

  async refresh() {
    try {
      this.infos = await api.scriptsList();
    } catch {
      this.infos = [];
    }
  }

  async respondGrant(allow: boolean) {
    const req = this.pendingGrant;
    if (!req) return;
    this.pendingGrant = null;
    try {
      await api.scriptGrantRespond(req.id, allow);
    } catch {
      // the request already expired (300s timeout / vault switch): the script
      // proceeded as DENIED — silently closing the dialog would let the user
      // believe an "allow" took effect
      if (allow) toast.push(t("vaultStore.grantExpired", { script: req.script }), "error");
    }
  }

  async respondPrompt(value: string | null) {
    const req = this.pendingPrompt;
    if (!req) return;
    this.pendingPrompt = null;
    try {
      await api.scriptPromptRespond(req.id, value);
    } catch {
      if (value != null) toast.push(t("vaultStore.promptExpired", { script: req.script }), "error");
    }
  }
}

export const scripts = new ScriptsStore();
