---
name: decision-log
description: Decisions and their evidence — which have none, what contradicts, what superseded
triggers: [decision log, decisions, why did we decide, evidence for, 决策记录, 决策依据, 为什么这么定, 意思決定ログ, なぜそう決めた, 의사결정 기록, 왜 그렇게 결정했나, journal des décisions, pourquoi cette décision, Entscheidungslog, warum wurde das entschieden, registro de decisiones, por qué se decidió]
version: 2
---

# Decision log

## When

"What does this decision rest on", "does this feedback change it", a decision
review.

## Plan

1. List the decisions
2. Match evidence to each
3. Record new evidence
4. Report the table

## Inputs

- query_blocks with kind: claim and note: <PRD> — the PRD's claims and decisions
  (a decision is indexed as a claim); audit_links — the evidence whose supports:
  reaches nothing.
- For each decision, every > [!evidence] block with supports:[[PRD#^id]]:
  query_blocks with kind: evidence and a text filter on the id.

## Do

- A decision with no evidence is the finding, not a gap to fill by guessing.
- New feedback that bears on a decision becomes an evidence block in the feedback
  note with a supports:[[PRD#^id]] tail pointing at the decision (append_note
  for a new block, edit_note to add the tail to an existing one; the user then
  sees the link in the Claims view). Feedback that contradicts it is said so in
  the reply; the decision is superseded only when the user decides.
- A superseding decision is a new > [!decision] block with
  [supersedes:: [[#^old]]]; the old block is not edited or removed.

## Landing

The feedback note for evidence; the PRD for a superseding decision.

## Shape

The reply: a table — decision · evidence count · latest evidence date · status.

## Never

- Inventing evidence for a decision that has none.
- Editing or removing a superseded decision.
