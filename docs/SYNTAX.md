# The vemry Markdown dialect

The single source of truth for a vault is the plain Markdown files on disk. This
document defines every construct vemry recognises **on top of CommonMark + GFM**,
and what each one degrades to **in a standard Markdown renderer that has never
heard of vemry**.

It is also the review criterion for the "no lock-in" principle in
[CONTRIBUTING.md](../CONTRIBUTING.md): new syntax lands here first, with its
fallback stated — see [Adding to the dialect](#adding-to-the-dialect) at the end.

Implementation: `crates/vemry-core/src/parser.rs` (a line-and-inline scanner,
deliberately not a full Markdown AST), `crates/vemry-core/src/note.rs`
(frontmatter), `src/lib/editor/` (rendering).

> 中文版：[SYNTAX.zh-CN.md](SYNTAX.zh-CN.md)

## Baseline

CommonMark + GFM (tables, strikethrough, task lists) + YAML frontmatter at the top
of the file. Anything not listed below, vemry treats as ordinary Markdown and does
not interpret further.

## Frontmatter

```yaml
---
title: Optional title
tags: [research, sleep]      # a single string is also accepted
aliases: [Alias one, Alias two]   # participate in bare-name wikilink resolution
created: 2026-08-20
private: true                # a privacy zone: permanently invisible to the AI
vemry:                       # provenance vemry writes itself, kept apart from your fields
  origin: agent              # agent | capture
  agent_session: <id>       # the research session that created the note (.vemry/sessions/<id>.jsonl)
---
```

Unknown fields survive round-trips verbatim (flattened through `Frontmatter.extra`),
so vemry never eats metadata that belongs to you or another tool. Block-level
provenance (who wrote which block) is not syntax: it lives in
`.vemry/provenance.jsonl`, and the body carries no marker.

**Fallback**: standard YAML frontmatter, which most Markdown tooling recognises
natively. A renderer that does not shows it as a `---`-fenced block of text at the
top; the body is untouched.

## Links and embeds

| Syntax | Meaning |
|---|---|
| `[[Target]]` | link to a note |
| `[[Target\|Alias]]` | display the alias |
| `[[Note#Section]]` | link to a heading |
| `[[Note#^blockId]]` | link to a block |
| `[[#Section]]` / `[[#^blockId]]` | empty target = this note |
| `![[image.png]]` | embed an image inline |
| `![[image.png\|300]]` | embed at a given width in px |
| `![[Note]]` | embed a whole note (its body, read-only) |
| `![[Note#Section]]` | embed a section |
| `![[Note#^blockId]]` | embed one block (a list item brings its sub-items) |

Parsing (`scan_inline`): `[[…]]` does not span lines, must be non-empty, and is not
parsed inside inline code spans.

Resolution (`BlockIndex::resolve_target`): an empty target means this note; a target
containing `/` matches the full relative path; a bare name matches a filename stem
or an entry in `aliases`, **case-insensitively**, with the shortest path winning a tie.

**Fallback**: renders as the literal text `[[Target]]` — readable, but not a link.
**This is the weakest fallback in the dialect**: the semantics (link graph,
backlinks) are lost in other tools, though no text is.

**Embeds** (`![[…]]` with a non-image target) show the target's current text
inside the editor — a live, read-only projection, nothing copied into the file.
Alone on its line the embed is a block with a header that jumps to the source;
inside a line it is a chip. An embed expands exactly one level: `![[…]]` inside
embedded content shows as a ⧉ link, so an embed can never loop, and an embed of
the note itself (or of the section it sits in) shows a marker. A target that
does not resolve keeps the literal text and shows "not found". The agent sees
the literal `![[…]]` of the note it reads, never the embedded content (see
"Privacy markers" for why that matters). Fallback: the literal text.

## Tags

Inline `#tag`, merged and deduplicated with frontmatter `tags`.

- Allowed characters: letters, digits, `_`, `-`, `/` (so `#research/sleep` nests)
- The preceding character must not be a letter, digit, `#` or `&` (which keeps
  entities like `&#8203;` out)
- An all-digits tag is not a tag (`#1` is ordinary text)
- Not parsed inside inline code spans

**Fallback**: ordinary text. A `#tag` at the start of a line is not mistaken for a
heading — ATX headings require a space after the `#`, and vemry agrees with
CommonMark on that.

## Tasks

```markdown
- [ ] To do
- [/] In progress
- [x] Done
- [ ] With dates 📅 2026-08-20 ⏳ 2026-08-15
- [ ] The equivalent in ASCII due::2026-08-20 scheduled::2026-08-15
```

- Any of `-`, `*`, `+` as the bullet, at any indentation
- The `]` in `[?]` must be followed by a space or the end of the line
- `x` and `X` both mean done; an unknown state character is treated as to-do
- Dates are always `YYYY-MM-DD`; due is `📅` or `due::`, scheduled is `⏳` or
  `scheduled::`

**Fallback**: `[ ]` and `[x]` are standard GFM task list items and render as
checkboxes. `[/]` is not a GFM marker (GFM accepts only a space or `x`/`X`), so it
degrades to an ordinary list item with the literal text `[/]` — still readable.
Dates are inline plain text.

## Lists

```markdown
- one item
  a continuation line, indented deeper than the marker
  - a nested item is its own block
1. ordered items too
2) with either `.` or `)`
```

