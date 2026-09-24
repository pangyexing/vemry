// The ink check: every text ink on every ground it is drawn on, in both themes, at the
// contrast the reading-first rules promise (WCAG 2 ratios). Reads the tokens straight out
// of src/app.css — the light values on :root, the dark ones on :root under
// `prefers-color-scheme: dark` — resolves var() references between them, and lays a
// translucent ground (the selection) over the page before measuring. Fails with the list
// of pairs that fall short; a token renamed or removed fails too, so the check cannot
// silently stop checking.
// Usage: node scripts/check-contrast.mjs  (run by `pnpm check` and CI)
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const css = readFileSync(fileURLToPath(new URL("../src/app.css", import.meta.url)), "utf8");

/** The body of the block opening at `open` (the index of its `{`), braces balanced. */
function blockAt(text, open) {
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    if (text[i] === "{") depth++;
    else if (text[i] === "}" && --depth === 0) return text.slice(open + 1, i);
  }
  throw new Error("unbalanced braces in app.css");
}

function tokensOf(body) {
  const out = {};
  const noComments = body.replace(/\/\*[\s\S]*?\*\//g, "");
  for (const m of noComments.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) out[m[1]] = m[2].trim();
  return out;
}

// Light: every top-level `:root {` block, in order (the provenance tokens have one of their
// own further down). Dark: every `:root {` inside a `prefers-color-scheme: dark` block.
const light = {};
const dark = {};
for (const m of css.matchAll(/^:root\s*\{/gm)) Object.assign(light, tokensOf(blockAt(css, m.index + m[0].length - 1)));
for (const m of css.matchAll(/@media\s*\(prefers-color-scheme:\s*dark\)\s*\{/g)) {
  const media = blockAt(css, m.index + m[0].length - 1);
  for (const r of media.matchAll(/:root\s*\{/g)) Object.assign(dark, tokensOf(blockAt(media, r.index + r[0].length - 1)));
}

/** A colour value → [r, g, b, a] (0–1), following var() through the theme's tokens. */
function resolve(theme, value, seen = new Set()) {
  const v = value.trim();
  const ref = /^var\((--[\w-]+)\)$/.exec(v);
  if (ref) {
    if (seen.has(ref[1])) throw new Error(`var() cycle at ${ref[1]}`);
    seen.add(ref[1]);
    const next = theme === "dark" ? (dark[ref[1]] ?? light[ref[1]]) : light[ref[1]];
    if (next == null) throw new Error(`${theme}: ${ref[1]} is not defined`);
    return resolve(theme, next, seen);
  }
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(v);
  if (hex) {
    const h = hex[1].length === 3 ? [...hex[1]].map((c) => c + c).join("") : hex[1];
    return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255).concat(1);
  }
  const rgba = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(v);
  if (rgba) return [rgba[1] / 255, rgba[2] / 255, rgba[3] / 255, rgba[4] == null ? 1 : +rgba[4]];
  throw new Error(`${theme}: cannot read the colour "${v}"`);
}

function token(theme, name) {
  const v = theme === "dark" ? (dark[name] ?? light[name]) : light[name];
  if (v == null) throw new Error(`${theme}: ${name} is not defined in app.css`);
  return resolve(theme, v);
}

/** `top` laid over the opaque `under`. */
const over = (top, under) => [0, 1, 2].map((i) => top[i] * top[3] + under[i] * (1 - top[3])).concat(1);
const channel = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const luminance = (c) => 0.2126 * channel(c[0]) + 0.7152 * channel(c[1]) + 0.0722 * channel(c[2]);
function contrast(a, b) {
  const x = luminance(a);
  const y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/** A ground by name: a token, or "--sel-bg" (a tint) laid over the page. */
function ground(theme, name) {
  const g = token(theme, name);
  return g[3] < 1 ? over(g, token(theme, "--bg")) : g;
}

// What each ink promises, and on which grounds. The grounds are the ones the ink is drawn
// on: the page, a side panel, a hovered row, an active row, a popover, a code block and the
// selection. Text: body ink ≥ 7, secondary ink ≥ 4.5; marks (faint ink, the review amber)
// ≥ 3; the accent and the danger ink are text in links, buttons and warnings (≥ 4.5).
const RULES = [
  { ink: "--fg", min: 7, on: ["--bg", "--bg-panel", "--bg-hover", "--bg-active", "--bg-popover", "--bg-code", "--sel-bg"] },
  { ink: "--fg-muted", min: 4.5, on: ["--bg", "--bg-panel", "--bg-hover", "--bg-popover", "--bg-code"] },
  { ink: "--fg-faint", min: 3, on: ["--bg", "--bg-panel"] },
  { ink: "--accent", min: 4.5, on: ["--bg", "--bg-panel"] },
  { ink: "--danger", min: 4.5, on: ["--bg", "--bg-panel", "--bg-hover"] },
  { ink: "--warning", min: 4.5, on: ["--bg"] },
  { ink: "--ok", min: 4.5, on: ["--bg"] },
  // the ground's colour is the ink of a glyph on an accent fill (a button's label, the done tick)
  { ink: "--bg", min: 4.5, on: ["--accent"] },
  // the callout heads, at body size and 600 weight
  ...["--co-info", "--co-tip", "--co-success", "--co-question", "--co-warn", "--co-danger", "--co-example", "--co-quote", "--co-claim", "--co-evidence"].map(
    (ink) => ({ ink, min: 4.5, on: ["--bg"] }),
  ),
  { ink: "--prov-agent", min: 3, on: ["--bg"] },
];

const failures = [];
let checked = 0;
for (const theme of ["light", "dark"]) {
  for (const rule of RULES) {
    for (const g of rule.on) {
      const ratio = contrast(token(theme, rule.ink), ground(theme, g));
      checked++;
      if (ratio + 1e-9 < rule.min) failures.push(`${theme}: ${rule.ink} on ${g} is ${ratio.toFixed(2)}:1, below ${rule.min}:1`);
    }
  }
}

if (failures.length) {
  console.error(`check-contrast: ${failures.length} of ${checked} pairs fall short\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log(`check-contrast: ${checked} ink/ground pairs pass in both themes`);
