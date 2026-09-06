---
name: iteration-plan
description: "Planning the next iteration: score the backlog, pick within capacity, tag the chosen blocks with the iteration, write the iteration note"
version: 1
---

# Iteration plan

When the user asks what the next iteration (sprint, version) should hold:

1. The backlog is every requirement not yet in an iteration: `query_blocks` with
   `kind: claim` is not it — requirements are callouts; use `search_notes` on the
   PRD and `query_blocks` with `field: "status = accepted"` and no `iteration`
   (`field: iteration` absent means unassigned; list those without it).
2. Score each candidate on reach / impact / confidence / effort (1–5 each; score
   = reach × impact × confidence ÷ effort) from what the notes say — cite the
   evidence for the numbers, and leave a number blank rather than invent it.
3. Call `propose_actions` once: text = the requirement, target = its note,
   source = its handle, fields = reach / impact / confidence / effort / score.
   The user edits the numbers on the card; the kept rows are the iteration.
4. Write what came back: `set_block_field` on each block for the four numbers,
   `score`, and `iteration:: <name>` (the board grouped by iteration is the
   iteration view). Then create `iterations/<name>.md` with append_note: goal,
   the hypotheses this iteration tests (each a hypothesis block with a review
   date), a `vemry-query` block (`kind: task`, `field: iteration = <name>`) for its
   tasks, and acceptance criteria.
5. Capacity is the user's number (tasks or a `estimate::` sum); say when the
   picked set exceeds it.
