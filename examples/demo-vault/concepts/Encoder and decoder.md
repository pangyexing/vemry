---
title: Encoder and decoder
tags: [concept]
---

# Encoder and decoder

A sequence-to-sequence model reads the input into a representation (the encoder) and writes the output from that representation one token at a time (the decoder). Before [[Attention Is All You Need]] both halves were usually recurrent, with attention as the bridge between them; the paper keeps the two halves and the bridge, and drops the recurrence.
