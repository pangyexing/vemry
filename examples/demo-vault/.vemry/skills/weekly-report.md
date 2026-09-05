---
name: weekly-report
description: The weekly report: what shipped this week, what is blocked, what is planned next — appended to the week's note, with a copyable version for others
---

# Weekly report

When the user asks for the week's report or review:

1. The week is Monday to the date in the turn context. `recent_changes` with
   `since: <monday>` and `kind: done` gives what was finished; `query_blocks` with
   `task_state: todo`, `due_before: <today>` gives what slipped; `task_state: doing`
   gives what is open.
2. `query_blocks` with `kind: claim` and a look at their `supports::` links names
   the claims still without evidence — a research week has those too.
3. Read this week's journal notes (`journal/<date>.md`) for promises made and
   blockers named in prose.
4. Write the report as one section: **Done** (cite each task as `[[note#^id]]`),
   **Blocked / slipped**, **Next week**. Append it to the week's note
   (`journal/<year>-W<week>.md`, created if missing) with `append_note`; propose
   the write.
5. Offer a second version for other people in the reply itself: no wikilinks, no
   handles, plain sentences — the user copies it. It is never written to the vault.

Keep both under a page.
