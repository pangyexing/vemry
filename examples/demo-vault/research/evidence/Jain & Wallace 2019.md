---
title: Jain & Wallace 2019
source: https://arxiv.org/abs/1902.10186
tags: [attention, reading]
---

# Jain & Wallace 2019

*Attention is not Explanation*, NAACL 2019. Models with attention over mostly recurrent (BiLSTM) encoders, on text classification, question answering and natural language inference.

> [!evidence] Attention weights correlate only weakly with gradient-based and leave-one-out measures of feature importance
> supports:[[Is attention explanation#^disagree]]

> [!evidence] Attention distributions constructed to differ from the learned ones often leave the prediction nearly unchanged
> supports:[[Is attention explanation#^swap]]

The models are recurrent, not Transformers: how far this carries to multi-head self-attention is not settled here. Filed against [[Open Questions]].
