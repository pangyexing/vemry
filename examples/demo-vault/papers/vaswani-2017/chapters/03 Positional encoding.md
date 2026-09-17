# Positional encoding
#chapter [status:: unread]

## In one sentence

With no recurrence, nothing in the model knows where a token stands, so each position gets a fixed pattern of sines and cosines — one frequency per pair of dimensions — added onto its embedding.

## From the paper

![[papers/vaswani-2017/source#^s35p1]]

![[papers/vaswani-2017/source#^s35p3]]

![[papers/vaswani-2017/source#^s35p4]]

## Notation

| symbol | meaning |
|---|---|
| $pos$ | the token's position in the sequence, counted from 0 |
| $i$ | the index of a dimension pair; dimensions $2i$ and $2i+1$ share one frequency |

## Try it

```js
// Drag d_model: the first dimensions keep their short wavelength while the last ones
// slow down until they barely move across the positions shown. Each row of the
// heatmap is one position's whole encoding.
const { d_model, positions } = vemry.lab.controls({
  d_model: { slider: [8, 64], step: 8, value: 16, label: "d_model" },
  positions: { slider: [8, 64], step: 8, value: 32, label: "positions shown" },
  try: { presets: { "narrow model": { d_model: 8, positions: 32 }, "wide model": { d_model: 64, positions: 64 } } },
});
const pe = (pos, dim) => {
  const angle = pos / Math.pow(10000, (2 * Math.floor(dim / 2)) / d_model);
  return dim % 2 === 0 ? Math.sin(angle) : Math.cos(angle);
};
const pos = Array.from({ length: positions }, (_, p) => p);
const dims = [0, 1, d_model - 2, d_model - 1];
vemry.lab.line(
  "the first two and the last two dimensions, across positions",
  dims.map((d) => ({ name: `dim ${d}`, y: pos.map((p) => pe(p, d)) })),
  { x: pos, xLabel: "position", yLabel: "PE", yMin: -1, yMax: 1 },
);
vemry.lab.heatmap(
  "PE, one row per position",
  pos.map((p) => Array.from({ length: d_model }, (_, d) => pe(p, d))),
  { rows: pos.map(String), cols: Array.from({ length: d_model }, (_, d) => String(d)), digits: 2, diverging: true },
);
```

## Where it stops holding

The linear-function property is about a fixed offset $k$, not about the model using it: the paper says it *hypothesized* the encoding would let the model attend by relative position, and reports that learned embeddings did about as well.

## Question

Why does one frequency need two dimensions, a sine and a cosine, rather than one?

## In my words

