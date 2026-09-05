---
name: feedback-triage
description: User feedback into blocks with feature, sentiment and source, linked to the requirement or decision it bears on
---

# Feedback triage

When the user pastes feedback, interview notes or reviews, or points at a clipped
page of them:

1. One piece of feedback, one block, in the project's feedback note
   (`projects/<X>/feedback.md`, created if missing): a `> [!evidence]` block with
   `[feature:: …] [sentiment:: positive|negative|mixed] [source:: …]` and, when it
   bears on a requirement or a decision, a `supports:[[PRD#^id]]` tail at the end of
   the block's text (not a field; edit_note adds it to an existing block) — find the
   id with `query_blocks` on the PRD first.
2. Keep the user's words for the quote; your classification goes in the fields,
   never into the text.
3. Propose the whole batch with `propose_actions` (text = the quote, target = the
   feedback note, fields = feature / sentiment / source) and write only what came
   back, one append_note per target.
4. Finish with `aggregate_blocks` (`kind: evidence`, `note: <feedback note>`,
   `group_by: feature`) so the reply ends with the counts per feature, and name the
   requirements that gained or lost support.
