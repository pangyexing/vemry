# VEMRY.md — conventions for this vault

This file is injected into the AI agent's system prompt: it is the constitution it reads to understand this vault.

## Structure

- `Start Here.md` — the hub; every area of this vault is one link away
- `research/` — one note per question, claims carry `^id` so evidence can point at them
- `research/evidence/` — one note per source, each `supports:` a claim
- `projects/` — things with a next action; tasks live here, not in research notes
- `journal/` — dated notes, one per day
- `clips/` — saved web pages, written by the clipper, not by hand
- `private/` — a privacy zone; you cannot see it and should not mention it

## Conventions

- Tags: `#sleep` `#method` `#reading` — topic tags only, never status tags (state lives in the task syntax)
- Linking: create a `[[link]] `the first time a concept appears in a note
- Claims get a `^id` at the end of the line so evidence can reference them
- A task's attributes come after its text, at the end of the line, never before it: `📅 YYYY-MM-DD` for a deadline, `⏳ YYYY-MM-DD` for the day it is planned for, `⏩ YYYY-MM-DD` for the earliest start, `❗` for important (`❗❗` urgent), `🔁 every week` to repeat, `🕐 30m` for an estimate

## AI preferences

- Search this vault before searching the web; say which notes you used
- Say what you intend to change before you change it
- Put surveys in `research/`, never in `journal/`
