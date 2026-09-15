// Platform-specific formatting of shortcut hints. The key handling itself has long
// been cross-platform (metaKey||ctrlKey, CodeMirror's Mod-), and this only produces
// the hint text: mac shows the compact glyphs (⌘⇧C), Windows/Linux the textual form
// (Ctrl+Shift+C), with the modifier names from the catalog so a German interface
// says Strg+Umschalt+C.
import { t } from "$lib/i18n/i18n.svelte";

export const isMac = /Mac/.test(navigator.platform || navigator.userAgent);

// Ctrl is the literal control key (⌃ on mac), distinct from Mod — the tab-cycling
// bindings use it on every platform.
const MAC: Record<string, string> = { Mod: "⌘", Shift: "⇧", Alt: "⌥", Ctrl: "⌃", Backspace: "⌫" };
function pc(token: string): string | undefined {
  switch (token) {
    case "Mod":
    case "Ctrl":
      return t("key.ctrl");
    case "Shift":
      return t("key.shift");
    case "Alt":
      return t("key.alt");
    case "Backspace":
      return t("key.backspace");
    default:
      return undefined;
  }
}

/** k("Mod+Shift+C") → mac "⌘⇧C" / win&linux "Ctrl+Shift+C"; an unmapped token (a letter, ←, ., Enter, click) passes through unchanged */
export function k(combo: string): string {
  return combo
    .split("+")
    .map((t) => (isMac ? MAC[t] : pc(t)) ?? t)
    .join(isMac ? "" : "+");
}
