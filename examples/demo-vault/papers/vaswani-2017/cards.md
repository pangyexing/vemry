---
title: Cards — Attention Is All You Need
tags: [paper, cards]
---

# Cards — Attention Is All You Need

From the retelling of [[01 Scaled dot-product attention]] on 25 September, one card per gap the check found, and one from chapter 2. The Review view schedules them.

> [!card] Why divide the dot products by $\sqrt{d_k}$ and not by $d_k$?
> A dot product of $d_k$ independent, unit-variance components has variance $d_k$; dividing by $\sqrt{d_k}$ brings the variance back to 1, so the softmax does not saturate (from [[papers/vaswani-2017/source#^s321p3]]).
> [due:: 2026-09-30] [interval:: 3] [ease:: 2.5] [reviews:: 2] #card

> [!card] Why do the dot products grow with $d_k$ in the first place?
> Each is a sum of $d_k$ products of independent components with mean 0 and variance 1, so its variance is $d_k$ (from [[papers/vaswani-2017/source#^s321fn4]]).
> [due:: 2026-09-30] [interval:: 3] [ease:: 2.5] [reviews:: 2] #card

> [!card] In multi-head attention, how many dimensions does each head get?
> $d_{model}/h$: 64 when $d_{model} = 512$ and $h = 8$, which is why eight heads cost about what one full-width head would (from [[papers/vaswani-2017/source#^s322p4]]).
> [due:: 2026-10-02] #card
