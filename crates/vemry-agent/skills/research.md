---
name: research
description: Look into a topic — the vault first, verified web sources, one note
triggers: [research, look into, survey, investigate, 调研, 研究, 了解一下, 综述]
version: 2
---

# Research method

## When

"Look into / research / tell me about / survey X" — anything wider than one
question. A question with one clear answer needs none of this: answer it, cite
it, stop.

## Plan

The five steps, verbatim (in the user's language), in this order. Add a step only
for something they do not cover; merge or skip one only with the reason in its
text. The same request produces the same plan.

1. Search the vault
2. Search the web by direction
3. Read and verify key pages
4. Write the note
5. Fill frontmatter, list open questions

## Inputs

- The working note, when the turn context names one ("Working note: …"): the
  findings go there. A note attached as context (the open note, an @ reference)
  is material, not the target.
- search_notes / query_blocks first — the vault is what the user already knows,
  and clips/ holds clipped pages, which count too. clips_lookup before citing any
  URL: a clipped page is cited as its [[wikilink]] and read with read_note.
- The web only for what the vault lacks or has too old: a search gives titles and
  snippets; open_for_reading (up to 5 URLs a call) gives the page, which then
  counts as a verified source.

## Do

- Directions: 2 to 4 (what happened, why, how it is valued, what is disputed).
  Read one direction's pages, write its conclusion down, then move on; do not
  queue every result, and do not carry whole pages from step to step.
- Stop a direction once new results only repeat what has been read.
- At least 3 pages across at least 2 directions before writing.
- In a long conversation, list_sources before citing again, so one source does
  not get two entries; a source marked unverified there was never confirmed —
  say so, or search to confirm it.
- Every factual claim gets its source right after it: [title](url) for the web,
  [[note#^id]] for the vault (read_block is enough when a citation is all you
  need). Quotation and paraphrase are told apart; an inference with no source is
  marked unverified; when sources disagree, say so rather than pick one.

## Landing

1. The working note → append_note adds the section, set_frontmatter records
   status: and sources:, edit_note fixes; these writes on the working note apply
   without a dialog.
2. A note or folder the user named → there.
3. Otherwise research/<topic>.md, the topic being the question in a few words in
   the user's language — one write_note when the investigation is done. First
   search_notes (or list_notes on research/) for a note on the same topic: one
   that exists is updated in place, not joined by a near-duplicate under a
   slightly different name; only a request for a fresh note gets a new one,
   named with the date.
4. A follow-up on the same topic in the same session updates that note.

## Shape

- Frontmatter: question: (the request in one line), status: (draft → reviewing
  → settled), sources: (the URLs and [[notes]] the claims rest on), related:
  (the [[notes]] it connects to).
- Body, in this order:
  1. A > [!summary] callout titled "Conclusion" in the user's language: at most
     five sentences — what is true, what is uncertain, what to watch.
  2. One ## section per direction; each claim followed by its source.
  3. ## Disputed — both sides with their sources (omitted when nothing is
     disputed).
  4. ## Open questions — as - [ ] tasks, so they show in the Agenda.
- Related vault notes as [[wikilinks]] in the body and in related:, never bare
  paths.

## Never

- A number without its date (a quarter, a report date): not a fact yet.
- Appending the result to whichever note happens to be open, or leaving a
  systematic investigation as a chat answer only.
- A bare path where a [[wikilink]] belongs.
