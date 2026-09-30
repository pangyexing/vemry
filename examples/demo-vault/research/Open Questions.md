---
title: Open Questions
tags: [research, attention]
---

# Open Questions

What [[Attention Is All You Need]] and [[Is attention explanation]] leave open for me.

- The $\sqrt{d_k}$ argument assumes independent, unit-variance components. Does it still matter when layer normalisation sits before the attention? It needs Ba et al. from the [[Reading Queue]]
- Do the eight heads end up attending at different relative offsets, or is that a story told about a few of them?
- Sinusoidal positions against rotary ones: what does rotating the queries and keys buy that adding a vector does not?

> [!claim] A heatmap of attention weights describes the computation, not the decision.
> A working position, until [[Is attention explanation]] settles more than it has. ^heatmap
