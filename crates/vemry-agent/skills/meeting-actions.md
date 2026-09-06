---
name: meeting-actions
description: Meeting notes into action items — who, what, by when, reviewed on a card
triggers: [action items, meeting notes, minutes, todos from the meeting, 会议记录, 行动项, 待办]
version: 2
---

# Meeting notes into action items

## When

The user pastes or points at meeting notes and wants the actions out of them.

## Plan

1. Read the notes
2. Extract and deduplicate the items
3. Review the list on the card
4. Write the kept items

## Inputs

- The notes: the attached selection, the note, or the pasted text.
- The date in the turn context, for resolving relative dates.
- query_blocks with kind: task and a text filter on each item's key words — an
  item that already exists is not proposed again; mention it instead.
- search_notes for the project note each item belongs to.

## Do

- Every commitment: who does what by when. Keep the user's words for the task
  text; put the sentence it came from in source (a [[note#^id]] when the notes
  are a vault note, the quoted sentence otherwise).
- Relative dates resolved against the turn's date and written as YYYY-MM-DD. No
  owner means no owner — do not guess one.
- propose_actions once with the whole list, then wait. The card is the user's
  review: they untick, edit dates, owners and targets.
- Write only what came back.

## Landing

Each item's target note — the project note the topic belongs to, else the note
the meeting notes live in, else inbox.md — with append_note under a ## Actions
heading when the note has one (anchor), else at the end. One write per target
note.

## Shape

- [ ] text [owner:: x] 📅 YYYY-MM-DD per item. The reply: one line per item
written and where it landed.

## Never

- An owner or a date the notes do not give.
- Writing an item the card did not return.
