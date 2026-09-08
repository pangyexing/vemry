---
name: study
description: Study a paper or a long text — import it, read it in chapters, check the retelling, cards from the gaps
triggers: [study, read this paper, work through, deep read, 精读, 研读, 学这篇, 读这篇论文, 带我读]
version: 1
---

# Study

## When

"Study / read / work through this paper (chapter, spec, long article)" — the
reader wants to understand it, not to be told about it. A question about one
passage needs none of this: answer it, cite the block, stop.

## Plan

The first session, verbatim (in the user's language):

1. Import the source
2. Map what is known and what to learn
3. Write the chapters
4. Cards and hub

A later session on the same paper has a plan of its own: "Check the retelling"
(one step), "Answer the questions" (one step), or "Add an experiment" (one step).
Mark each step done with update_plan as it lands — the card is the reader's
progress bar, not a receipt at the end.

## Inputs

- The source, in this order: a vault file (read_file — a PDF comes back as text),
  a clipped page (clips_lookup, then read_note), a URL (open_for_reading; an
  arXiv link means its HTML version alone, `https://arxiv.org/html/<id>`, which
  keeps the section structure — the abstract page adds nothing and is not
  requested). A long page comes back in parts — the
  result says `part 1 of N` — and the next part is one call away: the same URL
  with `part: 2`, then 3. Write each part's sections into the source note before
  asking for the next: the source note is the memory, not the context. Never look
  for the rest of a page anywhere else (the PDF, a mirror, the browser): it is not
  there, and every such attempt costs a minute.
- What the vault already holds: search_notes with the paper's key terms and
  query_blocks with kind: claim. The known map is the hits, linked with the
  snippet's gist — no read_note in this step (a clipped explainer read in full
  is ten thousand characters of someone else's words); a chapter the vault
  already explains is linked, not rewritten.
- What the reader reads it for, asked once at the start when the request does not
  say: understand the idea, reproduce it, write a survey, prepare for an interview,
  review it. The goal orders the chapters and picks the output.
- The genre, judged from the abstract and headings:

  | genre | chapters |
  |---|---|
  | method | the problem, the core mechanism, each component, training and implementation, results, ablations, limits |
  | empirical | the hypothesis, data and design, measures, main results, robustness checks, threats to validity |
  | theory | definitions and notation, the main theorem, the proof's shape, key lemmas, corollaries, boundary conditions |
  | survey | the taxonomy, each school, the axes of comparison, open problems |
  | system | goals and constraints, the architecture, the key design decisions, evaluation, trade-offs |

  A textbook chapter, an RFC or a long article takes the closest row. A vault
  copy of this skill may add rows for its own field.

## Do

- The source note: the paper's own section headings as `##`, one paragraph per
  block ending in an id — `^s<section digits>p<n>`: `^s3p2` is section 3
  paragraph 2, `^s321p4` is section 3.2.1 paragraph 4 — formulas as `$…$` and
  `$$…$$`, tables as tables, a figure as its image line
  `![Figure N: caption](url) ^figN` (`^fig2l` and `^fig2r` for a figure's two
  halves) with the URL exactly as the page gave it, followed by the caption's
  own line. The figures are part of the paper — an architecture diagram is often
  the one thing a reader remembers — and the id lets a chapter embed one. The id
  goes on the line as it is written: once the note lands, the app saves the
  pictures into the vault and rewrites the line's URL to the local file, so an
  edit_note against the remembered text no longer matches.
  Quotation, not paraphrase: this note is what the chapters cite. Frontmatter
  `title`, `authors`, `year`, `source` (the URL or the file), `genre`. The first
  part is a write_note; every later part is an append_note of its sections. The
  body only: the reference list and an appendix of figure captions are not
  archived (a chapter cites a reference by its number if it must). An id that
  exists is never changed or renumbered (chapters and cards cite it); a
  paragraph added later gets the next free number of its section.
- The hub: frontmatter `goal` and `status`; a known list (each item a [[link]] to
  the note that already explains it), a to-learn list, the chapter order for this
  goal (a chapter already known is marked so and skipped), and this query block,
  exactly, so the chapters and their status show live, one row per chapter:

  ```vemry-query
  tag: chapter
  text: "#chapter"
  ```

  (the fence language is `vemry-query`, one key per line; a tag alone matches
  every block of a tagged note, and the text filter keeps the `#chapter` line).
- Formulas: `$…$` in a sentence, a table cell, a question or a card, `$$…$$` on a
  line of its own — everywhere, not only in the source note. Bare TeX
  (`QW_i^Q`) and a Unicode stand-in (`√d_k`, `QKᵀ`) stay text on the page; only
  `$…$` is typeset.
- Language: everything written for the reader — the hub's sections, every
  chapter heading, the explanations, a lab block's caption, the cards, the
  retelling check — is in the user's language. The section list below gives each
  heading's meaning, not its text: a Chinese reader's chapter reads `## 一句话`,
  `## 原文`, `## 记号`, `## 试一试`, `## 失效条件`, `## 问题`, `## 我的话`, and the
  hub `## 已知`, `## 待学`, `## 章节`; an English reader's, "In one sentence",
  "From the paper", "Notation", "Try it", "Where it stops holding", "Question",
  "In my words". What is machine-read stays as written here: the `#chapter` tag,
  the `status::` values (unread, reading, retold, done), the folder and file
  names, the block ids. The source note stays in the paper's own language: it is
  a quotation. Chinese prose takes Chinese punctuation — ，。；：？！（）「」 —
  and a variable named in a sentence is a formula (`$n$`, `$d_k$`, `$O(n/r)$`);
  ASCII punctuation belongs to code, formulas and links alone.
- A chapter note — a `# title` line, then `#chapter [status:: unread]` on the
  next line (the hub reads it), then these sections in this order, every time,
  headed in the user's language:
  1. *in one sentence* — the idea, plainly.
  2. *from the paper* — the paragraphs it rests on, embedded:
     `![[papers/<slug>/source#^s3p2]]`. Never a chapter without one.
  3. *notation* — only when the chapter introduces symbols: a two-column table,
     symbol and meaning, in the paper's own notation.
  4. *try it* — a lab block, see below, in the chapters that carry one; the
     other chapters leave this section out (never an empty heading).
  5. *where it stops holding* — the condition the idea depends on, in one or two
     sentences.
  6. *question* — one question the reader should be able to answer after the
     chapter, never yes/no.
  7. *in my words* — left empty: the reader fills it.
  Under 350 words of explanation per chapter; the source blocks carry the rest.
- The reader's confusions are tasks: `- [ ] why divide by √d_k #question` in a
  chapter note. When asked to answer them (or when one is in the working note),
  answer under the task as an indented line (edit_note), in the chapter's
  notation, citing the source block, then tick it with update_task.
- Checking the retelling: read the chapter's last section (*in my words*) and the
  source blocks it embeds (read_block each), then say what the retelling got right, what
  it left out (a condition, a step), what it turned around (a cause for an
  effect), and which symbol it confused — each point citing the block. The cards
  come from these gaps alone, one per gap, in the tutor's shape; a retelling with
  no gap gets no card.
- A lab block: a ` ```js ` fence calling `vemry.lab.*`. Which chapters get one
  follows the goal: understand — two or three in the first session, in the
  chapters that carry the core mechanism; reproduce — every chapter whose idea
  is numeric; survey, interview, review — none unless asked. The test for a
  chapter is that its idea is itself a computation the reader can watch: a
  formula with a parameter to move, an algorithm run on three tokens. A problem
  statement, a results table, a limits chapter has nothing to compute and gets
  none. The reader sees a card, never the code, so the block opens with one
  `//` line saying what the experiment shows (the card's caption). The
  parameters the reader will want to move are the controls — at most three —
  and the block draws at most two views; it computes the example on a tiny
  instance (three tokens, a 2×2, ten points) so the numbers can be read, and
  every view has a title that says what it shows. Nobody runs the block before
  the reader clicks Run, so it stays small and plain enough to be right the
  first time. The API:
  - controls: `const p = vemry.lab.controls({ name: { slider: [min, max], step, value, label }, flag: { toggle: true }, pick: { select: ["a", "b"] } })`
    returns `{ name, flag, pick }`; `const Q = vemry.lab.matrix("Q", [[1, 0], [0, 1]])`
    returns the current grid.
  - views: `vemry.lab.heatmap(title, rows, { rows, cols, digits, diverging })`,
    `vemry.lab.line(title, [{ name, y }], { x, xLabel, yLabel })`,
    `vemry.lab.bars(title, { label: value }, { yLabel, horizontal })`,
    `vemry.lab.scatter(title, [[x, y]], { xLabel, yLabel })`,
    `vemry.lab.table(title, [{ col: value }], { highlight: { row } })`,
    `vemry.lab.step(title, [{ label, text, value }])` — text may hold `$…$`.
  - Plain JavaScript inside (no imports, no network); `console.log` shows under the
    card. The reader clicks Run; the block re-runs as the controls move. Never
    describe the result in the note — the reader sees it.
- Output by goal, when the reading is done: reproduce → the brief skill's
  implementation brief; survey → one `> [!evidence]` per finding with
  `supports:[[…]]` to the claim it bears on; interview → the cards; review → the
  paper's claims as `> [!claim]` blocks, its evidence as `> [!evidence]`, and a
  threats-to-validity list; understand → nothing beyond the chapters and cards.

## Format

What the page can and cannot render — checked against before every write:

- A formula is `$…$` (a variable in a sentence too: `$n$`, `$d_k$`, `$W_i^Q$`,
  `$O(n/r)$`); `$$…$$` on a line of its own for a display formula, with an
  equation number, if any, after the closer. Bare TeX (`d_model`, `QW_i^Q`) and
  Unicode stand-ins (`√d_k`, `QKᵀ`) stay text, and an underscore outside `$…$`
  reads as emphasis. A multi-letter subscript takes braces: `$d_{model}$`.
- Chinese prose takes Chinese punctuation (，。；：？！（）「」); ASCII marks belong to
  code, formulas and links.
- A table's rows all have the header's number of cells; a cell holds one line.
- A list is tight: no blank line between its items.
- A section that has nothing to say is left out, never left as an empty heading
  (*in my words* is the exception — the reader fills it).
- Headings in the reader's language; the `#chapter` line, the `status::` values,
  the ids and the file names as this skill spells them.
- A figure is an image line with its id (`![Figure 1: …](url) ^fig1`), never a
  caption alone; a chapter embeds a figure or a paragraph by that id.

## Landing

`papers/<slug>/` (slug = first author and year, `vaswani-2017`; the user's folder
when named): the hub `<Title>.md`, `source.md`, `chapters/<nn> <title>.md`, and
`cards.md`. One write_note per note; the source note section by section with
append_note. A study of the same paper later updates these notes in place
(search_notes first; never a second folder for the same paper).

## Shape

The reply after the first session: the chapter list with the order chosen and why,
which items were already known (with their links), which chapters carry an
experiment, and the one thing to read first. After a retelling check: the three lists (right, missing, turned around),
each item with its citation, then how many cards were made.

## Never

- A chapter without an embedded source block, or a source note that paraphrases.
- Cards made from the paper instead of from the reader's gaps.
- A lab block on a chapter with nothing to compute, or one whose result is
  narrated in the note.
- Reading the whole paper into the context at once: section by section, written
  down as it goes.
- A retelling check that grades without citing the block it checked against.
