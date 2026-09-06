// Flashcards are `> [!card]` callouts tagged `#card`: the first line is the question, the
// body is the answer, and the schedule lives in inline fields on the callout's last line
// (`due` / `interval` / `ease` / `reviews`). The parser knows nothing about cards — a card is
// a paragraph block with a tag and fields, so the Review view is one more projection of the
// query API, and grading is a user field write on the block's span (the same span-rewrite
// contract as tasks.ts: read, validate against the indexed content, rewrite exactly the
// span, false when stale).
import type { BlockHit } from "$lib/api";
import { setFieldValue, stripAnchor, stripFields } from "$lib/fields";
import { fieldOfLower } from "./queryrows";
import { editBlockSpan } from "./blockspan";
import { ymd } from "./tasks";

export type Grade = "again" | "hard" | "good" | "easy";
export const GRADES: readonly Grade[] = ["again", "hard", "good", "easy"];

/** The four schedule fields, in the order they are written onto the line. */
const SCHEDULE_KEYS = ["due", "interval", "ease", "reviews"] as const;

export interface Schedule {
  /** `YYYY-MM-DD`; null on a card nobody has graded yet (due now). */
  due: string | null;
  /** Days between the last review and `due`; 0 = new or lapsed. */
  interval: number;
  /** The ease factor (2.5 at the start; never below 1.3). */
  ease: number;
  reviews: number;
}

export interface Card extends Schedule {
  hit: BlockHit;
  /** The question: the callout's first line with fields, the tag and the anchor stripped. */
  front: string;
  /** The answer: the remaining lines, cleaned the same way; "" when nothing is written yet. */
  back: string;
}

const DEFAULT_EASE = 2.5;
const MIN_EASE = 1.3;
const MAX_EASE = 3.5;
/** Ten years: a card graded Easy for long enough stops growing rather than scheduling into a date that is a typo. */
const MAX_INTERVAL = 3650;

/** A line of a card without its metadata: inline fields, the `#card` tag and a trailing
 *  ` ^id` (through `stripAnchor`, so the question reads exactly as the parser reads the
 *  line). */
