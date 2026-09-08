// "Pin": write the lab controls' current values back into the block's code, so the
// experiment reopens where the reader left it. The one explicit write of the whole
// lab (moving a control never touches the note): it rewrites the literal a control
// was declared with and nothing else — `value: 64` in a `controls({…})` spec, the
// `toggle: true|false` of a toggle, the array literal of a `matrix("Q", [[…]])`. A
// control whose declaration is not a plain literal (a variable, a computed array) is
// left alone, and the caller reports what could not be pinned.
import type { LabControl } from "$lib/api";

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** A JS literal for a control value, as the author would have typed it. */
function literal(v: number | boolean | string): string {
  return typeof v === "string" ? JSON.stringify(v) : String(v);
}

/** The spec object of `name` inside a controls({…}) call: `name: { … }` with the key
 *  bare or quoted, and a body free of nested braces (a spec holds arrays, not objects). */
function specOf(code: string, name: string): { start: number; end: number; body: string } | null {
  const re = new RegExp(`(["']?)${escapeRe(name)}\\1\\s*:\\s*\\{([^{}]*)\\}`);
  const m = re.exec(code);
  if (!m) return null;
  const bodyStart = m.index + m[0].length - 1 - m[2].length;
  return { start: bodyStart, end: bodyStart + m[2].length, body: m[2] };
}

const VALUE_RE = /(\bvalue\s*:\s*)(-?\d+(?:\.\d+)?(?:e[+-]?\d+)?|true|false|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')/;
const TOGGLE_RE = /(\btoggle\s*:\s*)(true|false)/;

/** The closing bracket matching the `[` at `open`, or -1. */
function matchBracket(code: string, open: number): number {
  let depth = 0;
  for (let i = open; i < code.length; i++) {
    const ch = code[i];
    if (ch === "[") depth++;
    else if (ch === "]") {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** The rows of a matrix as an array literal: one line when short, else one row per line indented past the call. */
function matrixLiteral(rows: number[][], indent: string): string {
  const one = `[${rows.map((r) => `[${r.join(", ")}]`).join(", ")}]`;
  if (one.length <= 72) return one;
  return `[\n${rows.map((r) => `${indent}  [${r.join(", ")}]`).join(",\n")}\n${indent}]`;
}

export interface PinResult {
  code: string;
  /** The controls whose literal was rewritten. */
  pinned: string[];
  /** The controls whose declaration could not be found as a literal. */
  skipped: string[];
}

/** Rewrite `code` so each control's declared default is its current value. */
export function pinCode(code: string, controls: LabControl[]): PinResult {
  const pinned: string[] = [];
  const skipped: string[] = [];
  for (const c of controls) {
    if (c.kind === "matrix") {
      const re = new RegExp(`matrix\\(\\s*(["'])${escapeRe(c.name)}\\1\\s*,\\s*`);
      const m = re.exec(code);
      const open = m ? m.index + m[0].length : -1;
      if (open < 0 || code[open] !== "[") {
        skipped.push(c.name);
        continue;
      }
      const close = matchBracket(code, open);
      if (close < 0) {
        skipped.push(c.name);
        continue;
      }
      // unchanged numbers are left as the author formatted them
      let same = false;
      try {
        same = JSON.stringify(JSON.parse(code.slice(open, close + 1))) === JSON.stringify(c.rows);
      } catch {
        /* not JSON (a computed literal): rewritten below */
      }
      if (same) continue;
      const lineStart = code.lastIndexOf("\n", open) + 1;
      const indent = /^[ \t]*/.exec(code.slice(lineStart, open))![0];
      code = code.slice(0, open) + matrixLiteral(c.rows, indent) + code.slice(close + 1);
      pinned.push(c.name);
      continue;
    }
    const spec = specOf(code, c.name);
    if (!spec) {
      skipped.push(c.name);
      continue;
    }
    let body = spec.body;
    if (c.kind === "toggle") {
      if (!TOGGLE_RE.test(body)) {
        skipped.push(c.name);
        continue;
      }
      body = body.replace(TOGGLE_RE, `$1${literal(c.value)}`);
    } else if (VALUE_RE.test(body)) {
      body = body.replace(VALUE_RE, `$1${literal(c.value)}`);
    } else {
      // no value: written in, after the last field
      const trimmed = body.replace(/[\s,]*$/, "");
      body = `${trimmed}, value: ${literal(c.value)} `;
    }
    if (body !== spec.body) {
      code = code.slice(0, spec.start) + body + code.slice(spec.end);
      pinned.push(c.name);
    }
  }
  return { code, pinned, skipped };
}
