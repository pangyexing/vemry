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
  agent_session: <id>
---
```

Unknown fields survive round-trips verbatim (flattened through `Frontmatter.extra`),
so vemry never eats metadata that belongs to you or another tool.

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

Parsing (`scan_inline`): `[[…]]` does not span lines, must be non-empty, and is not
parsed inside inline code spans.

Resolution (`BlockIndex::resolve_target`): an empty target means this note; a target
containing `/` matches the full relative path; a bare name matches a filename stem
or an entry in `aliases`, **case-insensitively**, with the shortest path winning a tie.

**Fallback**: renders as the literal text `[[Target]]` — readable, but not a link.
**This is the weakest fallback in the dialect**: the semantics (link graph,
backlinks) are lost in other tools, though no text is.

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

## Callouts, claims and evidence

```markdown
> [!warning] Optional title
> Following lines that start with > join the same callout

> [!claim] Sleep affects memory consolidation ^c1

> [!evidence] Walker 2017, experimental data
> supports:[[Sleep research#^c1]], [[Another note#^c9]]
```

- `[!type]` is case-insensitive and may not contain whitespace; a `+`/`-` fold
  marker may follow
- Recognised types: `note`/`info`/`todo`/`abstract`/`summary`/`tldr`,
  `tip`/`hint`/`important`, `success`/`check`/`done`, `question`/`help`/`faq`,
  `warning`/`caution`/`attention`,
  `danger`/`error`/`failure`/`fail`/`missing`/`bug`, `example`, `quote`/`cite`,
  plus vemry's own `claim` and `evidence`
- Only `claim` and `evidence` become their own block types in the index; the rest
  are indexed as paragraphs
- `evidence` declares what it supports with `supports:`. **If `supports:` is
  present, only the links after it count**; if it is absent, every link in the
  block counts.

**Fallback**: a standard blockquote, with `[!warning]` visible as the first line of
text. This is the cleanest fallback in the dialect.

## Block IDs

A trailing ` ^id` (space, `^`, identifier) gives a block a stable anchor for
`[[Note#^id]]` to point at. Blocks without an explicit id get a positional
temporary id from the index, which is **never written back to disk**.

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

A YAML body with **seven flat scalars**, all optional:

| Field | Values |
|---|---|
| `kind` | `paragraph` / `heading` / `task` / `claim` / `evidence` / `code_block` |
| `note` | substring of a note path or name |
| `tag` | tag name (a leading `#` is stripped) |
| `text` | substring of the body |
| `task_state` | `todo` / `doing` / `done` |
| `due_before` | `YYYY-MM-DD` or `today` (evaluated in local time) |
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

Three independent entry points; matching any one puts a path in a privacy zone
(never read, never written, never in a prompt):

- `private: true` in a note's frontmatter
- a glob rule in `.vemry/privacy` (gitignore syntax, case-insensitive)
- any path segment beginning with `.` (built in, needs no configuration)

**Fallback**: `private: true` is an ordinary frontmatter key and `.vemry/privacy` is
an ordinary text file. Privacy isolation is vemry runtime behaviour; it does not
change the files.

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
