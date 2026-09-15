---
name: feedback-triage
description: Feedback into evidence blocks — feature, sentiment, source, linked to its claim
triggers: [feedback, triage, reviews, interview notes, complaints, 反馈, 归集, 用户评价, フィードバック整理, レビューをまとめる, 피드백 분류, 사용자 리뷰, trier les retours, avis utilisateurs, Feedback sichten, Nutzerbewertungen]
version: 2
---

# Feedback triage

## When

The user pastes feedback, interview notes or reviews, or points at a clipped
page of them.

## Plan

1. Split the feedback into items
2. Link each to what it bears on
3. Review the batch on the card
4. Write and count

## Inputs

- The pasted text, the note, or the clip.
- query_blocks on the PRD first, for the id of the requirement or decision an
  item bears on.

## Do

- One piece of feedback, one block: a > [!evidence] block with [feature:: …]
  [sentiment:: positive|negative|mixed] [source:: …] and, when it bears on a
  requirement or a decision, a supports:[[PRD#^id]] tail at the end of the
  block's text (not a field; edit_note adds it to an existing block).
- Keep the user's words for the quote; your classification goes in the fields,
  never into the text.
- propose_actions with the whole batch (text = the quote, target = the feedback
  note, fields = feature / sentiment / source) and write only what came back,
  one append_note per target.
- Finish with aggregate_blocks (kind: evidence, note: <feedback note>,
  group_by: feature).

## Landing

projects/<X>/feedback.md, created if missing.

## Shape

The reply ends with the counts per feature and names the requirements that
gained or lost support.

## Never

- Rewording a quote.
- Putting the classification into the text instead of the fields.
