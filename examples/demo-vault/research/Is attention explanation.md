---
title: Is attention explanation
tags: [research, attention]
---

# Is attention explanation?

Chapter 1's experiment draws the attention weights as a heatmap, and it is tempting to read the heatmap as the model's reasons. This thread is about whether that reading holds. The sources are one note each in `research/evidence/`, pointing back at the claims below.

## Claims

> [!claim] Attention weights often disagree with other measures of which inputs mattered ^disagree

> [!evidence] Jain & Wallace, 2019: weak agreement with gradient and leave-one-out importance ([[Jain & Wallace 2019]])
> supports:[[Is attention explanation#^disagree]]

> [!claim] Quite different attention can leave the prediction unchanged ^swap

> [!evidence] Jain & Wallace, 2019: counterfactual attention distributions ([[Jain & Wallace 2019]])
> supports:[[Is attention explanation#^swap]]

> [!claim] Whether attention explains depends on what "explanation" is asked to mean ^depends

> [!evidence] Wiegreffe & Pinter, 2019: weights swapped in after training are not the model's own ([[Wiegreffe & Pinter 2019]])
> supports:[[Is attention explanation#^depends]]

> [!claim] Some heads settle on a fixed relative offset, such as the previous token ^offsets

The last claim has nothing behind it yet: it started as a capture on the train, and [[Open Questions]] keeps it company.

## Why it matters here

The heatmap in [[01 Scaled dot-product attention]] is a picture of the computation. Whether it is also a picture of the reasons is what the claims above are about, and it is worth one sentence of caution on Friday's slides.
