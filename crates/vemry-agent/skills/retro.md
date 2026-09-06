---
name: retro
description: "Closing an iteration: what shipped, which hypotheses held, which decisions changed, what carries over — and the release notes"
version: 1
---

# Iteration retro

When an iteration ends and the user asks for its review or its release notes:

1. `query_blocks` with `kind: task` and `field: "iteration = <name>"` for done and
   not done; `recent_changes` since the iteration's start with `kind: done` for
   what was finished this iteration in fact.
2. Hypotheses: the `#hypothesis` tasks of the iteration — validated / refuted /
   inconclusive from their `status::`; an open one past its review date is named
   as overdue.
3. Decisions taken or superseded during the iteration: `query_blocks` with
   `kind: claim` and `field: "date >= <start>"`.
4. Feedback trend: `aggregate_blocks` (`kind: evidence`, `group_by: feature`) now
   versus the previous retro's numbers if the user's notes hold them.
5. Append `## Retro` to the iteration note (append_note): done · not done ·
   hypotheses · decisions · feedback · carry-over. Then `propose_actions` for the
   carry-over: each unfinished task with `fields: {iteration: <next>}` — the kept
   rows get `set_block_field iteration`.
6. Release notes are the same material for other readers: no wikilinks, no
   handles, plain sentences, offered in the reply (or written to
   `releases/<name>.md` when the user asks).
