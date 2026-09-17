# Multi-head attention
#chapter [status:: reading]

## In one sentence

Rather than one attention over the full width, the model runs eight narrow attentions side by side, each with its own projections of $Q$, $K$ and $V$, and stitches their outputs back together with one more projection.

## From the paper

![[papers/vaswani-2017/source#^s322p1]]

![[papers/vaswani-2017/source#^s322p2]]

![[papers/vaswani-2017/source#^s322p3]]

![[papers/vaswani-2017/source#^s322p4]]

## Notation

| symbol | meaning |
|---|---|
| $h$ | the number of heads, 8 in the paper |
| $W_i^Q, W_i^K, W_i^V$ | head $i$'s projections of the queries, keys and values, from $d_{model}$ down to $d_k$ or $d_v$ |
| $W^O$ | the projection that maps the concatenated heads back to $d_{model}$ |

## Where it stops holding

The cost argument holds only because $d_k = d_{model}/h$: eight heads at the full width would be eight times the work. And "different subspaces" is a hope the learned projections are free to fulfil, not a constraint the architecture puts on them.

## Question

Why does averaging with a single head "inhibit" attending to several positions at once, and what do the separate projections change about that?

## In my words

