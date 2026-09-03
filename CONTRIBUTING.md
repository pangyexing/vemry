# Contributing

Thanks for your interest in vemry. The bar is layered — **most contributions do
not require writing Rust**.

## The architecture in five minutes

Start with [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). The one-sentence version:

> Markdown on disk is the single source of truth → `vemry-core` parses it into a
> block-level index → every interface, every script and every AI tool is a
> projection queried out of that one index.

| You want to change… | Go here | Stack |
|---|---|---|
| UI / views / interaction | `src/` | Svelte 5 + TS |
| A new command | `src/lib/commands/registry.ts` + `App.svelte` | TS |
| Editor behaviour | `src/lib/editor/` | CodeMirror 6 |
| AI tools / prompts / skills | `crates/vemry-agent/src/{tools,prompt,skills}.rs` | Rust |
| LLM transport (Chat Completions / DeepSeek Responses + search) | `crates/vemry-agent/src/{client,responses}.rs` | Rust |
| Script sandbox / `vemry.*` API / workflows | `crates/vemry-script/src/` | Rust |
| Clipping (extraction / html→md / frontmatter) | `crates/vemry-clip/src/` | Rust |
| Browser windows / clip commands / ad blocking | `src-tauri/src/browser/` | Rust |
| The detached AI chat window (creation / handoff / close handshake / relay) | `src-tauri/src/agent_window.rs` + `src/AgentWindow.svelte` + `src/lib/host.ts` | Rust + TS |
| Parsing / indexing / search | `crates/vemry-core/src/` | Rust |
| IPC commands | `src-tauri/src/commands.rs` + `src/lib/api.ts` | Rust + TS (change both together) |
| Interface copy / adding a language | `scripts/gen-locales.py` → `locales/*.json` | Python + JSON (new terms go through `docs/GLOSSARY.md` first) |

Automating your own vault needs no change to vemry itself: drop a `.js` file into
the vault's `.vemry/scripts/` (see the `vemry.*` API in the README's scripting
section and ARCHITECTURE's "Programmability").

## Development environment

```bash
rustup toolchain install stable
pnpm install            # npm cannot install this: vite's devDependencies use the link: protocol
pnpm tauri dev          # needs `source ~/.cargo/env` first
```

## Before you commit

```bash
cargo test --workspace   # Rust tests (core/agent/script carry the highest density — keep it that way)
cargo clippy --workspace # no new warnings
pnpm check               # svelte-check clean (also the gatekeeper for missing catalog keys)
pnpm build               # the frontend builds
```

### Live API integration test (optional, skipped by default)

After touching the agent layer, run one real end-to-end turn (it costs a very
small number of tokens):

```bash
VEMRY_TEST_API_KEY=sk-... cargo test -p vemry-agent --test live -- --ignored

# print the system prompt a vault gets (what the model is sent; the UI and the session log show none of it)
VEMRY_PROMPT_VAULT=/path/to/vault cargo test -p vemry-agent print_system_prompt -- --ignored --nocapture
# optional: VEMRY_TEST_BASE_URL / VEMRY_TEST_MODEL (defaults to DeepSeek deepseek-v4-flash)
```

## Principles (a PR is reviewed against these)

1. **Disk is the single source of truth**: no database; the index must be fully
   rebuildable from the files.
2. **No lock-in**: new syntax lands in [docs/SYNTAX.md](docs/SYNTAX.md) (with
   [SYNTAX.zh-CN.md](docs/SYNTAX.zh-CN.md) updated alongside) and degrades
   gracefully in a standard Markdown renderer that has never heard of it. No
   private formats.
3. **The permission boundary lives in Rust**: every AI write path goes through
   `PermissionGate`; the frontend only renders the decision.
4. **Offline first**: exactly five controlled network exits — the LLM channel
   (server-side search included), a browser window the user opened, a
   blocking-rule download, the image proxy for the reader and the clipper
   (`vemry-asset://`), and the update check and download (user-triggered by
   default; see ARCHITECTURE's "The network-exit list"). No other network calls, and no
   telemetry.
5. **Everything is a command**: a new capability registers in the command
   registry, not just as a button somewhere.
6. **Core depends on neither UI nor AI**: `vemry-core` stays free of Tauri and
   agent dependencies so it can be reused on its own; `vemry-script` depends only
   on core.
7. **Copy has a single source**: user-visible strings live in `locales/*.json`
   (edit `scripts/gen-locales.py` and regenerate), never hardcoded in a component
   or a command; new terms go through [docs/GLOSSARY.md](docs/GLOSSARY.md) before
   they reach a catalog; the two READMEs mirror each other section for section.
   Copy that goes to the model (prompt envelopes, tool errors) and errors from the
   script API are two different audiences — both are English, and neither belongs
   in the catalog.
8. **Arbitrary code never runs by itself**: every script and code-block execution
   path starts from an explicit user action (a command, a run button, a trigger
   the user installed). The agent gets no tool that can execute arbitrary code.

## Writing

Everything written in this repository is in English — commit messages, PR text,
code comments and documentation. `README.zh-CN.md` and `docs/SYNTAX.zh-CN.md` are
the two deliberate exceptions: user-facing translations that mirror their English
originals and change with them. See CLAUDE.md for the full convention, including
the rule against naming other products.

## License

Dual-licensed FSL-1.1-ALv2. Opening a PR means you agree to release your
contribution under both.
