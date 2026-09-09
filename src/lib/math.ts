// The one grammar of a formula, shared by the editor's live preview (editor/math.ts)
// and the hand-written renderer (markdown.ts, which the embeds, the reading
// projection and the chat panel all draw with): `$…$` inline, `$$…$$` display on
// one line, and a display block whose `$$` fences sit on their own lines. The two
// consumers must agree on what is a formula, or a `$` folds in the editor and shows
// as text in an embed of the same note (docs/SYNTAX.md, "Math").
import katex from "katex";
import { stripAnchor } from "$lib/fields";

/** LaTeX → KaTeX HTML. The renderer's only third-party HTML injection point: called
 *  on recognised formula fragments alone, with trust:false forbidding dangerous
 *  commands such as \href; a syntax error renders as the source in red
 *  (throwOnError:false) rather than throwing and taking the whole document down. */
export function renderMathHtml(tex: string, display: boolean): string {
  const src = tex.trim();
  const key = (display ? "d" : "i") + src;
  const seen = MATH_HTML.get(key);
  if (seen !== undefined) return seen;
  let html: string;
  try {
    html = katex.renderToString(src, {
      displayMode: display,
      throwOnError: false,
      trust: false,
      output: "html",
    });
  } catch {
    html = `<code>${src.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")}</code>`;
  }
  if (MATH_HTML.size >= MATH_HTML_MAX) MATH_HTML.delete(MATH_HTML.keys().next().value!);
  MATH_HTML.set(key, html);
  return html;
}

/** The same formula is typeset over and over — a note scrolled back through the
 *  viewport rebuilds its widgets, a streaming answer re-renders its tail on every
 *  token, a translation its segment on every update — and each call is KaTeX parsing
 *  the source again. Keyed on the mode and the trimmed source; bounded, oldest out
 *  first, so a long session does not hold every formula it has ever drawn. */
const MATH_HTML = new Map<string, string>();
const MATH_HTML_MAX = 400;

/** A single-line display formula, `$$…$$` with the content on the same line. */
const DISPLAY_MATH_RE = /\$\$([^$\n]+?)\$\$/g;
/** An inline formula: non-whitespace at both ends and no digit after the closing `$`
 *  — a dollar amount in the body is not a formula. A backslash escapes the next
 *  character inside, so `\$` never closes. */
const INLINE_MATH_RE = /\$(?![\s$])((?:\\.|[^$\n\\])+?)\$(?!\d)/g;

export interface MathSpan {
  /** Offsets into the scanned text, the `$` delimiters included. */
  start: number;
  end: number;
  tex: string;
  display: boolean;
}

/** The formulas of one line of text, display ones first (their `$$` must not be read
 *  as two inline delimiters), in document order. `$…$` whose content ends in
 *  whitespace is left as text, the same rule the renderer applies. */
export function scanInlineMath(text: string): MathSpan[] {
  if (!text.includes("$")) return [];
  const out: MathSpan[] = [];
  const taken: [number, number][] = [];
  const free = (s: number, e: number) => taken.every(([a, b]) => e <= a || s >= b);
  DISPLAY_MATH_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = DISPLAY_MATH_RE.exec(text))) {
    out.push({ start: m.index, end: m.index + m[0].length, tex: m[1], display: true });
    taken.push([m.index, m.index + m[0].length]);
  }
  INLINE_MATH_RE.lastIndex = 0;
  while ((m = INLINE_MATH_RE.exec(text))) {
    if (/\s$/.test(m[1]) || !free(m.index, m.index + m[0].length)) continue;
    out.push({ start: m.index, end: m.index + m[0].length, tex: m[1], display: false });
  }
  return out.sort((a, b) => a.start - b.start);
}

/** Whether a line opens a multi-line display block: it starts with `$$` (after
 *  indentation) and does not close on the same line. `$$` alone and `$$ a + b` open;
 *  `$$x$$` is a one-line formula, and so are `$$x$$ ^id` and `$$x$$ (3)` — whatever
 *  follows the closer (an equation number, the block id) is text, inline's business.
 *  Mirrors the formatter's rule (format.rs), which leaves a true block verbatim. */
