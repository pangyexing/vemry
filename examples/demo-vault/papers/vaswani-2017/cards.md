---
title: Cards — Attention Is All You Need
tags: [paper, cards]
---

# Cards — Attention Is All You Need

From the retelling of [[01 Scaled dot-product attention]]: one card per gap the check found. The Review view schedules them.

> [!card] Why are the dot products divided by $\sqrt{d_k}$ before the softmax?
> Not to keep the numbers from overflowing: large scores push the softmax into a region where its gradients are tiny, and the scale keeps the weights soft (from
> [[papers/vaswani-2017/source#^s321p3]]).
> [due:: 2026-09-16] #card

> [!card] The softmax weights are applied to which of $Q$, $K$ and $V$?
> The values: the output is the weighted sum of the rows of $V$; the keys only decide the weights (from [[papers/vaswani-2017/source#^s321p1]]).
> [due:: 2026-09-17] #card
