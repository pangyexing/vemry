---
name: daily-start
description: The morning routine: what is overdue, due today or in progress, what yesterday's journal promised — into three things for today
---

# Daily start

When the user asks for today's start (or "what should I do today"), work from the
index, not from memory, and keep it to one screen.

1. Take the date from the turn context. Query the tasks that matter now:
   `query_blocks` with `kind: task`, `task_state: todo`, `due_before: <today>` for
   the overdue and due-today ones, and `task_state: doing` for what is in progress.
   If the user attached a view (`@` the Agenda), that is the table — do not query
   it again.
2. Read yesterday's journal note (`journal/<yesterday>.md`) if it exists, and name
   anything it said would be done today.
3. Propose **three things for today**, each a task that already exists (cite it as
   `[[note#^id]]`) or a new one the journal promised. Ask before adding more than
   three: a start is a focus, not a list of everything.
4. When the user confirms, schedule the chosen tasks for today with `update_task`
   (`scheduled: <today>`); a new one goes into today's journal note
   (`journal/<today>.md`, created if missing) with `append_note` as `- [ ] text`.
5. Reply with the three items and nothing else. No plan card is needed: this is a
   two-step routine.

Dates are always written as YYYY-MM-DD. Never mark a task done on the user's behalf.
