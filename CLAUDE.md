# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Start here

- [CONTRIBUTING.md](CONTRIBUTING.md) — the architecture in five minutes (a "change X, go here" table), development environment, the pre-commit checks, and the design principles a PR is reviewed against
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — the full architecture: the layers in detail, the key flows (an AI write, an external edit syncing back, a clipboard paste), security boundaries, how privacy isolation is implemented, and the seams left for later
- [docs/SYNTAX.md](docs/SYNTAX.md) — vemry's Markdown dialect: every construct it recognises on top of CommonMark/GFM, and what each degrades to in a standard renderer (read this before touching syntax or rendering)

Read these before making non-trivial changes — this file only adds what they don't already cover.

## Commands

```bash
pnpm install            # npm fails here: vite's devDependencies use the link: protocol, which npm does not support
pnpm tauri dev           # needs `source ~/.cargo/env` first

cargo test --workspace   # Rust tests
cargo clippy --workspace -- -D warnings # CI enforces zero warnings
pnpm check                # svelte-check
pnpm build                # the frontend builds

# after touching the agent layer, run one real end-to-end turn (crates/vemry-agent/tests/live.rs, --ignored by default)
VEMRY_TEST_API_KEY=sk-... cargo test -p vemry-agent --test live -- --ignored

# print the system prompt a vault gets (what the model is sent; the UI and the session log show none of it)
VEMRY_PROMPT_VAULT=/path/to/vault cargo test -p vemry-agent print_system_prompt -- --ignored --nocapture
```

## Architecture essentials

Five-crate Cargo workspace + Svelte 5 frontend (`src/`). Core principle: disk Markdown is the single source of truth; everything else (block index, every UI view, AI tools, user scripts) is a derived, rebuildable projection queried through one `query()` API — never a separate data path for AI vs. UI.

Layering (dependency direction is one-way):

- `vemry-core` — vault engine, **zero UI/AI/Tauri deps**. Parses Markdown into a typed `BlockIndex`, serves `query()`, handles the filesystem watcher and privacy-zone enforcement.
- `vemry-agent` — AI layer, depends on `core` only (not Tauri). Owns the LLM chat client (Chat Completions + the v0.4 DeepSeek Responses transport with server-side web_search — the LLM channel boundary), the 9 domain tools, the write `PermissionGate`, instruction-level skills, and the agent loop/session audit log.
- `vemry-script` — user-script layer (v0.3), depends on `core` only. QuickJS sandbox (no fs/net/process; `vemry.*` API whitelist only) on a dedicated thread; commands, event/cron workflows, first-write grant + audit log.
- `vemry-clip` — clip layer (v0.4), depends on `core` only, pure logic. Readability extraction (dom_smoothie) + html→markdown (htmd) + frontmatter/quote composition; clip URLs must stay in `[..](url)`/`<url>` form (bare `#fragment` gets parsed as a tag — regression-tested).
- `src-tauri` — glue layer: all IPC commands (`commands.rs`), agent event bridging (`agent_bridge.rs`), script bridging (`script_bridge.rs`), the browser panel and tabs/clipping/adblock (`browser/`), the detached AI chat window (`agent_window.rs` — a second webview running our own bundle; state truth stays in Rust session files, both windows are projections), native clipboard access, settings storage. `commands.rs::broadcast` is the single fan-out for vault events (frontend + workflow dispatch + script hot reload).
- `src` — Svelte 5 frontend; `api.ts` is the sole IPC entry point (typed mirror of `commands.rs` — **the two must be changed together**).
- `locales/{en,zh-CN}.json` — every user-visible string, generated from one table by `scripts/gen-locales.py`. Read by both `src/lib/i18n/i18n.svelte.ts` and `src-tauri/src/i18n.rs`; wording follows `docs/GLOSSARY.md`. Nothing user-visible may be hardcoded in a component or a command.

Design constraints worth internalizing before editing:

1. **Permission boundary lives in Rust, next to file-write capability** (`vemry-agent/permission.rs`) — the frontend only renders the diff/decision, it cannot bypass the gate. Any new AI write path must go through `PermissionGate`.
2. **Privacy zones make marked content genuinely nonexistent to the agent**, not merely access-denied — enforced at three read/write/prompt choke points in Rust (`vemry-core/src/privacy.rs` + `vault.rs`). If you add a new agent-visible read path, it must also be filtered here; see "Privacy isolation" in docs/ARCHITECTURE.md for the exact enforcement points and known limitations (two deliberate, narrow exceptions: `.vemry/skills/` via `read_skill`, and the schema of `.vemry/types/` — name, kind, tag, field keys, description — in the system prompt).
3. **Three audiences read strings out of Rust, and they are not the same**: the user (toasts, dialogs, VEMRY.md scaffolding) reads the catalog; the model (prompt envelopes, tool errors, attachment placeholders) reads English unconditionally — the system prompt already tells it to answer in the user's language; script authors read English constants, like any JS runtime's messages.
4. **Arbitrary code never auto-runs, and the agent can never run it**: every script/code-block execution starts from an explicit user action (command, run button, a trigger the user installed); there is no agent-facing run_code tool and script writes structurally cannot re-trigger workflows (loop prevention — see "Programmability" in docs/ARCHITECTURE.md).

## Writing conventions

**Language: everything written in this repository is in English** — commit
subjects and bodies, PR titles and bodies, code comments (including `///` and
`/** */` doc comments), and documentation (this file, `CONTRIBUTING.md`,
`docs/`).

Two deliberate exceptions, both user-facing translations rather than project
writing: `README.zh-CN.md` and `docs/SYNTAX.zh-CN.md` exist so Chinese readers get
the same two documents in their own language. They mirror their English originals
section for section and change together with them.

Every comment and document in the repository has been swept into English. What
Chinese remains is functional and stays: the second element of each pair in
`scripts/gen-locales.py`'s table (and the generated `locales/zh-CN.json`), the
Chinese column of `docs/GLOSSARY.md`, CJK test fixtures, and the character
classes and match keys of a few regexes. New comments are written in English from
the start.

Commit messages, documentation and code comments **describe the behaviour itself
and do not name other products** — write "preview tab semantics: a single click
reuses the preview slot", not "same as such-and-such editor". The reason: that
kind of analogy only informs people who have used the other product, and the
mechanism can always be stated directly.

The one exception is **interoperability facts**: keep the name when the name is
itself the thing being parsed — a directory that really exists on disk
(`.obsidian`), the renderer of a site being adapted for, a third-party filter
list the user subscribes to. Measured evidence about another product keeps the
measurement and drops the name ("measured across two mainstream desktop editors'
bundles: zero occurrences"): what the reader needs is that it was measured, not
whose bundle it was.
