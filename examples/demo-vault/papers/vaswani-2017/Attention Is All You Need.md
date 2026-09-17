---
title: Attention Is All You Need
goal: understand
status: reading
tags: [paper]
---

# Attention Is All You Need

Vaswani et al., 2017. The source is [[papers/vaswani-2017/source]], one paragraph per block; each chapter embeds the paragraphs it rests on and explains one idea. Reading it to understand the mechanism, not to reproduce it.

## Known

- Nothing in this vault yet — this is the first paper here, so the chapters start from zero

## To learn

- Why the scores are divided by $\sqrt{d_k}$
- What a head is, and why eight of them
- How a model with no recurrence knows where a token stands

## Chapters

```vemry-query
tag: chapter
text: "#chapter"
```

## Experiments to add

- [[02 Multi-head attention]] — move the number of heads with the total width fixed; watch what each head's attention picks out on three tokens
