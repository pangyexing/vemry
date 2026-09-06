// What the UI says about a skill. A built-in skill's file is English (the model reads it),
// so its title and one-line description for the welcome, the / picker and Settings come
// from the catalog (`skill.<name>.title` / `.desc`); a vault's own skill shows the
// description its author wrote, and an override of a built-in shows its own description
// when it has one (it is the user's file) and the catalog's otherwise.
import { t, hasKey } from "$lib/i18n/i18n.svelte";
import type { SkillEntry } from "$lib/api";

/** The scenes the welcome groups the built-ins by, in the order they are shown; the names
 *  inside a scene are in the order of use, not alphabetical. A vault's own skills form the
 *  last group. */
export const SKILL_GROUPS: { id: "research" | "daily" | "product" | "tidy"; names: string[] }[] = [
  { id: "research", names: ["research", "tutor"] },
  { id: "daily", names: ["daily-start", "daily-close", "weekly-report", "meeting-actions"] },
  { id: "product", names: ["prd", "feedback-triage", "decision-log", "iteration-plan", "retro", "brief"] },
  { id: "tidy", names: ["tidy", "tidy-note"] },
];

function catalogued(s: SkillEntry, part: "title" | "desc"): string | null {
  const key = `skill.${s.name}.${part}`;
  return s.source !== "vault" && hasKey(key) ? t(key) : null;
}

/** The short title a chip shows: the catalog's for a built-in, the name for a vault skill. */
export function skillTitle(s: SkillEntry): string {
  return catalogued(s, "title") ?? s.name;
}

/** The one-line description: the author's own for a vault skill or a customised copy that has one, else the catalog's. */
export function skillDesc(s: SkillEntry): string {
  if (s.source !== "builtin" && s.description) return s.description;
  return catalogued(s, "desc") ?? s.description;
}

/** The enabled skills by scene: the built-in groups (skills still enabled, in the scene's
 *  order), then the vault's own under "this vault". A group with nothing in it is left out. */
export function groupSkills(skills: SkillEntry[]): { label: string; skills: SkillEntry[] }[] {
  const byName = new Map(skills.map((s) => [s.name, s]));
  const placed = new Set<string>();
  const out: { label: string; skills: SkillEntry[] }[] = [];
  for (const g of SKILL_GROUPS) {
    const members = g.names.flatMap((n) => {
      const s = byName.get(n);
      if (!s) return [];
      placed.add(n);
      return [s];
    });
    if (members.length) out.push({ label: t(`skillGroup.${g.id}`), skills: members });
  }
  const rest = skills.filter((s) => !placed.has(s.name));
  if (rest.length) out.push({ label: t("skillGroup.vault"), skills: rest });
  return out;
}
