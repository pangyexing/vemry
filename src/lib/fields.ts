// Inline `key:: value` fields as the editor sees them — a mirror of vemry-core's
// `scan_fields` (parser.rs): the same three forms, the same boundary rule (`::`
// followed by whitespace, a digit or the end of the line), the same exclusions
// (backtick spans, `[[wikilinks]]`; the caller keeps code blocks and frontmatter
// out). Offsets are UTF-16 character positions. The Rust table of accepted and
// rejected examples is replayed against scanFields by the batch's harness, so the
// two cannot drift silently. Pure: no CodeMirror, no stores (BlockType is a type-only
// import and is erased).
import type { BlockType } from "$lib/api";

export type FieldForm = "line" | "bracket" | "paren" | "bare";

export interface FieldMark {
  key: string;
  value: string;
  /** The whole construct: `key:: value` to the end of the line, `[key:: v]`, `(key:: v)`, or `key:: token`. */
  from: number;
  to: number;
  /** Where the value text sits inside the construct (for an in-place rewrite). */
  valueFrom: number;
  valueTo: number;
  form: FieldForm;
}

const KEY_CHAR = /[\p{L}\p{N}_-]/u;
const BLOCK_ID_RE = /^[A-Za-z0-9-]+$/;
/** A trailing ` ^id` anchor, as the parser accepts it (blockid.rs::split_anchor): one
 *  horizontal space, then the id alphabet, at the end of the line. */
export const ANCHOR_TAIL_RE = /[ \t]\^[A-Za-z0-9-]+$/;

/** `line` without its trailing ` ^id` anchor (the mirror of blockid.rs::strip_anchor). */
export function stripAnchor(line: string): string {
  return line.replace(ANCHOR_TAIL_RE, "").trimEnd();
}

export function isFieldKey(s: string): boolean {
  if (!s) return false;
  let allDigits = true;
  for (const ch of s) {
    if (!KEY_CHAR.test(ch)) return false;
    if (!/\d/.test(ch)) allDigits = false;
  }
  return !allDigits;
}

function leadingWs(s: string): number {
  const m = /^[ \t]*/.exec(s);
  return m ? m[0].length : 0;
}

/** `- `, `* `, `+ `, `1. `, `1) ` → the marker's length including the space after it (mirrors parse_list_marker). */
function listMarkerLen(s: string): number {
  const m = /^(?:[-*+]|\d{1,9}[.)])[ \t]+/.exec(s);
  return m ? m[0].length : 0;
}

/** Where a line's own text starts: past indentation, quote markers, a list marker and a checkbox (mirrors block_content_start). */
export function blockContentStart(line: string): number {
  let i = leadingWs(line);
  for (;;) {
    const rest = line.slice(i);
    if (rest.startsWith(">")) {
      i += 1 + leadingWs(rest.slice(1));
      continue;
    }
    const mk = listMarkerLen(rest);
    if (mk) {
      i += mk;
      const after = line.slice(i);
      if (after[0] === "[" && after[2] === "]") i += 3 + leadingWs(after.slice(3));
      continue;
    }
    return i;
  }
}

export interface ScanOptions {
  /** Also report a field whose value is empty — `[key:: ]` or `[key::]`, or `key::`
   *  alone on its line — the placeholder a block type leaves behind, or a bracket the
   *  user closed with nothing typed. The index never holds one
   *  (the Rust scanner drops it, and so does this one by default: the mirror stays
   *  exact); only the editor's chip and setFieldValue's in-place fill ask for it. A
   *  bare `key::` at the end of a line is never reported: an empty token is nothing. */
  empty?: boolean;
}

