// Platform-specific formatting of shortcut hints. The key handling itself has long
// been cross-platform (metaKey||ctrlKey, CodeMirror's Mod-), and this only produces
// the hint text: mac shows the compact glyphs (⌘⇧C), Windows/Linux the textual form
// (Ctrl+Shift+C).
export const isMac = /Mac/.test(navigator.platform || navigator.userAgent);

// Ctrl is the literal control key (⌃ on mac), distinct from Mod — the tab-cycling
// bindings use it on every platform.
const MAC: Record<string, string> = { Mod: "⌘", Shift: "⇧", Alt: "⌥", Ctrl: "⌃", Backspace: "⌫" };
const PC: Record<string, string> = { Mod: "Ctrl", Shift: "Shift", Alt: "Alt", Ctrl: "Ctrl", Backspace: "Backspace" };

/** k("Mod+Shift+C") → mac "⌘⇧C" / win&linux "Ctrl+Shift+C"; an unmapped token (a letter, ←, ., Enter, click) passes through unchanged */
export function k(combo: string): string {
  const map = isMac ? MAC : PC;
  return combo
    .split("+")
    .map((t) => map[t] ?? t)
    .join(isMac ? "" : "+");
}