Every list item is a block of its own — the marker line plus the following
lines indented deeper than the marker. A new marker at any depth starts the
next block, so a nested item is a separate block and **an id on an item covers
that item only, not its children** (a reference or embed of the item does
include them). A blank line, a heading, a fence, a callout or a checkbox item
ends the item. A lazy continuation line (not indented) is not recognised: it
becomes a paragraph. `- - -` and `* * *` are thematic breaks, not items.

- Markers: `-`, `*`, `+`, or one to nine digits followed by `.` or `)`, then a
  space; any indentation (a tab counts as one character of indent)
- The index records the indent and whether the list is ordered
- Checkbox items are [tasks](#tasks), not list items; `1. [ ] x` is a list item
- Tables, plain quotes and thematic breaks stay paragraphs

**Fallback**: standard Markdown lists. This is indexing granularity, not
syntax — nothing is written differently.

## Inline fields

```markdown
author:: Ursula K. Le Guin
- Read it again [rating:: 5] [status:: done]
- [ ] Write the spec due::2026-08-10 #work
> status:: reading
```

A field is `key:: value`. The key is letters, digits, `_` and `-` (Chinese
keys work: `作者:: 勒古恩`) and is not all digits; the `::` must be followed by
whitespace, a digit or the end of the line — so `std::vector`, `C++::foo`,
`12::30` and `http://x` are never fields. Three forms:

| Form | Where | Value |
|---|---|---|
| `key:: value` as the whole line | after the block prefix (indent, `>`, list marker, checkbox) | the rest of the line, minus a trailing ` ^id` |
| `[key:: value]` / `(key:: value)` | anywhere in a line, not nested, not across lines | up to the matching bracket |
| `key:: token` inside a line | after whitespace | one token, up to the next whitespace |

- Every block keeps its fields (key, value as written, position); values are
  interpreted by whoever reads them — a date, a number, a `[[link]]`, a list
  `a, b` — not by the parser
- A task's `due::` / `scheduled::` are ordinary fields as well as task dates;
  the `📅` / `⏳` spellings are task dates only
- Not scanned inside inline code, `[[wikilinks]]`, fenced code or frontmatter
- Frontmatter scalars (and `created`) are the note's fields: a field filter
  falls back to them when a block has no field of that key (`title`, `tags`,
  `aliases`, `private` and `vemry` are metadata, not fields)

**Fallback**: `key:: value` is readable plain text; `[key:: value]` shows as a
bracketed span (it is not a link, having no `(url)`); `(key:: value)` is a
parenthesis. Nothing is swallowed and no list is broken.

## Callouts, claims and evidence

```markdown
> [!warning] Optional title
> Following lines that start with > join the same callout

> [!claim] Sleep affects memory consolidation ^c1

> [!evidence] Walker 2017, experimental data
> supports:[[Sleep research#^c1]], [[Another note#^c9]]
```

- `[!type]` is case-insensitive and may not contain whitespace; a `+`/`-` fold
  marker may follow — in the editor `-` opens the callout folded and `+` or no
  marker opens it expanded; the fold state is never written to the file
- Recognised types: `note`/`info`/`todo`/`abstract`/`summary`/`tldr`,
  `tip`/`hint`/`important`, `success`/`check`/`done`, `question`/`help`/`faq`,
  `warning`/`caution`/`attention`,
  `danger`/`error`/`failure`/`fail`/`missing`/`bug`, `example`, `quote`/`cite`,
  plus vemry's own `claim` and `evidence`
- `claim`, `evidence` and `private` become their own block types in the index;
  the rest are indexed as paragraphs
- `private` is the block-level privacy zone (see [Privacy markers](#privacy-markers)):
  it must start its line (after indentation), so inside a list item it goes on a
  continuation line of its own; a lazy line without `>` is outside the block
- `evidence` declares what it supports with `supports:`. **If `supports:` is
  present, only the links after it count**; if it is absent, every link in the
  block counts.

**Fallback**: a standard blockquote, with `[!warning]` visible as the first line of
text. This is the cleanest fallback in the dialect.

## Block IDs

A trailing ` ^id` (space, `^`, identifier) gives a block a stable anchor for
`[[Note#^id]]` to point at. Blocks without an explicit id get a positional
temporary id from the index, which is **never written back to disk**.

- Identifier characters: `A–Z`, `a–z`, `0–9`, `-` (no `_`, no spaces, must run
  to the end of the line)
- It sits on the block's last line: the end of a paragraph or list item, after
  a heading's title (`# Title ^h1` — the id is not part of the title, so
  `[[Note#Title]]` still resolves), the last `>` line of a callout. A table row
  or a fenced code block cannot carry one (it would break the construct).
- **Minting**: vemry writes an id only when you reference, embed or copy a
  link to a block that has none, and only for that block — six random
  characters from `a–z0–9`, unique within the note. Nothing is written
  proactively; positional ids never reach disk.
- An id on a list item names that item alone; the item's children are not
  part of the id's block, though a reference to the item shows them.

**Transient positional citations.** An AI answer may cite a block that has no
id yet as `[[path#bN]]`, where `bN` is the index's positional id from the tool
result. This form is **not part of the dialect**: it is valid only inside that
conversation, and vemry never writes it to a note. When an answer is copied,
inserted, pinned or saved as a note, every `[[path#bN]]` is materialized — the
cited block receives a minted ` ^id` and the link becomes `[[path#^id]]`; a
citation that cannot be pinned (the block is gone) degrades to `[[path]]`.

**Fallback**: ordinary text at the end of the line.

## Inline highlight

`==highlighted text==`. Neither CommonMark nor GFM has this; it is implemented as a
paired-delimiter extension modelled on `@lezer/markdown`'s own strikethrough.

**Fallback**: the literal `==highlighted text==`, which is readable.

## Query blocks

````markdown
```vemry-query
kind: task
task_state: todo
due_before: today
limit: 20
```
````

A YAML body with **eight flat scalars**, all optional (`field` may repeat):

| Field | Values |
|---|---|
| `kind` | `paragraph` / `heading` / `list_item` / `task` / `claim` / `evidence` / `code_block` / `private` (the last is for query blocks, views and scripts only: the agent never sees a private block) |
| `note` | the exact vault-relative path of one note, e.g. `projects/Alpha.md` |
| `tag` | tag name (a leading `#` is stripped) |
| `text` | substring of the body |
| `task_state` | `todo` / `doing` / `done` |
| `due_before` | `YYYY-MM-DD` or `today` (evaluated in local time) |
| `field` | `key` (present) / `key = v` / `key != v` / `key < v` / `key > v` / `key ~ text`; dates and numbers compare as such; several `field` lines all have to hold |
| `limit` | positive integer |

No nesting, no expressions — deliberately a safe subset: a query block is read-only,
cannot write to disk, and cannot reach the network.

**Fallback**: **an ordinary code block** showing the query as written. That is the
intent — results are derived data and are never written to disk, so what an outside
tool sees is exactly what is on disk.

## Runnable code blocks

A ` ```js ` / ` ```javascript ` block (the language tag is case-insensitive) gets a
Run button in the editor. Pressing it sends the block into a one-shot read-only
sandbox on the Rust side (2-second fuse, no write API, no network). Output renders
after the closing fence, lives only in editor state, is **never written to disk**,
and disappears when the document closes.

**Never runs by itself**: opening a note is not running it. That button is the only
entry point in the whole chain, and the AI has no tool that can run code either.

## Privacy markers

Four independent entry points. The first three put a whole path in a privacy zone
(never read, never written, never in a prompt); the fourth does the same for a
single block:

- `private: true` in a note's frontmatter
- a glob rule in `.vemry/privacy` (gitignore syntax, case-insensitive)
- any path segment beginning with `.` (built in, needs no configuration)
- a `> [!private]` callout — the block is removed from everything the AI reads
  (the note's text, search, block queries, references, attached selections) with
  no placeholder, and the note it lives in can no longer be overwritten whole by
  the AI (only edited). Its `#tags`, `[[links]]` and `key:: value` fields are not
  indexed for anyone, so they cannot leak through tag counts, backlinks or the
  graph.

**Fallback**: `private: true` is an ordinary frontmatter key and `.vemry/privacy` is
an ordinary text file. A `> [!private]` block is an ordinary blockquote whose
first line reads `[!private]` — **another tool shows its content**. Privacy
isolation is vemry runtime behaviour; it does not change the files.

## Adding to the dialect

Before anything joins this dialect, all three must hold:

1. **It is written down here**, together with its fallback;
2. **It degrades gracefully in a standard Markdown renderer** — becoming readable
   plain text or a standard structure is fine, but it may not make content vanish
   or derail the structures around it (swallowing a paragraph, breaking a list,
   confusing fence nesting);
3. **It does not need a sidecar file to mean what it means** — copy the vault
   directory somewhere else, open it in an ordinary editor, and what you wrote must
   still be there.

The criterion is falsifiable: run the file through a plain CommonMark renderer and
compare against the fallbacks stated above.