function clean(line: string): string {
  return stripAnchor(
    stripFields(line)
      .replace(/(^|\s)#card(?=\s|$)/gi, "$1")
      .trimEnd(),
  ).trim();
}

/** The stable identity of a card across a grade: the note plus the question. A span is not
 *  one (writing the schedule lengthens the line and shifts every block below it). */
export const cardKey = (c: Card) => `${c.hit.note}\n${c.front}`;

export function parseCard(hit: BlockHit): Card {
  const lines = hit.block.content.split("\n");
  const front = clean(lines[0] ?? "");
  const back = lines.slice(1).map(clean).filter(Boolean).join("\n");
  const field = (key: string) => fieldOfLower(hit, key)?.trim();
  const due = field("due");
  const interval = Number.parseInt(field("interval") ?? "", 10);
  const ease = Number.parseFloat(field("ease") ?? "");
  const reviews = Number.parseInt(field("reviews") ?? "", 10);
  return {
    hit,
    front,
    back,
    due: due && /^\d{4}-\d{2}-\d{2}$/.test(due) ? due : null,
    interval: Number.isFinite(interval) && interval >= 0 ? Math.min(interval, MAX_INTERVAL) : 0,
    ease: Number.isFinite(ease) ? Math.min(MAX_EASE, Math.max(MIN_EASE, ease)) : DEFAULT_EASE,
    reviews: Number.isFinite(reviews) && reviews >= 0 ? reviews : 0,
  };
}

/** Due on or before `today` (`YYYY-MM-DD` strings compare as dates), or never graded. */
export const isDue = (c: Schedule, today: string) => c.due === null || c.due <= today;

function addDays(today: string, days: number): string {
  const d = new Date(`${today}T00:00:00`);
  d.setDate(d.getDate() + days);
  return ymd(d);
}

/** The next schedule after a grade (a two-factor spaced-repetition rule: an interval that
 *  grows by the ease, an ease that drifts with the grades):
 *   - Again: back to today (the card returns at the end of this session), ease −0.2
 *   - Hard: ×1.2 (at least a day), ease −0.15
 *   - Good: 1 → 3 → ×ease
 *   - Easy: 3 on a new card, else ×ease ×1.3, ease +0.15
 *  `interval` is measured from `today`, so a card graded late is not penalised for the
 *  wait, and a card studied ahead counts as reviewed. */
export function schedule(c: Schedule, grade: Grade, today: string): Schedule {
  let interval = c.interval;
  let ease = c.ease;
  switch (grade) {
    case "again":
      interval = 0;
      ease = Math.max(MIN_EASE, ease - 0.2);
      break;
    case "hard":
      interval = Math.max(1, Math.round(interval * 1.2));
      ease = Math.max(MIN_EASE, ease - 0.15);
      break;
    case "good":
      interval = interval === 0 ? 1 : interval === 1 ? 3 : Math.round(interval * ease);
      break;
    case "easy":
      interval = interval === 0 ? 3 : Math.max(4, Math.round(interval * ease * 1.3));
      ease = Math.min(MAX_EASE, ease + 0.15);
      break;
  }
  interval = Math.min(interval, MAX_INTERVAL);
  return { due: addDays(today, interval), interval, ease: Math.round(ease * 100) / 100, reviews: c.reviews + 1 };
}

/** The callout's content as the parser indexes it (the title after `[!card]`, then each
 *  quoted line with its `>` and one space removed) — the stale-span guard compares this with
 *  the block's indexed content. null when the slice is not a card callout at all. */
function calloutContent(slice: string): string | null {
  const lines = slice.split("\n").map((l) => l.replace(/\r$/, ""));
  const m = /^>\s*\[!([^\]\s]+)\][+-]?(.*)$/.exec(lines[0].trim());
  if (!m || m[1].toLowerCase() !== "card") return null;
  let content = m[2].trim();
  for (const l of lines.slice(1)) {
    let s = l.trimStart().replace(/^>+/, "");
    if (s.startsWith(" ")) s = s.slice(1);
    if (content !== "") content += "\n";
    content += s;
  }
  return content;
}

/** The schedule written onto one line: existing fields are replaced in place, missing ones
 *  are appended before the trailing `#card` tag and `^id` (so the line stays
 *  `[fields] #card ^id`, the shape the type template and the tutor skill write). */
function writeSchedule(line: string, s: Schedule): string {
  const tail = /(?:[ \t]#card)?(?:[ \t]\^[A-Za-z0-9-]+)?[ \t]*$/i.exec(line);
  const cut = tail ? tail.index : line.length;
  let head = line.slice(0, cut);
  const rest = line.slice(cut).replace(/[ \t]+$/, "");
  const values: Record<(typeof SCHEDULE_KEYS)[number], string> = {
    due: s.due ?? "",
    interval: String(s.interval),
    ease: String(s.ease),
    reviews: String(s.reviews),
  };
  for (const key of SCHEDULE_KEYS) head = setFieldValue(head, key, values[key]);
  return head + rest;
}

/**
 * Grade a card: compute its next schedule and write the four fields onto the callout's
 * schedule line (the last line carrying any of them, else the callout's last line).
 * Returns the new schedule, or null when the span is stale (the note changed since the
 * query) — the caller requeries and lets the user grade again on fresh data.
 */
export async function gradeCard(card: Card, grade: Grade, today: string): Promise<Schedule | null> {
  const next = schedule(card, grade, today);
  const landed = await editBlockSpan(
    card.hit,
    (slice) => calloutContent(slice) === card.hit.block.content,
    (slice) => {
      const lines = slice.split("\n");
      let at = lines.length - 1;
      for (let i = lines.length - 1; i >= 0; i--) {
        if (/\[(?:due|interval|ease|reviews)::/i.test(lines[i])) {
          at = i;
          break;
        }
      }
      // A card whose last line is the question itself (no body yet) gets its schedule on a new quoted line
      if (at === 0 && lines.length === 1) lines.push(writeSchedule(">", next));
      else lines[at] = writeSchedule(lines[at], next);
      return lines.join("\n");
    },
  );
  return landed ? next : null;
}
