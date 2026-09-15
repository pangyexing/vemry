---
name: weekly-report
description: The week's report — shipped, slipped, next week, appended to the week's note
triggers: [weekly report, weekly review, this week, week in review, 周报, 周复盘, 本周总结, 週報, 今週のまとめ, 주간 보고, 이번 주 정리, rapport hebdomadaire, bilan de la semaine, Wochenbericht, Wochenrückblick, informe semanal, balance de la semana]
version: 2
---

# Weekly report

## When

"The week's report / review", "what happened this week", the Friday pass over
tasks, claims and loose ends.

## Plan

1. Collect done, slipped and open
2. Read this week's journal
3. Write the report
4. Offer the plain version

## Inputs

- The week is Monday to the date in the turn context.
- recent_changes with since: <monday> and kind: done — what was finished;
  query_blocks with task_state: todo and due_before: <today> — what slipped;
  task_state: doing — what is open.
- query_blocks with kind: claim, and their supports:: links — the claims still
  without evidence (a research week has those too).
- This week's journal notes (journal/<date>.md) — promises made and blockers
  named in prose.

## Do

Write the report as one section and append it to the week's note; propose the
write. Then offer, in the reply itself, a second version for other people: no
wikilinks, no handles, plain sentences — the user copies it.

## Landing

journal/<year>-W<week>.md, created if missing, via append_note.

## Shape

Done (each task cited as [[note#^id]]) → Blocked / slipped → Next week. Both
versions under a page.

## Never

- Marking anything done on the user's behalf.
- Writing the plain-text version into the vault.
