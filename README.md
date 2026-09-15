<div align="center">

# vemry

**Understand a paper, and keep what you learned.**

A local-first AI study workbench. Papers, pages and PDFs come in as plain Markdown
on your own disk, and the AI that works in that folder sees only what you allow.

[**Download**](https://github.com/pangyexing/vemry/releases) · [Get started](#get-started) · [Docs](#learn-more) · [Discussions](https://github.com/pangyexing/vemry/discussions)

[![Release](https://img.shields.io/github/v/release/pangyexing/vemry?include_prereleases&sort=semver)](https://github.com/pangyexing/vemry/releases)
[![CI](https://github.com/pangyexing/vemry/actions/workflows/ci.yml/badge.svg)](https://github.com/pangyexing/vemry/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-FSL--1.1--ALv2-blue)](#license)
![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)

[English](README.md) · [简体中文](README.zh-CN.md)

</div>

![vemry — a chapter of a paper under study, with the source embedded block by block](docs/assets/hero.png)

<br>

**Your notes are files.** Plain Markdown in a folder you chose. No database, no
account, no sync service. Open the folder in any editor and you see exactly what
vemry sees.

**The AI asks before it writes.** Every edit stops at a diff you allow, deny, or
edit and then apply. What you mark private, the model never sees.

**It works offline.** Nothing reaches the network unless you did something that
needs it. No telemetry, no account, and without an API key it is still a complete
local editor.

## What vemry is

vemry is an editor for a folder of Markdown, with an AI that works inside that
folder under your rules. Everything you see — the editor, every view, the AI's tools,
your own scripts — reads the same block-level index built from your files. There is
no second copy of your knowledge anywhere.

It is not a cloud service, and it is not a chat window with a notes folder
attached. The AI is one participant in your vault, with a permission gate in front
of every write and a privacy boundary it cannot see across.

**Fuss less. Understand more. Remember longer.** Finish a paper and explain it in
your own words. Where you drift, you see it on the spot. When it starts to fade, a
card brings it back. The tedious part, clipping to Markdown, translating side by
side, filing, tracing sources, making the cards, you never do by hand, and every
write still waits at a diff for your yes.

## What you can do

### Read a paper chapter by chapter

Paste an arXiv link, drop in a PDF or point at a clipped page and pick *Study a
paper*. The source arrives as a note you can move around in — every section,
equation, figure and table its own block — and the reading unfolds one chapter at a
time. Each chapter starts from what your vault already knew about the idea, marks
what is new, and links every sentence to the passage it came from.

### Find out whether you actually got it

Explain a chapter back in your own words and see exactly where your account drifts
from the paper. Want to try the idea? Ask for an experiment and a runnable block
appears in the chapter. What you did not fully get becomes flashcards, and the
Review view brings them back just before you would have forgotten.

### Read in two languages at once

Open a note, a web page or a PDF beside its translation, aligned paragraph by
paragraph. On a PDF, a translated paragraph lights up the passage it came from on
the page.

### Clip the web without leaving

A built-in browser opens next to your notes. Clip a page or a selection and it
lands in a note with its source and its images saved into your vault; ad blocking
is built in. A thought that does not belong anywhere yet goes to an inbox note
with one palette command.

### One vault, seen by the question you are asking

What is due today: Agenda. Where each thing stands: Board. What happened this
month: Timeline. How your notes connect: Graph. Which claims have evidence behind
them and which do not: Claims and Evidence. Which cards are due: Review. None of
them stores anything of its own; each is a query over the one index, and a
`vemry-query` fence inside a note renders the same query in place.

![Board view — every task in the vault, by state](docs/assets/kanban.png)

![Graph — the same index, drawn as links; the two private notes are visible to you and not to the model](docs/assets/graph.png)

### Ask, and check the answer

Every answer cites the block it read, and the citation opens it. With web search on,
each source is marked verified or not, and an answer you want to keep goes into a
note as a block. Press `@` to hand the AI a note, a folder or the current selection.

### Hand over the chores, on your terms

Fifteen built-in skills cover the work around reading: tidy a folder, restructure a
note, turn meeting notes into actions, draft the weekly report from the week's notes
for you to finish, close the day. The chores go to the AI; the reading, the
retelling and the decisions stay with you. Add your own as a Markdown file in
`.vemry/skills/`. Every write is a diff, a whole
turn's writes can be undone, and every session is an audit log you can replay in
the Session Replay view.

![The write gate — the agent proposed this edit; nothing reaches disk until you answer](docs/assets/permission-diff.png)

### Automate with a script, not a plugin

Drop a `.js` file into `.vemry/scripts/` and it is live on save. Scripts register
palette commands, subscribe to vault events and run on a cron, inside a sandbox
with no filesystem, no network and no processes. Every capability in the app, yours
included, is in one searchable palette (⌘P).

## Get started

1. **Install.** Download the `.dmg`, `.exe`, `.AppImage`, `.deb` or `.rpm` from
   [Releases](https://github.com/pangyexing/vemry/releases), or
   [build from source](#build-from-source). On macOS the first launch needs a
   right-click → Open, because the app is not yet notarised.
2. **Open the example vault.** Clone this repo, then ⌘P → *Open another vault…* →
   `examples/demo-vault`. It is a small real vault with one paper worked through
   end to end.
3. **Add an AI key** (optional). Settings → API key. Any OpenAI-compatible
   endpoint works, including a local Ollama; the mainland default is Zhipu, whose
   `glm-4.7-flash` is free and supports tool calls. Or switch the provider to *vemry
   hosted* and sign in with your e-mail address: the models come with the
   account, and a request goes through vemry's relay to the model vendor
   unchanged.
4. **Study a paper.** Open the AI panel, pick *Study a paper* and paste an arXiv
   link, or drop in a PDF. The source note, the chapters and the cards arrive as
   diffs you approve.

That is the whole loop. Everything it made is a Markdown file in your folder.

On Windows and Linux, read ⌘ as Ctrl.

### First things to try

Open the AI panel (⌘J from anywhere) and type one of these:

```
Study this: https://arxiv.org/abs/1706.03762
Here is my retelling of chapter 3: … Where does it drift from the paper?
Give me an experiment for section 3.2 that I can run here.
Quiz me on the cards that are due today.
What changed in this vault this week?
Tidy the research/ folder.
```

Then mark `private/` as a privacy zone and ask the AI to list your notes. It will
not see the folder, and it will not know that anything is missing.

## What stays on your machine

**Private means absent.** Mark a folder, a note or a single `> [!private]` block
private and the agent never reads it, never writes to it and never sees it in a
prompt. This is enforced in Rust at every read and write choke point, and a private
file returns byte-for-byte the same error as a file that was never there.

**Every write stops at a diff.** Allow it, deny it, or edit it and then apply. The
gate lives in the same Rust layer that owns file access, so nothing in the interface
can step around it.

**Five network exits, each one triggered by you.** The LLM channel, a page you
opened, the reader's image proxy, a blocking-rule download and an update check.
They are listed in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), and everything
else works with the network off.

**Arbitrary code never runs by itself.** Opening a note is not running it. Every
execution starts from something you did, and the agent has no tool that can run
code at all.

## The Markdown dialect

Semantics are ordinary Markdown that other tools already understand:

```markdown
- [ ] A task 📅 2026-08-20          ← due date
- [/] In progress
- [x] Done

> [!claim] Sleep consolidates memory ^c1        ← a claim, with a block id

> [!evidence] Walker 2017, deprivation study
> supports:[[Sleep and Memory#^c1]]             ← evidence, pointing at the claim

![[Sleep and Memory#^c1]]                        ← the block, embedded read-only
- 再读 [author:: Le Guin] [status:: reading]    ← inline fields: queryable, groupable
```

Every construct above renders as a list item or a blockquote anywhere else. The
complete dialect and each fallback: [docs/SYNTAX.md](docs/SYNTAX.md).

## Status

As of September 2026, working on `main`: the editor and its live preview, the
Agenda, Board, Timeline, Graph, Claims, Evidence and Review views, the block index
and query API, the agent with its tools behind the write gate, privacy zones, the scripting sandbox, the built-in browser with clipping,
side-by-side translation of a note, a web page or a PDF, and an interface in English
and Chinese (Japanese, Korean, French, German and Spanish follow as their translations land).

Next, in the order the study loop needs them: PDF annotation that lands in the
source note, reference management on source notes (DOI, BibTeX export), and reading
across papers (one claim, every source that bears on it). After that: semantic
retrieval, installable extensions and themes, and split-pane browsing in the main
window. The seam each one grows from is listed in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Learn more

- [docs/SYNTAX.md](docs/SYNTAX.md) — the Markdown dialect and how it degrades
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — the layers, the flows, the security
  boundaries, and the network-exit list
- [docs/GLOSSARY.md](docs/GLOSSARY.md) — the terms this project uses, in both languages
- [CONTRIBUTING.md](CONTRIBUTING.md) — how to build it, and the design rules a PR is
  reviewed against

## Community and feedback

| You want to… | Go to |
|---|---|
| Report a bug or ask a question | [Discussions](https://github.com/pangyexing/vemry/discussions) |
| Work on something in vemry | Say so in Discussions first; then [CONTRIBUTING.md](CONTRIBUTING.md) |
| Automate your own vault | A script in `.vemry/scripts/` — vemry itself needs no change |

vemry is developed by one person with limited time, so external pull requests are
closed for now.

## Build from source

```bash
git clone https://github.com/pangyexing/vemry.git
cd vemry
pnpm install          # npm cannot install this: vite's devDependencies use link:
pnpm tauri dev        # needs `source ~/.cargo/env` first
```

Requires Rust (stable) and Node 22+. Updates to the packaged app are signed with
minisign and verified against a public key compiled into the app; downloading and
installing always need your confirmation, and the startup check is off by default.

## License

[Functional Source License 1.1, Apache-2.0 future license](LICENSE) (FSL-1.1-ALv2).

You can use it, build it, modify it and deploy it inside your company. The one
thing you may not do is offer it, or something substantially like it, as a
competing commercial product or service. Two years after each version is released,
that version becomes Apache-2.0 automatically.
