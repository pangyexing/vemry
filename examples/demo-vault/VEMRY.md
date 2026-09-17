# VEMRY.md — conventions for this vault

This file is injected into the AI agent's system prompt: it is the constitution it reads to understand this vault.

## Structure

- `Start Here.md` — the hub; every area of this vault is one link away
- `inbox.md` — quick captures, one list item each with a time stamp, filed from here into their homes
- `papers/<author-year>/` — one folder per paper under study: `source.md` is the paper in blocks (a quotation, never edited), the hub note beside it maps known and to-learn, `chapters/` holds the reading one chapter per note, `cards.md` the paper's deck
- `cards/` — decks not tied to one paper; the Review view reads every `> [!card]` in the vault
- `research/` — one note per question; claims carry `^id` so evidence can point at them
- `research/evidence/` — one note per source; each finding is a `> [!evidence]` with `supports:`
- `projects/` — things with a next action; tasks live here and in the journal, not in research notes
- `journal/` — one note per day, `YYYY-MM-DD.md`
- `clips/` — saved web pages, written by the clipper, not by hand
- `private/` — a privacy zone; you cannot see it and should not mention it

## Conventions

- Tags say what a note is about (`#sleep` `#paper` `#method` `#reading`), never its status — state lives in the task syntax and in fields
- Link a concept the first time it appears in a note
- A claim ends its line with a `^id`; evidence points at it with `supports:[[Note#^id]]`
- A confusion while reading is a task tagged `#question` in the chapter note; the answer goes under the task
- A task's attributes come after its text, at the end of the line, never before it: `📅 YYYY-MM-DD` for a deadline, `⏳ YYYY-MM-DD` for the day it is planned for, `⏩ YYYY-MM-DD` for the earliest start, `❗` for important (`❗❗` urgent), `🔁 every week` to repeat, `🕐 30m` for an estimate

## AI preferences

- Search this vault before searching the web; say which notes you used
- Say what you intend to change before you change it
- New evidence goes in `research/evidence/`; new cards go next to their source (`papers/<slug>/cards.md` or `cards/`); nothing new goes in `journal/`
