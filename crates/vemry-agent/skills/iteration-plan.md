---
name: iteration-plan
description: Plan the next iteration — score the backlog, pick within capacity, tag, write
triggers: [iteration, sprint, next version, plan the release, backlog, 迭代规划, 下一轮, 排期, イテレーション計画, 次のバージョン, 이터레이션 계획, 다음 버전, planifier l'itération, prochaine version, Iterationsplanung, nächste Version, planificar la iteración, próxima versión]
version: 2
---

# Iteration plan

## When

"What should the next iteration (sprint, version) hold."

## Plan

1. Collect the backlog
2. Score the candidates
3. Review the scores on the card
4. Tag the chosen blocks
5. Write the iteration note

## Inputs

- The backlog: every accepted requirement not yet in an iteration —
  search_notes on the PRD, query_blocks with field: "status = accepted", keeping
  those with no iteration field (requirements are callouts, not claims).
- The capacity: the user's number (tasks, or an estimate:: sum).

## Do

- Score each candidate on reach / impact / confidence / effort (1–5 each; score
  = reach × impact × confidence ÷ effort) from what the notes say — cite the
  evidence for the numbers, and leave a number blank rather than invent it.
- propose_actions once: text = the requirement, target = its note, source = its
  handle, fields = reach / impact / confidence / effort / score. The user edits
  the numbers on the card; the kept rows are the iteration.
- Write what came back: set_block_field on each block for the four numbers,
  score, and iteration:: <name> (the board grouped by iteration is the iteration
  view).
- Say when the picked set exceeds the capacity.

## Landing

iterations/<name>.md via append_note (created if missing).

## Shape

The iteration note: the goal, the hypotheses it tests (each a hypothesis block
with a review date), a vemry-query block (kind: task, field: iteration = <name>)
for its tasks, the acceptance criteria.

## Never

- A score with no evidence behind it.
- Tagging a block the card did not return.
