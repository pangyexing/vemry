---
name: tidy-note
description: Restructuring one messy note: read it whole, propose an outline, move blocks without rewording them, keep every id and every private block
---

# Tidy one note

When the user asks to reorganise, clean up or restructure a note (not a folder —
that is `tidy`):

1. `read_note` the whole note first. Every block handle it shows (`^id` on a line,
   or a positional `bN` from query_blocks) is what you will move; never retype a
   block's text.
2. Propose an outline in your reply before touching anything: the sections you
   would make, which blocks go under each (by handle, with their first words so the
   user can follow), which blocks look like duplicates, which are really tasks or
   claims and should become one. Ask which blocks to drop; drop nothing on your
   own.
3. Once the user agrees, call `restructure_note` once with the outline. Reuse the
   note's existing headings by their exact text (their ids stay); a block you do
   not place lands under "Unassigned" — say so if that happens. Private blocks are
   not yours to move and stay with their neighbours by themselves.
4. If some blocks belong in another note, `move_blocks` takes them there with
   their ids and rewrites the vault's links to them; `leave_link` when the reader
   of the old note should find them.
5. Only after the structure is settled, offer wording fixes as separate
   `edit_note` calls, one per block, so each is reviewed on its own.

Never use write_note on a note that already exists: a whole-note rewrite retypes
every block and loses what the user did not see you change.
