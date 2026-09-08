---
title: Attention Is All You Need
authors: Vaswani et al.
year: 2017
source: https://arxiv.org/abs/1706.03762
genre: method
---

# Attention Is All You Need

An excerpt, not the paper: two paragraphs of section 3.2.1, kept in the paper's
words so the chapters can embed them. A real study imports the whole paper,
section by section.

## 3.2.1 Scaled Dot-Product Attention

The input consists of queries and keys of dimension $d_k$, and values of dimension $d_v$. We compute the dot products of the query with all keys, divide each by $\sqrt{d_k}$, and apply a softmax function to obtain the weights on the values. ^s3p1

$$
\mathrm{Attention}(Q, K, V) = \mathrm{softmax}\left(\frac{QK^T}{\sqrt{d_k}}\right) V
$$

We suspect that for large values of $d_k$, the dot products grow large in magnitude, pushing the softmax function into regions where it has extremely small gradients. To counteract this effect, we scale the dot products by $\frac{1}{\sqrt{d_k}}$. ^s3p2
