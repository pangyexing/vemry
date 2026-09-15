# Overlay catalogs

English and Chinese are written in `scripts/gen-locales.py`. Every other interface
language lives here as an overlay: `<tag>.json` holds only the keys that have been
translated, flat and dotted like `locales/en.json`, and the app falls back to
English for the rest. The generator (`python3 scripts/gen-locales.py`) validates an
overlay — unknown keys, placeholder names, plural forms — and writes
`locales/<tag>.json` from it.

Rules for a translator, human or model:

- Terms come from `docs/GLOSSARY.md`; a concept word (vault, block, projection,
  privacy zone, permission gate, skill) gets one rendering per language, decided
  there before it enters a file.
- `{name}` placeholders stay exactly as in the English string.
- Plural keys carry the CLDR forms the language needs (`_one`/`_other` for German,
  `_one`/`_many`/`_other` for French, `_other` alone for Japanese and Korean).
- `slash.*` stays English in every language: those are typed commands.
- `langName.*` stays as generated: a language's own name is not translated.
- No anthropomorphic phrasing for the AI, and headings say what the reader gets.

Tone per language:

| Tag | Register |
|---|---|
| `ja` | です・ます体; avoid the imperative in interface text |
| `ko` | 하세요체 (polite, the interface convention) |
| `de` | du, not Sie |
| `fr` | vous |

A language is offered in the settings picker once its high-exposure groups (`app.`,
`chat.`, `settings.`, `menu.`, `cmd.`, `permission.`, `skill.`) are complete and a
native speaker has read them; the generator prints that count.
