---
name: daily-close
description: "The evening routine: what got done and what changed today, appended to today's journal note"
version: 1
---

# Daily close

When the user asks to close the day (or "what did I do today"):

1. `recent_changes` with `since: <today>` and `kind: done` gives the tasks finished
   today; `recent_changes` with `since: <today>` alone gives the notes touched (a few
   blocks per note is enough — group by note, do not list every block).
2. Tasks still `doing` (`query_blocks`, `task_state: doing`) are tomorrow's
   carry-over: name them.
3. Write the close as one section appended to today's journal note
   (`journal/<today>.md`, created if missing) with `append_note`:
   `## Close` → done (cite each task as `[[note#^id]]`) → touched notes → carry-over.
   Propose it; the journal is the user's, so this write asks unless the journal is
   the working note.
4. One paragraph in the reply, then stop. A close nobody reads is worse than none.
