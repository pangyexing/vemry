// The frontend mirror of located insertion — the rules match
// vemry-core/src/insert.rs and change together with it: a section runs from its
// heading line to the next heading of a level ≤ its own (otherwise to the end of the
// file); the destination is after the section's last non-empty line; separators are
// normalised (blank lines around a block payload, a single-line list item packed
// against a list or a heading line, and exactly one blank line before what follows).
// Why mirror it at all: when the target is the note that is open, the insertion has
// to happen in the CodeMirror buffer (a write to disk would be overwritten by the
// autosave of a stale buffer), and the Rust primitive cannot reach the buffer.
// The only difference from Rust is the offset unit (UTF-16 characters here, bytes
// there); the rules themselves are whatever insert.rs's table of cases says.

import { stripAnchor } from "$lib/fields";

export interface InsertPlan {
  from: number;
  to: number;
  insert: string;
  /** Where the cursor should land after the insertion (the end of the payload). */
  caret: number;
  landed: "section" | "tail";
}

/** An ATX heading's level; null when it is not a heading (mirroring parser.rs's
 *  parse_heading: 1-6 #s followed by a space, with leading indentation allowed). */
function headingLevel(line: string): number | null {
  const t = line.trimStart();
  let n = 0;
  while (n < t.length && t[n] === "#") n++;
  return n >= 1 && n <= 6 && t[n] === " " ? n : null;
}

/** A heading's text minus a trailing ` ^id` (locate.rs::heading_text). */
function headingText(line: string): string {
  const t = line.trimStart();
  let n = 0;
  while (t[n] === "#") n++;
  return stripAnchor(t.slice(n).trim());
}

/** parser.rs::is_thematic_break: three or more of one of `-*_`, spaces between allowed. */
function isThematicBreak(t: string): boolean {
  return /^([-*_])(?:[ \t]*\1)*[ \t]*$/.test(t) && t.replace(/[ \t]/g, "").length >= 3;
}

/** parser.rs::parse_list_marker: a bullet that is not a spaced thematic break (`- - -`),
 *  or an ordered marker of at most nine digits. */
function isListLine(line: string): boolean {
  const t = line.trimStart();
  if (/^[-*+] /.test(t)) return !isThematicBreak(t);
  return /^\d{1,9}[.)] /.test(t);
}

/** The separator between a block payload and the line before it: packed against a
 *  heading line and against a list item for a single-line list payload, and one blank
 *  line otherwise (the trailing newline included, spliced straight after the previous
 *  line's end). */
export function separatorFor(prevLine: string, payload: string): string {
  const tight =
    headingLevel(prevLine) != null ||
    (!payload.includes("\n") && isListLine(payload) && isListLine(prevLine));
  return tight ? "\n" : "\n\n";
}

/** Strip the bare newlines from both ends of a payload; the separators are rebuilt by the normalisation. */
export function trimPayload(text: string): string {
  return text.replace(/^[\n\r]+|[\n\r]+$/g, "");
}

/** Whether the payload can pack against the next line (a run of list items only);
 *  otherwise a cursor block insertion has to add a blank line after the payload — a
 *  quote block directly against what follows would suck the next paragraph into a lazy
 *  continuation. */
export function tightBelow(payload: string, nextLine: string): boolean {
  return !payload.includes("\n") && isListLine(payload) && isListLine(nextLine);
}

/** The offset of the end of the `heading` section's content; null when the heading
 *  does not exist. It skips the frontmatter (a YAML comment `# x` would be mistaken
 *  for a heading) and phantom headings inside code fences, and takes the first heading
 *  of a repeated name in document order, an exact match before a case-insensitive one
 *  (locate.rs::find_heading) — all matching how the Rust side parses. */
export function sectionInsertOffset(doc: string, heading: string): number | null {
  const want = heading.trim();
  const headings = scanHeadings(doc);
  let at = headings.findIndex((h) => h.text === want);
  if (at < 0) {
    const lower = want.toLowerCase();
    at = headings.findIndex((h) => h.text.toLowerCase() === lower);
  }
  if (at < 0) return null;
  const level = headings[at].level;
  const next = headings.slice(at + 1).find((h) => h.level <= level);
  return next ? doc.slice(0, next.from).trimEnd().length : doc.trimEnd().length;
}

