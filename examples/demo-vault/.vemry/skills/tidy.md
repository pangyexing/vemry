---
name: tidy
description: Tidying a folder or the vault: audit first, propose every move and fix as a list the user reviews, then move with links rewritten — nothing is lost
---

# Tidy up

When the user asks to tidy, organise or clean up a folder (or the inbox, or the
whole vault):

1. Read VEMRY.md's structure and conventions first: the target layout is the
   user's, not yours. Then `list_notes` and `list_files` for the folder, and
   `audit_links` for what is broken there (unresolved links, orphans, dangling
   supports, duplicate clips).
2. Decide each item's destination by the conventions (a project note into its
   project folder, a clip that belongs to a topic next to it, an attachment into
   the note's bucket). When two notes are the same thing, plan a merge: the
   surviving note gets the other's name as an `aliases:` entry (set_frontmatter)
   so old links keep resolving, and the other moves to `archive/`.
3. Call `propose_actions` once with the whole list and wait. Use `text` to say
   the move in words ("move inbox/压测记录.md → projects/网关/会议/"), `target` for
   the destination, `source` for the reason (an audit row, the convention).
   Nothing moves before the user has reviewed the list.
4. Execute only what came back: `move_note` and `move_file` (links are rewritten
   in the same write), `set_frontmatter` for aliases and missing keys,
   `append_note` for a hub note's new entries. One tool call per item.
5. Reply with what moved where and what was left alone, then run `audit_links`
   again and report whether the unresolved count went down.

Never delete. Never move into or out of a private folder. A file you cannot
place goes into the list as "leave it" so the user decides.
