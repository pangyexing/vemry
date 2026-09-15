---
name: prd
description: Scattered input into a PRD — requirement, decision and question blocks
triggers: [prd, requirements, spec, product document, 需求文档, 产品文档, 写需求, 要件定義, 仕様書, 요구사항 문서, 기획서, cahier des charges, spécification, Anforderungsdokument, Spezifikation]
version: 2
---

# PRD

## When

The user wants a product requirements document (or wants one grown) out of
feedback clips, meeting notes and competitor clips.

## Plan

1. Gather the inputs
2. Draft or extend the sections
3. Write requirements and decisions as blocks
4. Record the change log

## Inputs

- The working note (projects/<X>/PRD.md): ask the user to follow it, or create
  it with append_note.
- The feedback clips, meeting blocks and competitor clips the user points at,
  and search_notes for the rest; query_blocks on the PRD for the ids a citation
  needs.

## Do

- Every requirement is a > [!requirement] block with [priority:: P0|P1|P2]
  [status:: draft] [owner:: …] on its line and a citation of where it came from
  right after (the feedback clip, the meeting block). One requirement, one
  block.
- Every decision is a > [!decision] block with [date:: YYYY-MM-DD]; when it
  replaces an earlier one, [supersedes:: old-id] (a field value cannot hold a
  wikilink). Decisions are claims: feedback supports them with the evidence
  block's supports:[[PRD#^id]] tail, written with edit_note.
- An unresolved point is a > [!question] block with an owner; do not answer it
  yourself.
- Append, do not rewrite: new requirements under ## Requirements with
  append_note; a status change is set_block_field; a changed requirement is
  edit_note on that block alone, plus a line under ## Change log saying what
  changed and why (cite the decision).

## Landing

projects/<X>/PRD.md, frontmatter status: draft and project:.

## Shape

## Background, ## Goal, ## Requirements, ## Decisions, ## Open questions,
## Feedback (a vemry-query block pulling the evidence that supports this PRD's
blocks), ## Change log.

## Never

- A requirement no input supports: say what is missing instead.
- Rewriting the whole note.