export function opensMathBlock(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed.startsWith("$$")) return false;
  return !trimmed.slice(2).includes("$$");
}

/** Whether a line closes a multi-line display block: it ends with `$$`, with at most
 *  a block id after it. A blank line never closes one — the callers stop looking at a
 *  blank line, so an opener with no closer in its paragraph stays text rather than
 *  swallowing the paragraphs below up to some later formula's closer. */
export function closesMathBlock(line: string): boolean {
  return /\$\$$/.test(stripAnchor(line));
}

/** The formula inside a multi-line block: the `$$` fences stripped from the first and
 *  last lines (a block id after the closer goes with it), the lines between kept as
 *  written. */
export function mathBlockTex(lines: string[]): string {
  if (!lines.length) return "";
  const parts = lines.slice();
  parts[0] = parts[0].trim().slice(2);
  const last = parts.length - 1;
  parts[last] = stripAnchor(parts[last]).replace(/\$\$$/, "");
  return parts.join("\n").trim();
}

/** The commands a plain-text row can stand in for; anything else keeps its name. */
const PLAIN_SYMBOLS: Record<string, string> = {
  cdot: "·", times: "×", sum: "∑", prod: "∏", int: "∫", infty: "∞", le: "≤", ge: "≥", ne: "≠", approx: "≈", pm: "±",
  to: "→", rightarrow: "→", leftarrow: "←", ldots: "…", cdots: "⋯",
  alpha: "α", beta: "β", gamma: "γ", delta: "δ", epsilon: "ε", varepsilon: "ε", zeta: "ζ", eta: "η", theta: "θ", lambda: "λ",
  mu: "μ", nu: "ν", xi: "ξ", pi: "π", rho: "ρ", sigma: "σ", tau: "τ", phi: "φ", varphi: "φ", chi: "χ", psi: "ψ", omega: "ω",
  Gamma: "Γ", Delta: "Δ", Theta: "Θ", Lambda: "Λ", Pi: "Π", Sigma: "Σ", Phi: "Φ", Psi: "Ψ", Omega: "Ω",
};

/** A formula as plain text — for a row that cannot typeset (a query result, a card, an
 *  Agenda line): √ for a root, a/b for a fraction, the symbols above for their commands,
 *  braces dropped, an unknown command kept by name. `$\sqrt{d_k}$` reads `√d_k`. */
export function mathPlain(tex: string): string {
  let s = tex.trim();
  for (let i = 0; i < 4; i++) {
    // innermost first, a few passes for nesting
    s = s
      .replace(/\\sqrt\s*\{([^{}]*)\}/g, "√$1")
      .replace(/\\frac\s*\{([^{}]*)\}\s*\{([^{}]*)\}/g, "$1/$2")
      .replace(/\\(?:mathrm|mathbf|mathit|mathcal|text|operatorname|boldsymbol)\s*\{([^{}]*)\}/g, "$1");
  }
  s = s.replace(/\\(?:left|right|quad|qquad|displaystyle)\b|\\[,;!]/g, " ");
  s = s.replace(/\\([A-Za-z]+)/g, (_, name: string) => PLAIN_SYMBOLS[name] ?? name);
  s = s.replace(/[{}]/g, "");
  return s.replace(/\s+/g, " ").trim();
}

/** Every formula in `text` replaced by what `render` makes of it (the `$` delimiters go
 *  with it). The one walk behind both the plain-text fold and the renderers' stashing,
 *  so "what is a formula" stays scanInlineMath's single answer. */
export function replaceMath(text: string, render: (span: MathSpan) => string): string {
  if (!text.includes("$")) return text;
  let out = "";
  let at = 0;
  for (const m of scanInlineMath(text)) {
    out += text.slice(at, m.start) + render(m);
    at = m.end;
  }
  return out + text.slice(at);
}

/** Every formula in `text` replaced by its plain form. */
export function foldMath(text: string): string {
  return replaceMath(text, (m) => mathPlain(m.tex));
}
