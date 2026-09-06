---
name: meeting-actions
description: "Meeting notes into action items: who, what, by when — each quoting its sentence, deduplicated, reviewed on the card, then written into the project notes"
version: 1
---

# Meeting notes into action items

When the user pastes or points at meeting notes and wants the actions out of them:

1. Read the notes (the attached selection, the note, or the pasted text). Extract
   every commitment: who does what by when. Keep the user's words for the task
   text; put the sentence it came from in `source` (a `[[note#^id]]` when the notes
   are a vault note, the quoted sentence otherwise).
2. Resolve every relative date against the date in the turn context and write it as
   YYYY-MM-DD. No owner means no owner — do not guess one.
3. Before proposing, look for duplicates: `query_blocks` with `kind: task` and a
   `text` filter on the item's key words. An item that already exists is not
   proposed again; mention it instead.
4. Decide each item's target note: the project note the topic belongs to
   (`search_notes` finds it), else the note the meeting notes live in, else
   `inbox.md`.
5. Call `propose_actions` once with the whole list and wait. The card is the
   user's review: they untick, edit dates, owners and targets.
6. Write only what came back, each item into its target with `append_note` as
   `- [ ] text [owner:: x] 📅 YYYY-MM-DD`, under a `## Actions` heading when the
   note has one (anchor), else at the end. One write per target note.
7. Reply with one line per item written and where it landed.