/** Every field of a text (one or more lines), offsets relative to `base`. */
export function scanFields(text: string, base = 0, opts: ScanOptions = {}): FieldMark[] {
  const out: FieldMark[] = [];
  if (!text.includes("::")) return out;
  let inCode = false;
  let lineStart = 0;
  for (const line of text.split("\n")) {
    const contentStart = blockContentStart(line);
    let i = 0;
    while (i < line.length) {
      const ch = line[i];
      if (ch === "`") {
        inCode = !inCode;
        i++;
        continue;
      }
      if (inCode) {
        i++;
        continue;
      }
      if (line.startsWith("[[", i)) {
        const end = line.indexOf("]]", i + 2);
        if (end < 0) break;
        i = end + 2;
        continue;
      }
      if (!line.startsWith("::", i)) {
        i++;
        continue;
      }
      const after = line.slice(i + 2);
      let keyStart = i;
      while (keyStart > 0 && KEY_CHAR.test(line[keyStart - 1])) keyStart--;
      const key = line.slice(keyStart, i);
      const before = keyStart > 0 ? line[keyStart - 1] : undefined;
      // `[key::]` — closed with nothing after the colons — is no field to the index (the
      // rule wants whitespace, a digit or the line's end after `::`) but is an empty
      // placeholder to the editor, like `[key:: ]`
      const closedEmpty = !!opts.empty && (before === "[" ? after.startsWith("]") : before === "(" && after.startsWith(")"));
      const afterOk = after === "" || /^[\s\d]/.test(after) || closedEmpty;
      if (!afterOk || !isFieldKey(key)) {
        i += 2;
        continue;
      }
      let found: { from: number; to: number; valueFrom: number; valueTo: number; value: string; form: FieldForm } | null = null;
      if (before === "[" || before === "(") {
        const close = before === "[" ? "]" : ")";
        const restStart = i + 2 + leadingWsAll(after);
        const rest = line.slice(restStart);
        const c = rest.indexOf(close);
        if (c >= 0 && !rest.slice(0, c).includes(before)) {
          const raw = rest.slice(0, c);
          const valueFrom = restStart + leadingWsAll(raw);
          const value = raw.trim();
          found = {
            from: keyStart - 1,
            to: restStart + c + 1,
            valueFrom,
            valueTo: valueFrom + value.length,
            value,
            form: before === "[" ? "bracket" : "paren",
          };
        }
      } else if (keyStart === contentStart) {
        const lineEnd = line.replace(/\s+$/, "").length;
        let valueFrom = i + 2 + leadingWsAll(after);
        let valueTo = lineEnd;
        let value = line.slice(valueFrom, valueTo);
        const p = value.lastIndexOf(" ^");
        if (p >= 0 && BLOCK_ID_RE.test(value.slice(p + 2))) {
          value = value.slice(0, p).replace(/\s+$/, "");
          valueTo = valueFrom + value.length;
        }
        if (valueFrom > valueTo) valueFrom = valueTo;
        found = { from: keyStart, to: lineEnd, valueFrom, valueTo, value, form: "line" };
      } else if (before !== undefined && /\s/.test(before)) {
        const restStart = i + 2 + leadingWsAll(after);
        const rest = line.slice(restStart);
        const tokenEnd = rest.search(/\s/);
        const tokenLen = tokenEnd < 0 ? rest.length : tokenEnd;
        found = {
          from: keyStart,
          to: restStart + tokenLen,
          valueFrom: restStart,
          valueTo: restStart + tokenLen,
          value: rest.slice(0, tokenLen),
          form: "bare",
        };
      }
      if (found && (found.value !== "" || (opts.empty && found.form !== "bare"))) {
        out.push({
          key,
          value: found.value,
          from: base + lineStart + found.from,
          to: base + lineStart + found.to,
          valueFrom: base + lineStart + found.valueFrom,
          valueTo: base + lineStart + found.valueTo,
          form: found.form,
        });
        i = Math.max(found.to, i + 2);
      } else {
        i += 2;
      }
    }
    lineStart += line.length + 1;
  }
  return out;
}

function leadingWsAll(s: string): number {
  const m = /^\s*/.exec(s);
  return m ? m[0].length : 0;
}

export type FieldValueKind = "date" | "number" | "text";

export function fieldValueKind(value: string): FieldValueKind {
  if (/^\d{4}-\d{2}-\d{2}$/.test(value)) return "date";
  if (/^-?\d+(?:\.\d+)?$/.test(value)) return "number";
  return "text";
}

/** What a set of block types declares for one field key: every value in order, each
 *  tagged with the type that names it, and the value kind (`date` / `number`) the first
 *  spec naming one gives it. The chip's value menu and the `key:: ` completion read this
 *  one answer — two walks of the same types let them disagree about, say, whether a key
 *  is a date, and the chip would open the calendar where the completion offered plain
 *  values. */
export function declaredField(types: BlockType[], key: string): { values: { value: string; type: string }[]; kind?: string } {
  const lower = key.toLowerCase();
  const values: { value: string; type: string }[] = [];
  let kind: string | undefined;
  for (const ty of types)
    for (const f of ty.fields) {
      if (f.key.toLowerCase() !== lower) continue;
      for (const value of f.values ?? []) values.push({ value, type: ty.name });
      kind ??= f.type;
    }
  return { values, kind };
}

/** Set, replace or (with null) remove a field on one line. The field with that key is
 *  rewritten in place, keeping the form the user wrote — an empty placeholder `[key:: ]`
 *  is filled rather than doubled; with several of the key, `at` (an offset into the
 *  line, the clicked chip) picks one, else the first. A new field is appended as
 *  `[key:: value]` before any trailing ` ^id` (appending after it would kill the
 *  anchor). A task line's due/scheduled dates are not this function's business — the
 *  caller routes those to rescheduleLine, which knows the emoji spellings. */
export function setFieldValue(line: string, key: string, value: string | null, at?: number): string {
  const lower = key.toLowerCase();
  const marks = scanFields(line, 0, { empty: true }).filter((f) => f.key.toLowerCase() === lower);
  const mark = (at === undefined ? undefined : marks.find((f) => f.from <= at && at <= f.to)) ?? marks[0];
  if (mark) {
    if (value === null) {
      let before = line.slice(0, mark.from);
      const after = line.slice(mark.to);
      if (/\s$/.test(before) && (after === "" || /^\s/.test(after))) before = before.replace(/\s+$/, "");
      return before + after;
    }
    // an empty `key::` has no space after the colons yet, and `key::value` is not a field
    const sep = line.slice(mark.from, mark.valueFrom).endsWith("::") ? " " : "";
    return line.slice(0, mark.valueFrom) + sep + value + line.slice(mark.valueTo);
  }
  if (value === null) return line;
  const anchor = /[ \t]\^[A-Za-z0-9-]+[ \t]*$/.exec(line);
  const piece = ` [${key}:: ${value}]`;
  if (anchor) return line.slice(0, anchor.index).replace(/[ \t]+$/, "") + piece + line.slice(anchor.index);
  return line.replace(/[ \t]+$/, "") + piece;
}

/** Text with its inline fields removed (a view row shows the words, not the metadata). */
export function stripFields(text: string): string {
  const marks = scanFields(text);
  if (!marks.length) return text;
  let out = "";
  let last = 0;
  for (const m of marks) {
    out += text.slice(last, m.from);
    last = m.to;
  }
  out += text.slice(last);
  return out.replace(/[ \t]{2,}/g, " ").replace(/[ \t]+$/gm, "");
}
