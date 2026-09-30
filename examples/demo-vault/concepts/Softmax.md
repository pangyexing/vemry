---
title: Softmax
tags: [concept]
---

# Softmax

Turns a row of scores into weights that are positive and sum to one:

$$\mathrm{softmax}(s)_j = \frac{e^{s_j}}{\sum_i e^{s_i}}$$

From last term's course notes.

- Adding the same number to every score changes nothing; only the differences count
- Scale the scores up and the output hardens toward a single one; scale them down and it flattens toward uniform
- Once it has hardened, its gradients are tiny, which is the problem [[01 Scaled dot-product attention]] solves
