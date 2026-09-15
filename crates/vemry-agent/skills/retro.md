---
name: retro
description: Close an iteration — shipped, hypotheses, decisions, carry-over, release notes
triggers: [retro, retrospective, release notes, close the iteration, 复盘, 迭代总结, 发布说明, ふりかえり, リリースノート, 회고, 릴리스 노트, rétrospective, notes de version, Retrospektive, Release-Notes, retrospectiva, notas de la versión]
version: 2
---

# Iteration retro

## When

An iteration ends and the user asks for its review or its release notes.

## Plan

1. Collect done and not done
2. Judge the hypotheses
3. List decisions and feedback trend
4. Append the retro
5. Carry over on the card

## Inputs

- query_blocks with kind: task and field: "iteration = <name>" — done and not
  done; recent_changes since the iteration's start with kind: done — what was
  finished this iteration in fact.
- The iteration's #hypothesis tasks and their status:: — validated / refuted /
  inconclusive; one open past its review date is overdue.
- query_blocks with kind: claim and field: "date >= <start>" — decisions taken
  or superseded during the iteration.
- aggregate_blocks (kind: evidence, group_by: feature) — the feedback trend,
  against the previous retro's numbers when the notes hold them.

## Do

- Append ## Retro to the iteration note.
- propose_actions for the carry-over: each unfinished task with fields:
  {iteration: <next>}; the kept rows get set_block_field iteration.
- Release notes are the same material for other readers: no wikilinks, no
  handles, plain sentences — in the reply, or written to releases/<name>.md
  when the user asks.

## Landing

The iteration note (append_note); releases/<name>.md only on request.

## Shape

## Retro: done · not done · hypotheses · decisions · feedback · carry-over.

## Never

- Moving a task to the next iteration the card did not return.
- Marking a hypothesis validated without its status:: saying so.
