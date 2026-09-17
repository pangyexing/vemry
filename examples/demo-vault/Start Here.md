---
title: Start Here
tags: [meta]
---

# Start Here

This is one learner's vault: a paper being read in chapters, the cards it left behind, a research thread on what sleep does to memory, a nap experiment that uses the cards as its test, an inbox of quick captures, a clipped page, a journal and a private folder. Every file is plain Markdown — open the folder in any editor and it is all there.

It is a snapshot of one week in September 2026, so the dates read from there.

## What is where

- [[Attention Is All You Need]] — the paper under study, as *Study a paper* leaves it: the source in blocks, three chapters at three stages, an experiment you can run, and [[papers/vaswani-2017/cards|the cards]] made from the gaps in the retelling
- [[Sleep and Memory]] — a research thread: claims, and the evidence notes that point at them; one claim has nothing behind it yet
- [[Nap Experiment]] — a project: its tasks fill Agenda and the board, its decision and its hypothesis are typed blocks, and its raw log is in [[Health Log]], which the AI cannot see
- [[Reading Queue]] — what is done, in progress, due and undated
- [[inbox]] — quick captures waiting to be filed
- [[2026-09-16]] — the latest journal entry

## Where to look

Every view on the activity bar is a query over the same index; nothing is stored twice. What is due right now, across the whole vault:

```vemry-query
kind: task
task_state: todo
due_before: today
```

- **Agenda** — today, overdue, upcoming; the write-up stays grey until the log it waits on is done
- **Board** — the same tasks in columns; drag a card to reschedule it
- **Claims** and **Evidence** — every claim, and which of them nothing supports yet
- **Review** — the cards that are due; a grade is written onto the card's own line
- **Graph** — how the notes connect; the two private notes are on it for you, not for the model
- **Session Replay** — two past AI sessions: the inbox being filed (the plan, the card, four moves, each write through the gate), and an earlier edit that was refused at the gate and never reached the file

## Try this

1. Open [[01 Scaled dot-product attention]] and press Run on the experiment: drag $d_k$ with the scale off and watch the weights collapse onto one key
2. Open the Review view and grade a card
3. Press ⌥⇧Space (Alt+Shift+Space on Windows and Linux) from any app, type a line, press Enter: it lands in [[inbox]]. With an API key set, ask the AI to *file the inbox* — it proposes a home for each capture and moves the blocks you approve
4. Ask the AI to *check my retelling* of chapter 1: it reads *In my words* against the source blocks and says where the retelling drifts
5. Ask it to *study* another paper — paste an arXiv link
6. Ask it to list every note in this vault: `private/` is not in the answer, and it does not know that anything is missing
7. ⌘P — every command, including the two this vault's scripts add: *Open today's journal note* and *What is due today?*
