// Inline `key:: value` fields as the editor sees them — a mirror of vemry-core's
// `scan_fields` (parser.rs): the same three forms, the same boundary rule (`::`
// followed by whitespace, a digit or the end of the line), the same exclusions
// (backtick spans, `[[wikilinks]]`; the caller keeps code blocks and frontmatter
// out). Offsets are UTF-16 character positions. The Rust table of accepted and
// rejected examples is replayed against scanFields by the batch's harness, so the
// two cannot drift silently. Pure: no CodeMirror, no stores.

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

/** Every field of a text (one or more lines), offsets relative to `base`. */
export function scanFields(text: string, base = 0): FieldMark[] {
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
      const afterOk = after === "" || /^[\s\d]/.test(after);
      let keyStart = i;
      while (keyStart > 0 && KEY_CHAR.test(line[keyStart - 1])) keyStart--;
      const key = line.slice(keyStart, i);
      if (!afterOk || !isFieldKey(key)) {
        i += 2;
        continue;
      }
      const before = keyStart > 0 ? line[keyStart - 1] : undefined;
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
      if (found && found.value !== "") {
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

/** Set, replace or (with null) remove a field on one line. The first field with that
 *  key is rewritten in place, keeping the form the user wrote; a new field is appended
 *  as `[key:: value]` before any trailing ` ^id` (appending after it would kill the
 *  anchor). A task line's due/scheduled dates are not this function's business — the
 *  caller routes those to rescheduleLine, which knows the emoji spellings. */
export function setFieldValue(line: string, key: string, value: string | null): string {
  const lower = key.toLowerCase();
  const mark = scanFields(line).find((f) => f.key.toLowerCase() === lower);
  if (mark) {
    if (value === null) {
      let before = line.slice(0, mark.from);
      const after = line.slice(mark.to);
      if (/\s$/.test(before) && (after === "" || /^\s/.test(after))) before = before.replace(/\s+$/, "");
      return before + after;
    }
    return line.slice(0, mark.valueFrom) + value + line.slice(mark.valueTo);
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
