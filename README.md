# vemry

**A local-first AI study workbench.**

Papers, pages and PDFs come in as plain Markdown on your own disk. You read,
translate, question and card them with an AI that cites the block it read, stops at
a diff before it writes, and never sees what you mark private.

[![CI](https://github.com/pangyexing/vemry/actions/workflows/ci.yml/badge.svg)](https://github.com/pangyexing/vemry/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/pangyexing/vemry?include_prereleases&sort=semver)](https://github.com/pangyexing/vemry/releases)
[![License](https://img.shields.io/badge/license-FSL--1.1--ALv2-blue)](#license)
![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)
![No telemetry](https://img.shields.io/badge/telemetry-none-success)

[English](README.md) · [简体中文](README.zh-CN.md)

![vemry — editor, file tree and outline](docs/assets/hero.png)

---

vemry keeps your knowledge as plain Markdown files on your own disk. No database,
no account, no sync service.

On top of that folder it builds exactly one derived thing: a block-level index.
Every view you see — agenda, board, timeline, graph, claims, evidence, review, session
replay — is a query over that index. So are your scripts. So are the AI agent's
tools. There is no second data path, which is what makes the AI native here rather
than bolted on.

And the boundary runs the other way too: anything you mark private is filtered out
in Rust, at the layer that owns file access. Not "access denied" — absent.

## Why vemry

### Understand a paper, and keep what you learned

Paste an arXiv link, drop in a PDF or point at a clipped page and say *study
this*. The paper arrives in your vault as a note you can move around in — every
section, equation, figure and table its own block — and the reading unfolds
chapter by chapter, at your pace. Each chapter starts from what your vault already
knew about the idea and marks what is new, and every sentence links to the passage
it came from, so the original is always one click away.

Explain a chapter back in your own words, and you see exactly where your account
drifts from the paper. Want to try the idea? Ask for an experiment and a runnable
block appears in the chapter. What you did not fully get becomes flashcards, and
the Review view brings them back just before you would have forgotten.

Reading in a language you are less fluent in? Open the text beside its
translation — a note, a web page or a PDF, aligned paragraph by paragraph. And
everything you make along the way — the source, your chapters, your cards, the
translation — is a plain Markdown file in your folder, yours to open anywhere.

### It is a folder. You can walk away.

Your vault is plain Markdown you could have written by hand. vemry recognises a
superset of standard Markdown, and every extension it adds degrades to something
readable in any other renderer — the dialect and each fallback are written down in
[docs/SYNTAX.md](docs/SYNTAX.md), so "no lock-in" is a document you can check rather
than a promise. The index is derived and rebuildable; delete it and nothing is lost.

### One index, every view

```
Markdown on disk  →  block index  →  agenda · board · timeline · graph
(single truth)       (only derived    claims · evidence · review · session replay
                      layer)          your JS scripts · the agent's tools
```

Seven projections, all queries. Write a `vemry-query` fence inside a note and it
renders live results that follow the index. Because the agent queries the same
`query()` your views do, anything you can see it can compute, and nothing is
computed for it alone.

![Board view — every task in the vault, by state](docs/assets/kanban.png)

![Graph — the same index, drawn as links; the two private notes are visible to you and not to the model](docs/assets/graph.png)

### Everything is a command; everything is scriptable

Every capability in the app registers in one searchable palette (⌘P) — no feature
hides in a menu. Drop a `.js` file into `.vemry/scripts/` and it is live on save:
it runs in a QuickJS sandbox on the Rust side with no filesystem, no network and no
processes, reaching the vault only through the `vemry.*` API. Scripts can register
palette commands, subscribe to vault events, and run on a cron.

Arbitrary code never runs by itself. Opening a note is not running it: every
execution path starts from something you did, and the agent has no tool that can
run code at all.

### What you mark private does not exist

Mark a folder, a note or a single `> [!private]` block private and the agent never
reads it, never writes to it, and never sees it in a prompt. This is enforced at
every read and write choke point in Rust, and a private file returns byte-for-byte
the same error as a file that was never there — so the model cannot even infer that
something is being withheld.

The AI's writes stop at a diff you allow, deny, or edit and then apply. Every
session — each read, each write, each permission decision, each privacy filter — is
a JSONL audit log inside your vault, replayable in the app.

![The write gate — the agent proposed this edit; nothing reaches disk until you answer](docs/assets/permission-diff.png)

### Offline unless you ask otherwise

Exactly five things can reach the network, each triggered by something you did, each
listed in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): the LLM channel, a page you
opened, the reader's image proxy, a blocking-rule download, and an update check. No
telemetry, no account. Everything else works with the network off — including the
editor, every view, search, and your scripts.

The AI is optional. Without a key, vemry is a complete local editor.

## Install

Download for macOS (`.dmg`), Windows (`.exe`) or Linux (`.AppImage` / `.deb` /
`.rpm`) from [Releases](https://github.com/pangyexing/vemry/releases). Updates are
signed with minisign and verified against a public key compiled into the app;
downloading and installing always need your confirmation, and the startup check is
off by default.

macOS is not yet notarised, so the first launch needs a right-click → Open, or
`xattr -cr /Applications/vemry.app`.

### From source

```bash
git clone https://github.com/pangyexing/vemry.git
cd vemry
pnpm install          # npm cannot install this: vite's devDependencies use link:
pnpm tauri dev        # needs `source ~/.cargo/env` first
```

Requires Rust (stable) and Node 22+.

## Quick start

1. **Open the example vault.** Clone this repo, then ⌘P → *Open vault…* →
   `examples/demo-vault`. It is a small real vault, wired so every projection has
   something to show.
2. **Press ⌘P.** Everything vemry can do is in that one list.
3. **Add an AI key** (optional). Settings → API key. The default endpoint is Zhipu,
   whose `glm-4.7-flash` is free and supports tool calls; any OpenAI-compatible
   endpoint works, including a local Ollama.
4. **Study a paper.** Open the AI panel, pick *Study a paper* and paste an arXiv
   link, or drop in a PDF. The source note, the chapters and the cards arrive as
   diffs you approve — which is also the shortest way to see the permission gate for
   yourself. The example vault has one paper already worked through, under `papers/`.

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

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — the layers, the flows, the security
  boundaries, and the network-exit list
- [docs/SYNTAX.md](docs/SYNTAX.md) — the Markdown dialect and how it degrades
- [docs/GLOSSARY.md](docs/GLOSSARY.md) — the terms this project uses, in both languages
- [CONTRIBUTING.md](CONTRIBUTING.md) — how to build it, and the design rules a PR is
  reviewed against

## Status

Working today on `main`: the editor and its live preview, seven projections, the
block index and query API, the agent with its domain tools behind a write gate,
privacy zones, the scripting sandbox, the built-in browser with clipping and a
reading projection, side-by-side translation of a note, a web page or a PDF, and an
interface that speaks English and Chinese.

**The published installers predate this README.** The English interface in
particular has not shipped yet — the latest release is Chinese-only. Until 0.9.0 is
released, build from source to get what is described here.

Next, in the order the study loop needs them: PDF annotation that lands in the
source note, reference management on source notes (DOI, BibTeX export), and reading
across papers (one claim, every source that bears on it). After that: semantic
retrieval, installable extensions and themes, and split-pane browsing in the main
window. The seam each one grows from is listed in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Contributing

vemry is developed by one person. The source is public so that you can audit it and
build it yourself; external pull requests are off by default, and bug reports go
through [Discussions](https://github.com/pangyexing/vemry/discussions). If you want
to work on something, say so there first — [CONTRIBUTING.md](CONTRIBUTING.md) has a
five-minute tour of where things live. If you only want to automate your own vault,
you do not need to touch vemry at all: write a script in `.vemry/scripts/`.

## License

[Functional Source License 1.1, Apache-2.0 future license](LICENSE) (FSL-1.1-ALv2).

You can use it, build it, modify it and deploy it inside your company. The one
thing you may not do is offer it, or something substantially like it, as a
competing commercial product or service. Two years after each version is released,
that version becomes Apache-2.0 automatically.
