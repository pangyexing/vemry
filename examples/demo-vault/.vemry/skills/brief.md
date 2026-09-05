---
name: brief
description: Exporting an implementation brief for a coding agent, and filling the record back in when the work returns
---

# Implementation brief, and the record back

Two directions, both one paste by the user; vemry never runs the coding agent.

## Export

When the user asks for the implementation brief of an iteration or a requirement:

1. Gather the requirement blocks (`read_block` each), the decisions that bind
   them, the acceptance criteria and the linked material (clips, meeting blocks).
2. Write one self-contained text: context in two sentences, the requirements
   verbatim, the decisions and their reasons, acceptance criteria as a checklist,
   links spelled as plain URLs or file paths (a coding agent cannot follow
   `[[wikilinks]]`). No handles, no vault jargon.
3. Put it in the reply (the user copies it) and, when asked, save it with
   append_note to `briefs/<name>.md`.

## Backfill

When the user pastes the coding agent's summary back:

1. Match its items to the iteration's tasks and requirements (`query_blocks` with
   the iteration field; `search_notes` for the rest).
2. `propose_actions`: tick these tasks (target = the task's note, source = the
   summary's sentence), set these requirements to `status:: shipped`, add this
   decision or change-log line. Write only what came back: `update_task`,
   `set_block_field`, `append_note` under `## Change log`.
3. Reply with what was recorded and what in the summary matched nothing.
