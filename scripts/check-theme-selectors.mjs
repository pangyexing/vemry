// The app has no theme attribute: dark values follow the system's colour scheme, written
// under `@media (prefers-color-scheme: dark)`. A rule keyed on a `data-theme` attribute
// never matches — the provenance colours sat under one for months and the dark theme drew
// the light ones — so any such selector in src/ fails the build.
// Usage: node scripts/check-theme-selectors.mjs  (run by `pnpm check` and CI)
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const SELECTOR = /\[\s*data-theme\b/;
const hits = [];
(function walk(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p);
    else if (/\.(css|svelte|ts|js|html)$/.test(name)) {
      readFileSync(p, "utf8")
        .split("\n")
        .forEach((line, i) => {
          if (SELECTOR.test(line)) hits.push(`${relative(root, p)}:${i + 1}: ${line.trim()}`);
        });
    }
  }
})(join(root, "src"));

if (hits.length) {
  console.error(`check-theme-selectors: a data-theme selector never matches (dark values go under prefers-color-scheme)\n  ${hits.join("\n  ")}`);
  process.exit(1);
}
console.log("check-theme-selectors: no data-theme selectors");
