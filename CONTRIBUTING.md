# Contributing

vemry is developed by one person with limited time: **external pull requests are
closed for now**, and the repository does not have an issue tracker. Bug reports
and questions go through [Discussions](https://github.com/pangyexing/vemry/discussions); security reports
follow [SECURITY.md](.github/SECURITY.md).

If you want to work on something — a fix you have already tested on your platform,
a locale — say so in Discussions first. Pull requests are opened for
collaborators case by case, and the licensing of your contribution is settled
there before any code moves. The rest of this document is the map of the codebase
that a collaborator (or a curious reader) needs; **most of it does not require
writing Rust**.

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
| AI tools / prompts / skills | `crates/vemry-agent/src/{tools,prompt,skills}.rs` + the built-in skills in `crates/vemry-agent/skills/` | Rust |
| LLM transport (Chat Completions / DeepSeek's Anthropic-format line + search) | `crates/vemry-agent/src/{client,anthropic}.rs` | Rust |
| The provider (bring your own key / vemry hosted) and the hosted model catalog | `crates/vemry-agent/src/provider.rs` + `src-tauri/src/settings.rs` | Rust |
| The hosted account (sign-in, the device token, the entitlement snapshot, relay errors) | `src-tauri/src/account.rs` + `src/lib/components/AccountPanel.svelte` + `src/lib/stores/account.svelte.ts` | Rust + Svelte |
| Script sandbox / `vemry.*` API / workflows | `crates/vemry-script/src/` | Rust |
| Clipping (extraction / html→md / frontmatter) | `crates/vemry-clip/src/` | Rust |
| Browser tabs and panel / detached window / clip commands / ad blocking | `src-tauri/src/browser/` + `src/lib/components/BrowserPanel.svelte` | Rust + TS |
| The detached AI chat window (creation / handoff / close handshake / relay) | `src-tauri/src/agent_window.rs` + `src/AgentWindow.svelte` + `src/lib/host.ts` | Rust + TS |
| The quick capture window (the global shortcut / show and hide / the landing relay) | `src-tauri/src/capture_window.rs` + `src/CaptureWindow.svelte` + `src/lib/components/CaptureComposer.svelte` + `src/lib/capture.ts` | Rust + TS |
| The phone inbox (the pull loop, the landing vault and the direct landing, the ledger, pairing) | `src-tauri/src/inbox.rs` + the phone section of `src/lib/components/settings/CapturePage.svelte` | Rust + TS |
| The resident state (the close button hides the main window, the menu bar / tray icon) | `src-tauri/src/tray.rs` + the close hooks in `src-tauri/src/lib.rs` | Rust |
| Parsing / indexing / search | `crates/vemry-core/src/` | Rust |
| Note history (what a write replaced, and restoring it) | `crates/vemry-core/src/history.rs` + `src/lib/components/HistoryDialog.svelte` | Rust + Svelte |
| A setting (its row, its words, where search and "go to setting" find it) | `src/lib/settings/index.ts` (the id, page and catalog keys) + the page in `src/lib/components/settings/` + `src-tauri/src/settings.rs` | TS + Rust |
| IPC commands | `src-tauri/src/commands.rs` + `src/lib/api.ts` | Rust + TS (change both together) |
| Interface copy / adding a language | `scripts/gen-locales.py` → `locales/*.json` | Python + JSON (new terms go through `docs/GLOSSARY.md` first) |

Automating your own vault needs no change to vemry itself: drop a `.js` file into
the vault's `.vemry/scripts/` (see the `vemry.*` API in the README's scripting
section and ARCHITECTURE's "Programmability").

## Development environment

```bash
rustup toolchain install stable
pnpm install            # pnpm only: the lockfile and the supply-chain settings in pnpm-workspace.yaml are pnpm's
pnpm tauri dev          # needs `source ~/.cargo/env` first
```

## Before you commit

```bash
cargo test --workspace   # Rust tests (core/agent/script carry the highest density — keep it that way)
cargo clippy --workspace -- -D warnings   # zero warnings, as CI enforces
cargo test -p vemry-agent --test evals   # offline agent evals: each tests/evals/*.json replays scripted model output through the real runner
python3 scripts/gen-locales.py           # regenerate locales/*.json; CI fails if a committed catalog differs from the generator's output
pnpm check               # the ink check (check:ink: every text ink ≥ its contrast on every ground, both themes; no data-theme selectors), then svelte-check clean (also the gatekeeper for missing catalog keys)
pnpm build               # the frontend builds
```

The evals are part of `cargo test --workspace` as well; the separate line runs
only them after a change to prompts, tools or skills.

### Live API integration test (optional, skipped by default)

After touching the agent layer, run one real end-to-end turn (it costs a very
small number of tokens):

```bash
VEMRY_TEST_API_KEY=sk-... cargo test -p vemry-agent --test live -- --ignored

# print the system prompt a vault gets (what the model is sent; the UI and the session log show none of it)
VEMRY_PROMPT_VAULT=/path/to/vault cargo test -p vemry-agent print_system_prompt -- --ignored --nocapture
# optional: VEMRY_TEST_BASE_URL / VEMRY_TEST_MODEL (defaults to DeepSeek deepseek-flash)
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
4. **Offline first**: exactly six controlled network exits — the LLM channel
   (server-side search included), a browser window the user opened, a
   blocking-rule download, the image proxy for the reader and the clipper
   (`vemry-asset://`), the update check and download (user-triggered by
   default), and the hosted account and relay (only after signing in or switching
   the provider to vemry hosted; see ARCHITECTURE's "The network-exit list"). No
   other network calls, and no telemetry.
5. **Everything is a command**: a new capability registers in the command
   registry, not just as a button somewhere.
6. **Core depends on neither UI nor AI**: `vemry-core` stays free of Tauri and
   agent dependencies so it can be reused on its own; `vemry-script` depends only
   on core.
7. **Copy has a single source**: user-visible strings live in `locales/*.json`
   (English and Chinese in `scripts/gen-locales.py`, the other languages as
   overlays in `locales/src/`; regenerate after either), never hardcoded in a
   component or a command; new terms go through [docs/GLOSSARY.md](docs/GLOSSARY.md)
   before they reach a catalog; the two READMEs mirror each other section for
   section, and are the only two — other languages get the website, not a README.
   Copy that goes to the model (prompt envelopes, tool errors) and errors from the
   script API are two different audiences — both are English, and neither belongs
   in the catalog.
8. **Arbitrary code never runs by itself**: every script and code-block execution
   path starts from an explicit user action (a command, a run button, a trigger
   the user installed). The agent gets no tool that can execute arbitrary code.
9. **The user stays the author**: an AI feature is reviewed by what the user
   understands, keeps or decides after it runs, not by what it produces. The
   chores (clipping, translating, filing, tracing sources, making cards) may be
   done for the user; the reading, the retelling and the decisions are never done
   instead of them. Outward copy says what the reader gets, never what they must
   do (the fixed phrasings in the glossary).
10. **The interface language is data**: dates and numbers are shown through
   `Intl` with `i18n.locale`, names sort through `compareNames`, fonts and line
   breaking follow the `lang` attribute. What is written to disk never follows the
   interface language — ISO dates, `every 2 weeks`, `3d2h`, callout type names,
   the `/` commands — so a note is the same file on a Japanese and a German
   machine. No `Ctrl+Alt+<letter>` shortcut: on French, German and Spanish layouts AltGr
   is Ctrl+Alt, so such a binding steals a character key.

## Writing

Everything written in this repository is in English — commit messages, PR text,
code comments and documentation. `README.zh-CN.md` and `docs/SYNTAX.zh-CN.md` are
the two deliberate exceptions: user-facing translations that mirror their English
originals and change with them. See CLAUDE.md for the full convention, including
the rule against naming other products.

## License

[FSL-1.1-ALv2](LICENSE): source-available, any use except a competing commercial
product, each version converting to Apache-2.0 two years after release. Because
pull requests are not open, there is no contributor agreement to sign up front;
a contributor grants the licensor the rights to their contribution as part of
becoming a collaborator (see the top of this document).
