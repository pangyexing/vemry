---
name: daily-start
description: The morning start — overdue, due today, in progress, three things
triggers: [start my day, today, this morning, what should I do, 今日启动, 今天做什么, 早上好]
version: 2
---

# Daily start

## When

"Start my day", "what should I do today", the morning routine.

## Plan

None — a two-step routine needs no plan card.

## Inputs

- The date from the turn context. Dates are read and written as YYYY-MM-DD.
- query_blocks with kind: task — task_state: todo and due_before: <today> for
  the overdue and due-today ones; task_state: doing for what is in progress. If
  the user attached a view (@ the Agenda), that is the table — do not query it
  again.
- Yesterday's journal note (journal/<yesterday>.md) when it exists: anything it
  said would be done today.

## Do

- Propose three things for today, each a task that already exists (cited as
  [[note#^id]]) or a new one the journal promised. Ask before adding more than
  three: a start is a focus, not a list of everything.
- When the user confirms, schedule the chosen tasks for today with update_task
  (scheduled: <today>); a new one goes into today's journal note with
  append_note as - [ ] text.

## Landing

journal/<today>.md (created if missing) for a new task; a scheduled task stays in
its own note. Nothing else is written.

## Shape

The reply: the three items, one line each, and nothing else.

## Never

- Marking a task done on the user's behalf.
- A date in any form but YYYY-MM-DD.
- Working from memory instead of the index.
