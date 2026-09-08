---
name: study
description: Study a paper or a long text — import it, read it in chapters, check the retelling, cards from the gaps
triggers: [study, read this paper, work through, deep read, add an experiment, check my retelling, 精读, 研读, 学这篇, 读这篇论文, 带我读, 加实验, 可加的实验, 检查复述]
version: 2
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
The plan is proposed before anything is designed: the chapter and its line in
the hub are all the list needs, and the experiment is worked out after the
reader has said yes. Mark each step done with update_plan as it lands — the
card is the reader's progress bar, not a receipt at the end.

## Inputs

- The source, in this order: a vault file (read_file — a PDF comes back as text),
  a clipped page (clips_lookup, then read_note), a URL — import_page, one call:
  the app fetches the page and writes the source note itself (sections, ids,
  equations, figures, tables), and returns the outline of ids. An arXiv link
  means its HTML version alone, `https://arxiv.org/html/<id>`, which keeps the
  section structure — the abstract page adds nothing and is not requested. Then
  read the note with read_note; a long one comes in parts — the result says
  `part 1 of N` — and the next is one call away (`part: 2`, then 3). Write the
  chapters a part supports before reading the next: the source note is the
  memory, not the context. Never look for the rest of a page anywhere else (the
  PDF, a mirror, the browser): it is not there, and every such attempt costs a
  minute.
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

- The source note is import_page's: the paper's sections as headings, one block
  per paragraph ending in its id — `^s<section digits>p<n>`: `^s3p2` is section
  3 paragraph 2, `^s321p4` is section 3.2.1 paragraph 4, `^s0p1` the abstract's
  first — each equation as `$$…$$` with its number, the tables, each figure with
  its caption as one block (`^fig1`, `^fig2`: a chapter embeds the diagram by
  that id — an architecture diagram is often the one thing a reader remembers),
  each table's caption as `^tab1`; the reference list and the appendix behind it
  are not archived (a chapter cites a reference by its number if it must).
  Quotation, not paraphrase: this note is what the chapters cite, and it is not
  rewritten — not reformatted, not "cleaned", not renumbered (chapters and cards
  cite the ids). Pass import_page the frontmatter you know (`title`, `authors`,
  `year`, `genre`); set what you learn later with set_frontmatter. A source that
  is a vault file or a clip is written with write_note in this same shape.
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
  After it, `## 可加的实验` when any chapter's experiment was listed rather than
  written (see the lab block below): one line per experiment — the chapter as a
  [[link]], what to move, what to watch — so the reader picks the next one.
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
  a quotation. The Format section below says how formulas and punctuation are
  written.
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
- A lab block: a ` ```js ` fence calling `vemry.lab.*`. Whether a chapter has
  one is decided by its idea alone: an experiment where the idea is itself a
  computation the reader can watch — a formula with a parameter to move, an
  algorithm run on three tokens — and none where there is nothing to compute (a
  problem statement, a results table, a limits chapter). No number decides it.
  What the first session writes is decided by budget: the experiments of the
  chapters that carry the core mechanism — the ones that make this paper this
  paper — are written now; every other chapter that qualifies gets its
  experiment listed instead, under `## 可加的实验` in the hub (heading in the
  user's language), one line each: the chapter, what to move, what to watch. A
  later "add the … experiment" turns one line into a block in its chapter's
  *try it* section and removes the line. Survey, interview and review goals
  list, never write, unless asked; reproduce writes them all.
  The reader sees a card, never the code, so the block opens with one `//` line
  — the card's caption — that says what to move and what to watch ("drag d_k;
  the unscaled weights collapse to one-hot"). The parameters the reader will
  want to move are the controls — at most three, plus one row of presets: two
  to four named settings that each set several controls at once ("all keys
  equal", "hide the future"), the guided experiments of the card. The block
  draws at most two views; it computes the example on a tiny instance (three
  tokens, a 2×2, ten points) so the numbers can be read, and every view has a
  title that says what it shows. Nobody runs the block before the reader clicks
  Run, so it stays small and plain enough to be right the first time. Design it
  once: the hub's line (or the chapter's idea) already says what to move and
  what to watch; take the plainest computation that shows it on the tiny
  instance and write the block in one pass — no weighing of alternative
  formulations, no study of numerical corner cases; when the plain computation
  misbehaves at an edge, shrink the instance rather than change the method. The API:
  - controls: `const p = vemry.lab.controls({ name: { slider: [min, max], step, value, label }, flag: { toggle: true }, pick: { select: ["a", "b"] }, try: { presets: { "all keys equal": { name: 1, flag: false } } } })`
    returns `{ name, flag, pick, try }` (`try` is the preset last pressed, else
    null); `const Q = vemry.lab.matrix("Q", [[1, 0], [0, 1]])` returns the
    current grid.
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
- A figure is an image with its caption in one block, `^fig1` (import_page's
  shape), never a caption alone; a chapter embeds a figure or a paragraph by
  that id.

## Landing

`papers/<slug>/` (slug = first author and year, `vaswani-2017`; the user's folder
when named): the hub `<Title>.md`, `source.md`, `chapters/<nn> <title>.md`, and
`cards.md`. One write_note per note; the source note by import_page. A study of
the same paper later updates these notes in place — edit_note and append_note on
the chapters and cards, never a write_note that retypes one — (search_notes
first; never a second folder for the same paper).

## Shape

The reply after the first session: the chapter list with the order chosen and why,
which items were already known (with their links), which chapters carry an
experiment, and the one thing to read first. After a retelling check: the three lists (right, missing, turned around),
each item with its citation, then how many cards were made.

## Never

- A chapter without an embedded source block, or a source note that paraphrases.
- Cards made from the paper instead of from the reader's gaps.
- A lab block on a chapter with nothing to compute, or one whose result is
  narrated in the note; a caption that says what the card shows instead of what
  to move and what to watch.
- Reading the whole paper into the context at once: part by part, the chapters
  of a part written before the next is read.
- A retelling check that grades without citing the block it checked against.
