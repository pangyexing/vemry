---
name: decision-log
description: "Decisions and their evidence: which decisions have none, which feedback contradicts one, what a decision superseded"
version: 1
---

# Decision log

When the user asks what a decision rests on, whether new feedback changes it, or
for a decision review:

1. `query_blocks` with `kind: claim` and `note: <PRD>` lists the PRD's claims and
   decisions (a decision is indexed as a claim); `audit_links` names the evidence
   whose `supports:` reaches nothing.
2. For each decision, the evidence that supports it is every `[!evidence]` block
   with `supports:[[PRD#^id]]` — `query_blocks` with `kind: evidence` and a `text`
   filter on the id finds them. A decision with none is the finding, not a gap to
   fill by guessing.
3. New feedback that bears on a decision becomes an evidence block in the
   feedback note with a `supports:[[PRD#^id]]` tail pointing at the decision
   (append_note for a new block, edit_note to add the tail to an existing one; the
   user then sees the link in the Claims view); feedback that contradicts it is said so
   in the reply, and the decision is only superseded when the user decides.
4. A superseding decision is a new `> [!decision]` block with
   `[supersedes:: [[#^old]]]`; the old block is not edited or removed.
5. Reply with a table: decision · evidence count · latest evidence date · status.
