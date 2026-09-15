// Quick capture's shared pieces: the default landing spot from the settings, the
// attachment name, and the global shortcut in its two forms — the plugin's grammar
// ("Alt+Shift+Space", "Super+KeyK": what settings.json stores and Rust registers)
// and the label the user reads. The capture window lands with the first, the
// settings page records with the last, and the activity bar and the palette row
// show the label; keeping the three here means the grammar exists once.
import { k } from "$lib/shortcut";
import { prefs } from "$lib/stores/prefs.svelte";
import { localDate } from "$lib/views/tasks";

/** settings.rs default_capture_hotkey; the settings page's "reset" puts it back. */
export const CAPTURE_HOTKEY_DEFAULT = "Alt+Shift+Space";

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
      return { path: `journal/${localDate()}.md`, heading: null };
    case "pinned":
      return prefs.capturePinnedPath
        ? { path: prefs.capturePinnedPath, heading: prefs.capturePinnedHeading || null }
        : { path: "inbox.md", heading: null };
    default:
      return { path: "inbox.md", heading: null };
  }
}

/** `note.md › Section` (the receipt's and the target row's spelling). */
export function spotLabel(s: CaptureSpot): string {
  return s.heading ? `${s.path} › ${s.heading}` : s.path;
}

/** `capture-YYYYMMDD-HHMMSS.<ext>` for an image that arrived as bytes (a screenshot);
 *  the backend deduplicates a second one in the same second. */
export function captureImageName(ext: string): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `capture-${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}.${ext}`;
}

/** The image format of clipboard bytes, from their magic number (the reader
 *  prefers PNG; TIFF is what an older copy source may offer). */
export function sniffImageExt(bytes: Uint8Array): string {
  const [a, b, c, d] = bytes;
  if (a === 0x89 && b === 0x50 && c === 0x4e && d === 0x47) return "png";
  if (a === 0xff && b === 0xd8) return "jpg";
  if (a === 0x47 && b === 0x49 && c === 0x46) return "gif";
  if ((a === 0x49 && b === 0x49 && c === 0x2a) || (a === 0x4d && b === 0x4d && c === 0x00)) return "tiff";
  if (a === 0x52 && b === 0x49 && c === 0x46 && d === 0x46) return "webp";
  return "png";
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

/** A recorded keydown → the plugin's grammar, or null when the press is not a
 *  usable global shortcut: a modifier alone (the key is still coming), or a key
 *  with no modifier at all (that would take a letter away from every application).
 *  The key travels as the physical `code` — layout-independent, and the names the
 *  registrar accepts (Space, KeyK, Digit1, F5, ArrowUp, Comma…). */
export function comboFromEvent(e: KeyboardEvent): string | null {
  if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return null;
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  if (!mods.length || !e.code) return null;
  return [...mods, e.code].join("+");
}
