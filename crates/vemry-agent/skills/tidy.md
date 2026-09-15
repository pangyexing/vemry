---
name: tidy
description: Tidy a folder or the vault — audit, review every move, links rewritten
triggers: [tidy, organise, organize, clean up, inbox zero, archive, 整理, 归档, 清理收件箱, 整理する, アーカイブ, 片付け, 정리, 보관, 인박스 비우기, ranger, archiver, trier, aufräumen, archivieren, sortieren, ordenar, archivar, limpiar]
version: 2
---

# Tidy up

## When

"Tidy / organise / clean up" a folder, the inbox, or the whole vault. One note's
insides are tidy-note, not this.

## Plan

1. Read the conventions and audit
2. Decide every item's destination
3. Review the list on the card
4. Move and fix
5. Audit again and report

## Inputs

- VEMRY.md's structure and conventions first: the target layout is the user's,
  not yours.
- list_notes and list_files for the folder; audit_links for what is broken there
  (unresolved links, orphans, dangling supports, duplicate clips).

## Do

- Each item's destination by the conventions: a project note into its project
  folder, a clip that belongs to a topic next to it, an attachment into the
  note's bucket. Two notes that are the same thing: plan a merge — the surviving
  note gets the other's name as an aliases: entry (set_frontmatter) so old links
  keep resolving, and the other moves to archive/.
- propose_actions once with the whole list, then wait: text says the move in
  words ("move inbox/x.md → projects/y/"), target the destination, source the
  reason (an audit row, the convention). Nothing moves before the user has
  reviewed the list.
- Execute only what came back: move_note and move_file (links are rewritten in
  the same write), set_frontmatter for aliases and missing keys, append_note for
  a hub note's new entries. One tool call per item.
- audit_links again afterwards.

## Landing

The destinations the card returned; archive/ for the merged-away note.

## Shape

The reply: what moved where, what was left alone, and whether the unresolved
count went down.

## Never

- Delete anything.
- Move into or out of a private folder.
- Place a file you cannot place: it goes on the list as "leave it", and the user
  decides.
