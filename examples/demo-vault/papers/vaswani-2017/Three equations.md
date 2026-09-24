---
title: Three equations — Attention Is All You Need
tags: [paper]
---

# Three equations

The whole of [[Attention Is All You Need]] rests on three equations. This page puts them side by side, each with the source paragraph around it and a line in my own words, so a review is one scroll instead of three chapters.

## Cards still to review

The cards the retellings left behind, the soonest due first:

```vemry-query
tag: card
sort: due
```

## Scaled dot-product attention

![[papers/vaswani-2017/source#^s321p1]]

![[papers/vaswani-2017/source#^s321p2]]

In my words: every query scores every key, the scores are shrunk by $\sqrt{d_k}$, and the softmax of the scores weighs the values. See [[01 Scaled dot-product attention]].

![[papers/vaswani-2017/source#^s321p3]]

The scale is not about overflow: without it the softmax saturates and its gradients vanish.

## Multi-head attention

![[papers/vaswani-2017/source#^s322p1]]

![[papers/vaswani-2017/source#^s322p3]]

In my words: the same attention run eight times on narrower projections, the eight answers joined and projected back. See [[02 Multi-head attention]].

![[papers/vaswani-2017/source#^s322p4]]

Eight heads of width 64 cost about what one head of width 512 would.

## Positional encoding

![[papers/vaswani-2017/source#^s35p1]]

![[papers/vaswani-2017/source#^s35p3]]

In my words: each position gets a vector of sines and cosines at wavelengths from $2\pi$ to $10000 \cdot 2\pi$, added to the token's embedding. See [[03 Positional encoding]].

![[papers/vaswani-2017/source#^s35p4]]

Still open: why a fixed offset being a linear map should make relative positions easy to learn.
