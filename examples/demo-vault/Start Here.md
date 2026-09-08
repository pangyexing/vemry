---
title: Start Here
tags: [meta]
---

# Start Here

This is a small, real vault. Every file is plain Markdown on disk — open the
folder in Finder and you will see exactly what you see here.

It is wired so that all seven projections have something to show. Open them from
the activity bar on the far edge of the window, or with ⌘P.

## What is in here

- [[Sleep and Memory]] — the research thread, with claims that evidence points at
- [[Open Questions]] — what is still unsettled
- [[Self-Experiment]] — the project, with the tasks that fill Agenda and the board
- [[Reading Queue]] — what is queued, in progress and done
- [[2026-08-25]] — the most recent journal entry
- [[Attention Is All You Need]] — a paper under study: its source in blocks, a
  chapter with a lab block you can run and drag, and the cards its gaps left

## Everything below is live

This is a query block. It is plain Markdown — a fenced block any renderer shows
as code — but vemry runs it and renders the result, refreshing as the index changes.

```vemry-query
kind: task
task_state: todo
limit: 5
```

Claims and evidence are ordinary callouts, so they survive anywhere:

> [!claim] A vault you cannot leave is not yours ^ownership

> [!evidence] Every extension in this vault degrades to readable Markdown
> supports:[[Start Here#^ownership]]
> The dialect and its fallbacks are written down in docs/SYNTAX.md — that document
> is the receipt for this claim, not a promise.

## Try this

1. ⌘P — every capability in the app is in that one list
2. Click a `[[link]]` to follow it; ⌘⌥← comes back
3. Open the graph and watch this note sit at the centre of it
4. Mark `private/` as a privacy zone and ask the AI to list your notes — it will
   not see the folder, and it will not know it is missing
