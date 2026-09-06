---
name: literature-note
description: Write up one source as an evidence note that supports a claim
triggers: [literature note, write up this paper, evidence note, 文献笔记, 读论文]
---

# Literature note

## When

"Write up this source / paper / article" into the vault.

## Plan

1. Read the source
2. Find the claims it bears on
3. Write the evidence note

## Inputs

The source (a clip, an attached page or file). query_blocks with kind: claim for
the claims it could support — an evidence block needs a claim's id.

## Do

Each finding becomes its own > [!evidence] callout — one finding, one block —
carrying supports:[[Note#^id]] to the claim it bears on. If there is no claim
yet, say so and propose one; do not invent an id. Caveats go in prose under the
callouts, not inside them: a callout is a thing the source shows; a caveat is a
thing I think.

## Landing

research/evidence/<author-year>.md, one note per source, linked back to the
research note so the graph connects.

## Shape

Frontmatter source: (the URL or [[clip]]); the evidence callouts; the caveats;
a closing [[link]] to the research note.

## Never

- An evidence block with no supports: tail.
- Pretending the source bears on a claim it does not — a note that supports
  nothing is a real answer.
