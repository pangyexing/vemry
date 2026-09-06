---
name: research
description: The research method — a fixed plan skeleton, the vault before the web, verify and cite every source, and one rule for where the result lands
---

# Research method

Follow this for any "look into / search for / tell me about X" request.

## Shape of the work

1. A question with one clear answer needs no plan: answer it, cite it, stop.
2. Anything wider follows the standard plan below: call update_plan with it (all
   pending), then keep it current as you go — doing when you start a step, done
   when it is finished, skipped (with the reason in its text) when you drop it.
   Never restate the plan in prose; the UI shows it.
3. A step is a terse verb phrase in the user's language, at most about 8 words
   (20 characters in CJK).

## The standard plan

Use these five steps, in this order and in the user's language. Add a step only
for something they do not cover; merge or skip one only with the reason in its
text. The same request should produce the same plan.

1. Search the vault (notes and clips) for what is already known.
2. Search the web one direction at a time — 2 to 4 directions, such as what
   happened, why, how it is valued, what is disputed.
3. Open the key pages and verify them (open_for_reading, up to 5 URLs per call;
   at least 3 pages across at least 2 directions before writing).
4. Write the note, by the landing rule below, in one write_note.
5. Fill in the frontmatter (`sources:`, `status:`) and list the open questions.

## Where the result lands — a rule, not a choice

1. The turn context names a working note ("Working note: …", the note the
   session follows) → the findings go there: append_note adds a section,
   set_frontmatter records `status:` and `sources:`, edit_note fixes. Those
   writes on the working note apply without a dialog. A note attached as
   context (the one the user has open, or an @ reference) is reference
   material, not the working note.
2. The user names a note or a folder → there.
3. Otherwise a new note `research/<topic>.md`, the topic being the question in a
   few words in the user's language — created once, in one write_note, when the
   investigation is done. Never append the result to whichever note happens to
   be open, and never leave a systematic investigation as a chat answer only.
4. A follow-up on the same topic in the same session updates that note
   (edit_note / append_note) rather than creating a second one.
5. Before creating, look for a note on the same topic (search_notes, or
   list_notes on research/): one that exists is updated in place — its
   conclusion, claims and sources brought up to date — rather than joined by a
   near-duplicate under a slightly different name. Only a request for a fresh
   note gets a new one, named with the date.

## Order: the vault first, then the web

1. Search the vault first with search_notes / query_blocks — do not go looking
   for what the user already knows; the clips/ directory holds clipped pages,
   which count as existing knowledge too.
2. Before citing a web URL, call clips_lookup: a page that has been clipped is
   cited as a [[wikilink]] to the local note (and read with read_note), not as a
   URL.
3. Only when the vault has nothing, or nothing recent enough, search the web
   (if web search is enabled in the settings). A search gives titles and
   snippets; to actually read a page, call open_for_reading — the user approves
   the URLs on a card (or has let the session read without asking) and each page
   comes back as text, which then counts as a verified source. Read one
   direction's pages, write its conclusion down, then move on; do not queue
   every result.
4. Stop searching a direction once new results only repeat what has been read.
5. In a long conversation, call list_sources before citing again so one source
   does not get two entries; a source marked unverified there was never confirmed
   by a search — say so, or search to confirm it.

## Citation discipline

- Every factual claim gets a source right after it: `[title](url)` for the web,
  `[[note#^id]]` for the vault (read_block is enough when a citation is all you need).
- A number carries its date (a quarter, a report date); a figure without one is
  not a fact yet.
- Distinguish a quotation (verbatim) from a paraphrase (summarised); mark an
  inference with no source as unverified.
- When sources disagree, say so rather than silently picking one.
- Read one direction to its conclusion and write that conclusion down before
  starting the next; do not carry whole pages from step to step.

## The note's shape

The same shape every time, so a second investigation of the same kind reads
like the first:

- Frontmatter: `question:` (the request in one line), `status:` (draft →
  reviewing → settled), `sources:` (the URLs and [[notes]] the claims rest on).
- Body, in this order:
  1. A `> [!summary] <title>` callout, the title being "Conclusion" in the
     user's language, with the conclusion in at most five sentences — what is
     true, what is uncertain, what to watch.
  2. One `##` section per direction; each claim followed by its source.
  3. `## Disputed` — where sources disagree, both sides with their sources
     (omit the section when nothing is disputed).
  4. `## Open questions` — as `- [ ]` tasks, so they show up in the Agenda.
- Related vault notes are `[[wikilinks]]` — a `related:` list in the
  frontmatter and `[[path]]` in the body — never bare paths in prose, so the
  graph and the backlinks see them.
- Suggest that a page worth keeping be opened in the built-in browser and
  clipped into clips/.
