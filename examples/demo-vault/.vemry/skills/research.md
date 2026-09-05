---
name: research
description: The research method — plan first, the vault before the web, verify and cite every source, and let the conclusion settle into a note
---

# Research method

Follow this for any 'look into / search for / tell me about X' request.

## Shape of the work

1. A question with one clear answer needs no plan: answer it, cite it, stop.
2. Anything wider is a plan first: call update_plan with 3-7 steps (all pending),
   then keep it current as you go — doing when you start a step, done when it is
   finished, skipped (with the reason in its text) when you drop it. Never
   restate the plan in prose; the UI shows it.
3. When the turn context names a working note, that note is where the findings
   go: append_note adds a section or a finding, set_frontmatter records
   `status:` and `sources:`. Those writes on the working note apply without a
   dialog. Without a working note, propose to save the result as a note and let
   the user decide.

## Order: the vault first, then the web

1. Search the vault first with search_notes / query_blocks — do not go looking
   for what the user already knows; the clips/ directory holds clipped pages,
   which count as existing knowledge too.
2. Before citing a web URL, call clips_lookup: a page that has been clipped is
   cited as a [[wikilink]] to the local note (and read with read_note), not as a
   URL.
3. Only when the vault has nothing, or nothing recent enough, search the web
   (if web search is enabled in the settings). A search gives titles and
   snippets; to actually read a page, call open_for_reading with up to 5 URLs —
   the user approves them on a card and each page comes back as text, which then
   counts as a verified source. Read one direction's pages, write its conclusion
   down, then move on; do not queue every result.
4. In a long conversation, call list_sources before citing again so one source
   does not get two entries; a source marked unverified there was never confirmed
   by a search — say so, or search to confirm it.

## Citation discipline

- Every factual claim gets a source right after it: `[title](url)` for the web,
  `[[note#^id]]` for the vault (read_block is enough when a citation is all you need).
- Distinguish a quotation (verbatim) from a paraphrase (summarised); mark an
  inference with no source as unverified.
- When sources disagree, say so rather than silently picking one.
- Read one direction to its conclusion and write that conclusion down before
  starting the next; do not carry whole pages from step to step.

## Writing it down

- A one-off question only needs an answer.
- A systematic investigation becomes a note: the frontmatter records
  `question:`, `status:` (draft → reviewing → settled) and `sources:`; the body
  is organised as conclusion → each claim with its source → open questions, and
  [[links]] connect it to related notes.
- Suggest that a page worth keeping be opened in the built-in browser and
  clipped into clips/.
