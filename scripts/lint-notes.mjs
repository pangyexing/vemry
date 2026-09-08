// Format lint for a study's notes (or any folder of notes): what the app cannot render right.
// Usage: node scripts/lint-notes.mjs <folder> [--fix] [--all]
// Reports, per kind: bare-math (a formula outside $…$; --fix wraps the simple ones), ascii-punct
// (ASCII punctuation against CJK text), katex-error (a $…$ KaTeX rejects), display-tail, remote-image
// (a figure not yet localised), embed-unresolved (a source id no block carries), english-heading,
// missing-chapter-tag, empty-try-it, table-shape, loose-list. --all lists every finding.
import katex from "katex";
import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
const ROOT = process.argv[2];
const FIX = process.argv.includes("--fix");
const CJK = "[\\u3000-\\u303f\\u3400-\\u4dbf\\u4e00-\\u9fff\\uff00-\\uffef]";
const files = [];
(function walk(d) { for (const n of readdirSync(d)) { const p = join(d, n); if (statSync(p).isDirectory()) walk(p); else if (n.endsWith(".md")) files.push(p); } })(ROOT);
const source = files.find((f) => f.endsWith("/source.md"));
const sourceIds = new Set(source ? [...readFileSync(source, "utf8").matchAll(/\^([A-Za-z0-9-]+)\s*$/gm)].map((m) => m[1]) : []);
// spans the prose rules must not touch
const PROTECT = /(```[\s\S]*?```|`[^`\n]+`|\$\$[^$\n]+\$\$|\$(?![\s$])(?:\\.|[^$\n\\])+?\$(?!\d)|!?\[\[[^\]\n]+\]\]|!?\[[^\]\n]*\]\([^)\n]*\)|\[[^\]\n]*::[^\]\n]*\]|<[^>\n]+>|https?:\/\/\S+|^---\n[\s\S]*?\n---\n)/gm;
// a bare formula token: an identifier with a subscript or superscript, a root, a transpose, a squared, an O(…)
const BARE = /(?<![\\$\w/])(?:√[A-Za-z]\w*(?:[_^]\{?\w+\}?)*|[A-Za-z](?:[_^]\{?[A-Za-z0-9]+\}?)+(?:[=/·×+\-][A-Za-z0-9]+(?:[_^]\{?[A-Za-z0-9]+\}?)*)*|[A-Za-z]+ᵀ|\bO\((?:[^()\n]|\([^()\n]*\))+\))(?![\w$])/g;
const findings = [];
const report = (file, line, kind, text) => findings.push({ file: file.slice(ROOT.length + 1), line, kind, text });
const toTex = (tok) =>
  tok.replace(/ᵀ/g, "^T").replace(/√([A-Za-z]\w*(?:[_^]\w+)*)/g, "\\sqrt{$1}").replace(/·/g, "\\cdot ").replace(/×/g, "\\times ")
    .replace(/([_^])([A-Za-z0-9]{2,})/g, "$1{$2}").replace(/\b(head|softmax|Attention|Concat|FFN|max)\b/g, "\\mathrm{$1}");
for (const file of files) {
  let text = readFileSync(file, "utf8");
  const orig = text;
  const lines = text.split("\n");
  const rel = file.slice(ROOT.length + 1);
  // formulas typeset by KaTeX: any that error (a fenced block is code, not formulas)
  let fenced = false;
  for (const [i, line] of lines.entries()) {
    if (/^\s*```/.test(line)) { fenced = !fenced; continue; }
    if (fenced) continue;
    for (const m of line.matchAll(/\$\$([^$\n]+?)\$\$|\$(?![\s$])((?:\\.|[^$\n\\])+?)\$(?!\d)/g)) {
      const tex = m[1] ?? m[2];
      try { katex.renderToString(tex, { throwOnError: true, trust: false }); } catch (e) { report(file, i + 1, "katex-error", `${tex} — ${String(e.message).slice(0, 80)}`); }
    }
    if (/^\$\$[^$\n]+\$\$\s*\S/.test(line) && !/^\$\$[^$\n]+\$\$\s*(\(\d+\)\s*)?(\^[\w-]+)?\s*$/.test(line)) report(file, i + 1, "display-tail", line.slice(0, 80));
    if (/!\[[^\]]*\]\(https?:/.test(line)) report(file, i + 1, "remote-image", line.slice(0, 80));
    for (const m of line.matchAll(/!\[\[[^\]#]*source#\^([\w-]+)\]\]/g)) if (!sourceIds.has(m[1])) report(file, i + 1, "embed-unresolved", m[0]);
    if (/^#{1,6} [A-Za-z][A-Za-z ,'-]*$/.test(line) && rel.includes("chapters/")) report(file, i + 1, "english-heading", line);
  }
  if (rel.includes("chapters/") && !/^#chapter \[status:: \w+\]$/m.test(text)) report(file, 0, "missing-chapter-tag", "");
  if (/^## 试一试\s*$/m.test(text) && !/```js/.test(text)) report(file, 0, "empty-try-it", "");
  // tables: every row the header's width
  let width = 0;
  for (const [i, line] of lines.entries()) {
    if (/^\s*\|.*\|\s*$/.test(line)) { const n = line.trim().slice(1, -1).split(/(?<!\\)\|/).length; if (/^\s*\|[\s:-]+\|\s*$/.test(line)) { width = n; continue; } if (width && n !== width) report(file, i + 1, "table-shape", `${n} cells vs ${width}`); } else width = 0;
  }
  // loose list: a blank line between two list items
  for (let i = 1; i + 1 < lines.length; i++) if (!lines[i].trim() && /^\s*[-*] /.test(lines[i - 1]) && /^\s*[-*] /.test(lines[i + 1])) report(file, i + 1, "loose-list", lines[i + 1].slice(0, 60));
  // prose rules, outside protected spans
  let out = ""; let at = 0;
  const prose = (chunk, offset) => {
    const lineOf = (idx) => text.slice(0, offset + idx).split("\n").length;
    for (const m of chunk.matchAll(new RegExp(`(?<=${CJK})[,;:?!()"]|[,;:?!()"](?=${CJK})`, "g"))) report(file, lineOf(m.index), "ascii-punct", chunk.slice(Math.max(0, m.index - 12), m.index + 12).replace(/\n/g, " "));
    // replacements by position, last first, so one never lands inside another
    const edits = [];
    for (const m of chunk.matchAll(BARE)) {
      const tok = m[0];
      if (/^[A-Za-z]_?\d?$/.test(tok) || /^O\(\d+\)$/.test(tok)) continue;
      const tex = toTex(tok);
      let ok = true; try { katex.renderToString(tex, { throwOnError: true, trust: false }); } catch { ok = false; }
      report(file, lineOf(m.index), ok ? "bare-math" : "bare-math-unfixable", `${tok} → $${tex}$`);
      if (ok) edits.push([m.index, tok.length, `$${tex}$`]);
    }
    if (!FIX) return chunk;
    let fixed = chunk;
    for (const [at, len, rep] of edits.reverse()) fixed = fixed.slice(0, at) + rep + fixed.slice(at + len);
    return fixed;
  };
  for (const m of text.matchAll(PROTECT)) { out += prose(text.slice(at, m.index), at); out += m[0]; at = m.index + m[0].length; }
  out += prose(text.slice(at), at);
  if (FIX && out !== orig) writeFileSync(file, out);
}
const byKind = {};
for (const f of findings) (byKind[f.kind] ??= []).push(f);
for (const [k, v] of Object.entries(byKind).sort((a, b) => b[1].length - a[1].length)) {
  console.log(`\n== ${k}: ${v.length}`);
  for (const f of v.slice(0, process.argv.includes("--all") ? 1e9 : 6)) console.log(`  ${f.file}:${f.line}  ${f.text}`);
}
if (!findings.length) console.log("clean");
