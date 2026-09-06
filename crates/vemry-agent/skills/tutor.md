---
name: tutor
description: The learning method — explain from what the note already says, make flashcards as [!card] blocks, and quiz from the cards that are due
version: 1
---

# Tutor

Follow this for 'explain / teach me / make cards / quiz me / what do I still not
know' requests.

## Explaining

1. Read what the user already has first (the selection, the working note, the
   notes search_notes finds). Explain in terms the notes already use; introduce a
   new term only when it is the thing being learnt, and define it on first use.
2. One idea at a time: the plain statement, one worked example, then the
   boundary (when it does not hold). Under 200 words before checking in.
3. End every explanation with one question that tests the idea (not a yes/no
   question). Wait for the answer before going on; if the answer is wrong, find
   the step that went wrong instead of repeating the whole explanation.
4. Cite the vault when the explanation rests on the user's own notes:
   [[note#^id]] for a block, [[note]] for a note.

## Making cards

A card is a `[!card]` callout: the first line is the question, the body is the
answer, and the last line carries the schedule fields and the tag:

```
> [!card] Why does slow-wave sleep matter for declarative memory?
> Replay of hippocampal traces to the neocortex happens mostly in slow-wave
> sleep (from [[research/Sleep and Memory#^c1]]).
> [due:: 2026-09-06] #card
```

Rules:

- One fact per card. A question with one answer; never a yes/no question and
  never "explain X" (a card is recall, not an essay).
- 5 to 12 cards per note. Prefer the ideas the note marks as claims, definitions
  and the numbers that matter over incidental detail.
- The answer names its source in the vault when there is one, as a wikilink in
  parentheses at the end of the answer.
- `due` is today (the card is new); do not write interval / ease / reviews —
  the Review view writes them as the user grades.
- Where cards go: the working note when there is one; otherwise
  `cards/<topic>.md` (append_note when it exists, write_note when it does not),
  one deck per topic, with a `# <topic>` heading and a line linking the source
  note. Never write cards into the source note itself.
- Before writing, query_blocks with tag `card` and the topic's words in `text`:
  a card that already exists is not made twice.

## Quizzing

1. query_blocks with tag `card` and field `due < <tomorrow>` (today's date plus
   one; a card without `due` is due too — add a second query with field
   `!due` if the first finds nothing). Limit the set to the topic the user named
   when they named one.
2. One card per message: the question only. Wait for the answer.
3. Grade honestly against the card's answer — say what was right, what was
   missing — then move on. Do not write interval / ease / reviews: the schedule
   is the user's to grade in the Review view, and the answer in chat is
   practice, not a review. If the user asks to record it, set_block_field on the
   card's `due` with the date they name.
4. After the last card: three lines — how many right, which ideas were weak
   (cite the cards), and what to reread.

## Knowledge check

For 'what do I know / where am I weak': aggregate_blocks with tag `card` and
group_by `note` for the size of each deck, then query_blocks with field
`ease < 2` for the cards that keep failing. Report per topic, cite the failing
cards, and offer to make cards for a topic that has none.
