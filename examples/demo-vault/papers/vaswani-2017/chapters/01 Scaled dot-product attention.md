# Scaled dot-product attention
#chapter [status:: reading]

## In one sentence

Every query scores every key with a dot product, the scores are shrunk by
$\sqrt{d_k}$ so the softmax stays soft, and the resulting weights mix the values.

## From the paper

![[papers/vaswani-2017/source#^s3p1]]

![[papers/vaswani-2017/source#^s3p2]]

## Notation

| symbol | meaning |
|---|---|
| $Q, K, V$ | the query, key and value matrices, one row per token |
| $d_k$ | the width of a query or key vector |

## Try it

Press Run, then drag $d_k$ with the scale off: the weights collapse onto one key
as the scores grow. Switch the scale on and they stay spread. Edit a cell of $Q$
or $K$ to move the attention.

```js
// Scaled dot-product attention on three tokens. The dot products are scaled up
// as if d_k dimensions contributed like the four shown, so d_k can be dragged.
const { d_k, scale } = vemry.lab.controls({
  d_k: { slider: [4, 512], step: 4, value: 64, label: "d_k" },
  scale: { toggle: true, label: "divide by √d_k" },
});
const tokens = ["The", "cat", "sat"];
const Q = vemry.lab.matrix("Q", [[1, 0, 1, 0], [0, 1, 0, 1], [1, 1, 0, 0]]);
const K = vemry.lab.matrix("K", [[1, 0, 0, 1], [0, 1, 1, 0], [1, 0, 1, 0]]);
const dot = (a, b) => a.reduce((s, v, i) => s + v * b[i], 0);
const raw = Q.map((q) => K.map((k) => (dot(q, k) * d_k) / 4));
const scores = raw.map((r) => r.map((v) => (scale ? v / Math.sqrt(d_k) : v)));
const softmax = (r) => {
  const m = Math.max(...r);
  const e = r.map((v) => Math.exp(v - m));
  const z = e.reduce((a, b) => a + b, 0);
  return e.map((v) => v / z);
};
vemry.lab.heatmap("scores", scores, { rows: tokens, cols: tokens, digits: 1 });
vemry.lab.heatmap("attention weights", scores.map(softmax), { rows: tokens, cols: tokens, digits: 2, min: 0, max: 1 });
vemry.lab.step("the first row, step by step", [
  { label: "dot products", text: "$q_1 \\cdot k_j$ for every key", value: raw[0] },
  { label: scale ? "divide by √d_k" : "no scaling", text: scale ? "$s_j / \\sqrt{d_k}$" : "the scores as they are", value: scores[0] },
  { label: "softmax", text: "$e^{s_j} / \\sum_j e^{s_j}$", value: softmax(scores[0]) },
]);
```

## Where it stops holding

The $\sqrt{d_k}$ argument assumes the components of $q$ and $k$ are independent
with unit variance. With a learned temperature the exact constant matters less;
the point that survives is that the softmax saturates when the scores grow.

## Question

What happens to the gradient of the softmax when $d_k$ grows and the scores are
not scaled?

## In my words

