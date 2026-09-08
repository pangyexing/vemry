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

## Inputs

- The source, in this order: a vault file (read_file — a PDF comes back as text),
  a clipped page (clips_lookup, then read_note), a URL (open_for_reading; for an
  arXiv abstract page ask for its HTML version, `https://arxiv.org/html/<id>`,
  which keeps the section structure). Read a long paper section by section and
  write each section down before reading the next: the source note is the memory,
  not the context.
- What the vault already holds: search_notes with the paper's key terms and
  query_blocks with kind: claim — the known map is built from hits, and a chapter
  the vault already explains is linked, not rewritten.
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
  block ending in an id (`^s3p2` = section 3, paragraph 2), formulas as `$…$` and
  `$$…$$`, tables as tables, a figure as one line naming it and its caption.
  Quotation, not paraphrase: this note is what the chapters cite. Frontmatter
  `title`, `authors`, `year`, `source` (the URL or the file), `genre`.
- The hub: frontmatter `goal` and `status`; a known list (each item a [[link]] to
  the note that already explains it), a to-learn list, the chapter order for this
  goal (a chapter already known is marked so and skipped), and a query block
  with `tag: chapter` and `text: "#chapter"` so the chapters and their status
  show live, one row per chapter (a tag alone matches every block of a tagged
  note; the text filter keeps the `#chapter` line itself).
- A chapter note, in this order, every time:
  1. `#chapter [status:: unread]` on its first line — the hub reads it.
  2. `## In one sentence` — the idea, plainly.
  3. `## From the paper` — the paragraphs it rests on, embedded:
     `![[papers/<slug>/source#^s3p2]]`. Never a chapter without one.
  4. `## Notation` — only when the chapter introduces symbols: a two-column table,
     symbol and meaning, in the paper's own notation.
  5. `## Try it` — only when the idea is numeric and the reader asked for an
     experiment (or the goal is reproduce): a lab block, see below.
  6. `## Where it stops holding` — the condition the idea depends on, in one or
     two sentences.
  7. `## Question` — one question the reader should be able to answer after the
     chapter, never yes/no. Then `## In my words`, left empty: the reader fills it.
  Under 350 words of explanation per chapter; the source blocks carry the rest.
- The reader's confusions are tasks: `- [ ] why divide by √d_k #question` in a
  chapter note. When asked to answer them (or when one is in the working note),
  answer under the task as an indented line (edit_note), in the chapter's
  notation, citing the source block, then tick it with update_task.
- Checking the retelling: read the chapter's `## In my words` and the source
  blocks it embeds (read_block each), then say what the retelling got right, what
  it left out (a condition, a step), what it turned around (a cause for an
  effect), and which symbol it confused — each point citing the block. The cards
  come from these gaps alone, one per gap, in the tutor's shape; a retelling with
  no gap gets no card.
- A lab block: a ` ```js ` fence calling `vemry.lab.*`, written only when the
  reader asks for an experiment or a chapter's `## Try it` calls for one. The
  reader sees a card, never the code, so the block opens with one `//` line
  saying what the experiment shows (the card's caption). The parameters the
  reader will want to move are the controls; the block computes the example on
  a tiny instance (three tokens, a 2×2, ten points) so the numbers can be read;
  every view has a title that says what it shows. The API:
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

## Landing

`papers/<slug>/` (slug = first author and year, `vaswani-2017`; the user's folder
when named): the hub `<Title>.md`, `source.md`, `chapters/<nn> <title>.md`, and
`cards.md`. One write_note per note; the source note section by section with
append_note. A study of the same paper later updates these notes in place
(search_notes first; never a second folder for the same paper).

## Shape

The reply after the first session: the chapter list with the order chosen and why,
which items were already known (with their links), and the one thing to read
first. After a retelling check: the three lists (right, missing, turned around),
each item with its citation, then how many cards were made.

## Never

- A chapter without an embedded source block, or a source note that paraphrases.
- Cards made from the paper instead of from the reader's gaps.
- A lab block nobody asked for, or one whose result is narrated in the note.
- Reading the whole paper into the context at once: section by section, written
  down as it goes.
- A retelling check that grades without citing the block it checked against.
