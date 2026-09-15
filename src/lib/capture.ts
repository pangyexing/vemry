// Quick capture's shared pieces: the default landing spot from the settings, and
// the global shortcut in its two forms — the plugin's grammar ("Alt+Shift+Space",
// "Super+KeyK": what settings.json stores and Rust registers) and the label the
// user reads. The capture window lands with the first, the settings page records
// with the last, and the activity bar and the palette row show the label; keeping
// the three here means the grammar exists once.
import { k } from "$lib/shortcut";
import { prefs } from "$lib/stores/prefs.svelte";
import { journalToday } from "$lib/views/tasks";

/** A note and, optionally, the heading of the section a capture lands in. */
export interface CaptureSpot {
  path: string;
  heading: string | null;
}

/** Where Enter lands a capture, from the settings: inbox.md, today's journal note
 *  (the same journal/YYYY-MM-DD.md the daily skills write), or the pinned note and
 *  section — a pin with no note set falls back to inbox.md rather than failing. */
export function defaultCaptureSpot(): CaptureSpot {
  switch (prefs.captureTarget) {
    case "journal":
      return { path: journalToday(), heading: null };
    case "pinned": {
      // The pin is typed by hand (the settings field): a path without the extension is the
      // same note to Rust (note_id appends it) but not to the main window's editor-buffer check
      // (currentPath carries it), and a miss there writes the disk copy under an open note
      const p = prefs.capturePinnedPath.trim();
      return p ? { path: /\.md$/i.test(p) ? p : `${p}.md`, heading: prefs.capturePinnedHeading || null } : { path: "inbox.md", heading: null };
    }
    default:
      return { path: "inbox.md", heading: null };
  }
}

/** `note.md › Section` (the receipt's and the target row's spelling). */
export function spotLabel(s: CaptureSpot): string {
  return s.heading ? `${s.path} › ${s.heading}` : s.path;
}

/** The shortcut as the user reads it: modifiers as the platform's glyphs or names
 *  (⌥⇧Space, Alt+Shift+Space) and the key by its plain name (KeyK → K, Digit1 → 1,
 *  ArrowUp → ↑). */
export function hotkeyLabel(combo: string): string {
  const parts = combo.split("+").map((p) => p.trim()).filter(Boolean);
  const key = parts.pop() ?? "";
  return k([...parts, keyName(key)].join("+"));
}

const ARROWS: Record<string, string> = { ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→" };
function keyName(code: string): string {
  if (/^Key[A-Z]$/i.test(code)) return code.slice(3).toUpperCase();
  if (/^Digit\d$/i.test(code)) return code.slice(5);
  return ARROWS[code] ?? code;
}

/** A modifier pressed on its own: the key of the combination is still coming. */
export function isModifierKey(e: KeyboardEvent): boolean {
  return ["Shift", "Control", "Alt", "Meta"].includes(e.key);
}

/** A recorded keydown → the plugin's grammar, or null when the press is not a
 *  usable global shortcut: a modifier alone (`isModifierKey`), or a key with no
 *  modifier at all (that would take a letter away from every application). The
 *  key travels as the physical `code` — layout-independent, and the names the
 *  registrar accepts (Space, KeyK, Digit1, F5, ArrowUp, Comma…). */
export function comboFromEvent(e: KeyboardEvent): string | null {
  if (isModifierKey(e)) return null;
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  if (!mods.length || !e.code) return null;
  return [...mods, e.code].join("+");
}
