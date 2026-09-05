---
name: prd
description: From scattered input to a PRD: background, goal, requirement blocks with their sources, decisions, open questions, a change log
---

# PRD

When the user wants a product requirements document (or wants one grown) out of
feedback clips, meeting notes and competitor clips:

1. Work in a working note (`projects/<X>/PRD.md`; ask the user to follow it, or
   create it with append_note). Frontmatter: `status: draft`, `project:`.
2. Structure: `## Background`, `## Goal`, `## Requirements`, `## Decisions`,
   `## Open questions`, `## Feedback` (a `vemry-query` block pulling the evidence
   that supports this PRD's blocks), `## Change log`.
3. Every requirement is a `> [!requirement]` block with `[priority:: P0|P1|P2]
   [status:: draft] [owner:: …]` on its line and a citation of where it came from
   right after (the feedback clip, the meeting block). One requirement, one block.
4. Every decision is a `> [!decision]` block with `[date:: YYYY-MM-DD]`; when it
   replaces an earlier one, `[supersedes:: old-id]` (a field value cannot hold a
   wikilink). Decisions are claims: feedback supports them with the evidence
   block's `supports:[[PRD#^id]]` tail, written with edit_note.
5. An unresolved point is a `> [!question]` block with an owner; do not answer it
   yourself.
6. Append, do not rewrite: new requirements go under `## Requirements` with
   append_note; a status change is set_block_field; a changed requirement is
   edit_note on that block alone, plus a line under `## Change log` saying what
   changed and why (cite the decision).
7. Never invent a requirement no input supports. Say what is missing instead.
