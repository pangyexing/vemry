---
name: tidy-note
description: Restructure one messy note — outline first, blocks moved not retyped
triggers: [restructure, reorganise, reorganize, clean up this note, outline this, 重整, 重构笔记, 理一下这篇, ノートを整理, 構成し直す, 노트 정리, 구조 다시 잡기, restructurer cette note, réorganiser, Notiz umstrukturieren, neu gliedern]
version: 2
---

# Tidy one note

## When

"Reorganise / clean up / restructure" one note. A folder is tidy, not this.

## Plan

1. Read the whole note
2. Propose the outline
3. Restructure once agreed
4. Move blocks that belong elsewhere
5. Offer wording fixes one by one

## Inputs

read_note the whole note first. Every block handle it shows (^id on a line, or a
positional bN from query_blocks) is what you will move; never retype a block's
text.

## Do

- Propose the outline in the reply before touching anything: the sections you
  would make, which blocks go under each (by handle, with their first words so
  the user can follow), which look like duplicates, which are really tasks or
  claims and should become one. Ask which blocks to drop; drop nothing on your
  own.
- Once the user agrees, call restructure_note once with the outline. Reuse the
  note's existing headings by their exact text (their ids stay); a block you do
  not place lands under "Unassigned" — say so if that happens. Private blocks are
  not yours to move and stay with their neighbours by themselves.
- Blocks that belong in another note: move_blocks takes them there with their
  ids and rewrites the vault's links to them; leave_link when the reader of the
  old note should find them.
- Only after the structure is settled, offer wording fixes as separate edit_note
  calls, one per block, so each is reviewed on its own.

## Landing

The note itself; the other note for moved blocks.

## Shape

The outline in the reply: one line per section, the handles under it.

## Never

- write_note on a note that already exists: a whole-note rewrite retypes every
  block and loses what the user did not see you change.
- Dropping a block the user did not name.
