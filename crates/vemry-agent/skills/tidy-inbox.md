---
name: tidy-inbox
description: File the captures in inbox.md — grouped by theme, a home proposed for each, moved as blocks
triggers: [file the inbox, sort the inbox, inbox zero, process my captures, file my captures, where should these go, 整理速记, 整理收件箱, 清理收件箱, 归类速记, 受信箱を整理, 速記を振り分け, インボックスゼロ, 인박스 정리, 메모 분류, 수신함 비우기, trier la boîte de réception, classer les notes rapides, vider l'inbox, Eingang sortieren, Notizen einsortieren, Inbox leeren, ordenar la bandeja de entrada, archivar las notas rápidas, vaciar el inbox]
version: 1
---

# File the inbox

## When

"File / sort / clean up the inbox", "inbox zero", "where should these go". The
object is the captures inside one note — inbox.md, or the journal or pinned note
quick capture lands in. A folder is tidy, one note's own structure is tidy-note,
not this.

## Plan

1. Read the inbox and the vault's layout
2. Group the captures and pick a home for each
3. Review the list on the card
4. Move, one destination at a time
5. Report what moved and what stayed

## Inputs

- read_note the inbox whole: inbox.md unless the user attached (@) another note.
  A capture is a list item ending in a stamp, `- text *(YYYY-MM-DD HH:MM)*`, with
  its continuation lines (a second line, an image embed) indented under it; the
  handle read_note shows (^id on the line, or a positional bN) is what you move.
  Anything else in the note — a heading, a paragraph the user wrote — is not a
  capture and stays.
- VEMRY.md's structure and conventions, and list_notes: the homes are the user's
  existing notes and sections, and their names are the vocabulary the captures
  are matched against. search_notes for a capture whose words name a note you
  have not seen.

## Do

- Group the captures by what they are about, then give each group a home: the
  note (and the section, when the note has one for it) where the user would look
  for it — a project note for a thought about that project, a reading note for a
  remark on that paper, the note a capture links to when it links to one. A
  capture that is a task belongs where that project's tasks live. A capture with
  no home stays in the inbox and goes on the list as "leave it"; do not invent a
  note for one stray line — propose a new note only when three or more captures
  would fill it, and say so.
- propose_actions once with the whole list, then wait: text is the capture's
  first words, target the destination note, source the capture's handle as
  [[inbox#^id]] (or its bN). Nothing moves before the user has reviewed the list.
- Move only what came back: move_blocks from the inbox to each destination, one
  call per destination note with every block going there, anchor = the section
  agreed. A block travels whole — its stamp, its continuation lines, its embed —
  and its ^id and the links to it follow; never retype one. No leave_link: an
  inbox keeps no trace.
- Afterwards, a moved capture the user wants as a task, a claim or a cleaner line
  is one edit_note per block, reviewed on its own.

## Landing

The destination notes the card returned; the inbox loses the moved blocks and
nothing else.

## Shape

Before the card: the groups, one line each — the theme, the proposed home, the
handles under it. After the moves: what went where, what stayed and why.

## Never

- Delete a capture, or a line that is not one.
- Retype a capture instead of moving its block.
- write_note on the inbox: a whole-note rewrite loses what you did not see, and a
  note holding a private block refuses it.
- Move a capture the user did not keep on the card.