/** The headings of `doc` in order: level, text and the offset their line starts at. */
function scanHeadings(doc: string): { level: number; text: string; from: number }[] {
  const out: { level: number; text: string; from: number }[] = [];
  const lines = doc.split("\n");
  let pos = 0;
  let i = 0;
  if (lines[0]?.trimEnd() === "---") {
    for (let j = 1; j < lines.length; j++) {
      const t = lines[j].trimEnd();
      if (t === "---" || t === "...") {
        for (let k = 0; k <= j; k++) pos += lines[k].length + 1;
        i = j + 1;
        break;
      }
    }
  }
  let fence: { ch: string; len: number } | null = null;
  for (; i < lines.length; i++) {
    const line = lines[i];
    const t = line.trimStart();
    if (fence) {
      const m = t.match(/^(`{3,}|~{3,})\s*$/);
      if (m && m[1][0] === fence.ch && m[1].length >= fence.len) fence = null;
    } else {
      const open = t.match(/^(`{3,}|~{3,})/);
      if (open) {
        fence = { ch: open[1][0], len: open[1].length };
      } else {
        const level = headingLevel(line);
        if (level != null) out.push({ level, text: headingText(line), from: pos });
      }
    }
    pos += line.length + 1;
  }
  return out;
}

export function contentEndOffset(doc: string): number {
  return doc.trimEnd().length;
}

/** The minimal read surface of CodeMirror's Text (this module does not depend on CM; structural compatibility is enough). */
export interface DocReader {
  readonly lines: number;
  lineAt(pos: number): { number: number; from: number; to: number; text: string };
  line(n: number): { number: number; from: number; to: number; text: string };
}

/** The cursor-anchored block insertion plan (used when a clip or a memo lands in "the
 *  current note with no section"): on a blank line it lands in place; on a non-empty
 *  line it lands after the end of the line. The separator rules follow the same
 *  normalisation as planInsert: above, it goes by separatorFor (a blank line already
 *  provides one newline, so only "is a blank line needed" is added); below, a
 *  non-empty line that would not make a run of list items gets a blank line. */
export function planCursorInsert(
  doc: DocReader,
  at: number,
  text: string,
): { from: number; to: number; insert: string; caret: number } {
  const line = doc.lineAt(at);
  const payload = trimPayload(text);
  const empty = line.text.trim() === "";
  const prev = empty ? (line.number > 1 ? doc.line(line.number - 1).text : "") : line.text;
  const pre = empty
    ? prev.trim() !== "" && separatorFor(prev, payload) === "\n\n"
      ? "\n"
      : ""
    : separatorFor(line.text, payload);
  const next = line.number < doc.lines ? doc.line(line.number + 1).text : "";
  const post = next.trim() !== "" && !tightBelow(payload, next) ? "\n" : "";
  const from = empty ? line.from : line.to;
  return { from, to: line.to, insert: pre + payload + post, caret: from + pre.length + payload.length };
}

/** Produce the minimal CM change for one located insertion. A null heading means the
 *  end of the file; a missing heading falls back to the end of the file and says so
 *  through landed (never silently, never as an error). Any existing trailing blank
 *  lines are eaten whole and the separators are rebuilt. */
export function planInsert(doc: string, heading: string | null, text: string): InsertPlan {
  const sec = heading != null ? sectionInsertOffset(doc, heading) : null;
  const landed: "section" | "tail" = sec != null ? "section" : "tail";
  const from = sec ?? contentEndOffset(doc);
  // from sits at the end of the content (mid-line), so the rest of the current line and the blank lines after it are eaten up to the start of the next non-empty line
  let to = from;
  while (to < doc.length) {
    const nl = doc.indexOf("\n", to);
    const lineEnd = nl === -1 ? doc.length : nl;
    if (doc.slice(to, lineEnd).trim() !== "") break;
    to = nl === -1 ? doc.length : nl + 1;
  }
  const payload = trimPayload(text);
  const before = doc.slice(0, from);
  const sep = before ? separatorFor(before.slice(before.lastIndexOf("\n") + 1), payload) : "";
  const insert = sep + payload + "\n" + (to >= doc.length ? "" : "\n");
  return { from, to, insert, caret: from + sep.length + payload.length, landed };
}
