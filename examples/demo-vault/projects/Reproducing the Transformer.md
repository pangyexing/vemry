---
title: Reproducing the Transformer
tags: [project]
---

# Reproducing the Transformer

Rebuild the attention block small enough to check every number by hand, until it behaves the way [[Attention Is All You Need]] says it does; then present multi-head attention at the group meeting on Friday 2 October.

- [x] Import the source ✅ 2026-09-24
- [x] Retell chapter 1 📅 2026-09-25 ✅ 2026-09-25
- [ ] Retell chapter 2 📅 2026-09-30
- [/] Get the attention experiment running 📅 2026-10-01
- [ ] Present multi-head attention at group meeting ❗ 📅 2026-10-02
- [ ] Read chapter 3: positional encoding 📅 2026-10-06

> [!decision] Rebuild it in plain arrays first, not a framework.
> Small enough that every number can be checked by hand, which is what matters before Friday. A framework can come after.
> [date:: 2026-09-24]

## Hypothesis

- [ ] With the total width fixed, eight heads attend to more distinct positions than one [metric:: distinct positions attended per token] [expected:: more with 8 heads of 64 than 1 of 512] [status:: open] 📅 2026-10-06

The experiment in [[02 Multi-head attention]] is where this gets tested; what a head does or does not *explain* is a separate question, in [[Is attention explanation]].
