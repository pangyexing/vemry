---
name: brief
description: An implementation brief for a coding agent, and its summary recorded back
triggers: [brief, implementation brief, hand off, backfill, coding agent, 简报, 交给编码 agent, 回填]
version: 2
---

# Implementation brief, and the record back

## When

Two directions, both one paste by the user; vemry never runs the coding agent.
Export: "the brief for this iteration / requirement". Backfill: the user pastes
the coding agent's summary back.

## Plan

For an export:
1. Gather the blocks
2. Write the brief

For a backfill:
1. Match the summary to the blocks
2. Review the changes on the card
3. Record what came back

## Inputs

- Export: the requirement blocks (read_block each), the decisions that bind
  them, the acceptance criteria, the linked material (clips, meeting blocks).
- Backfill: the pasted summary; query_blocks with the iteration field and
  search_notes for the items it names.

## Do

- Export: one self-contained text — context in two sentences, the requirements
  verbatim, the decisions and their reasons, acceptance criteria as a checklist,
  links spelled as plain URLs or file paths (a coding agent cannot follow
  [[wikilinks]]). No handles, no vault jargon.
- Backfill: propose_actions — tick these tasks (target = the task's note, source
  = the summary's sentence), set these requirements to status:: shipped, add
  this decision or change-log line. Write only what came back: update_task,
  set_block_field, append_note under ## Change log.

## Landing

Export: the reply (the user copies it); briefs/<name>.md with append_note when
asked. Backfill: the tasks' notes, the PRD, the change log.

## Shape

The reply after a backfill: what was recorded, and what in the summary matched
nothing.

## Never

- Running or scheduling the coding agent.
- Recording an item the card did not return.
