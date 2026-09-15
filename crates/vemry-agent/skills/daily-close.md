---
name: daily-close
description: The evening close — done today, notes touched, carry-over, into the journal
triggers: [close my day, end of day, what did I do today, wrap up, 今日收尾, 今天做了什么, 收工, 今日のまとめ, 一日を締める, 하루 마무리, 오늘 한 일, clôturer ma journée, bilan du jour, Tagesabschluss, was habe ich heute gemacht, cerrar el día, qué hice hoy]
version: 2
---

# Daily close

## When

"Close the day", "what did I do today", the evening routine.

## Plan

None — no plan card.

## Inputs

- The date from the turn context.
- recent_changes with since: <today> and kind: done — the tasks finished today;
  recent_changes with since: <today> alone — the notes touched (a few blocks
  per note is enough: group by note, do not list every block).
- query_blocks with task_state: doing — tomorrow's carry-over.

## Do

One section, appended to today's journal note: done (each task cited as
[[note#^id]]), the notes touched, the carry-over. Propose the write: the journal
is the user's, so it asks unless the journal is the working note.

## Landing

journal/<today>.md, created if missing, via append_note.

## Shape

## Close → done → touched notes → carry-over. In the reply: one paragraph, then
stop — a close nobody reads is worse than none.

## Never

- Marking a task done on the user's behalf.
- Listing every block that changed.
