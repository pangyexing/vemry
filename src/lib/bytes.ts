// UTF-8 byte offsets (what Rust counts in — `Block.span`, `block_id_plan`'s offset
// selector, a provenance record's range) against UTF-16 character positions (what
// CodeMirror and every string index in the frontend count in). Nothing here knows about
// tasks, views or the editor: the conversion is a property of the two encodings, and
// the editor, the vault store and the block-reference plumbing all need it.

/** A UTF-8 byte offset (Rust's block.span) → a UTF-16 character position (the units
 *  CodeMirror's revealPos takes). For CJK content the two differ by a factor of 2-3,
 *  and mixing them jumps to the wrong line. Clamped when out of range. */
export function byteToChar(content: string, byte: number): number {
  const bytes = new TextEncoder().encode(content);
  return new TextDecoder().decode(bytes.slice(0, Math.min(byte, bytes.length))).length;
}

/** The inverse: a UTF-16 character position → a UTF-8 byte offset (what `block_id_plan`'s
 *  offset selector and `Block.span` count in). */
export function charToByte(content: string, ch: number): number {
  return new TextEncoder().encode(content.slice(0, Math.max(0, ch))).length;
}

/** Many byte spans → character ranges in one pass over the text (byteToChar per span
 *  would re-encode the whole document for each block). Offsets past the end clamp. */
export function byteSpansToChar(content: string, spans: { start: number; end: number }[]): { from: number; to: number }[] {
  const wanted = Array.from(new Set(spans.flatMap((s) => [s.start, s.end]))).sort((a, b) => a - b);
  const map = new Map<number, number>();
  let byte = 0;
  let wi = 0;
  let i = 0;
  while (wi < wanted.length && i <= content.length) {
    while (wi < wanted.length && wanted[wi] <= byte) map.set(wanted[wi++], i);
    if (i === content.length) break;
    const cp = content.codePointAt(i)!;
    byte += cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
    i += cp >= 0x10000 ? 2 : 1;
  }
  while (wi < wanted.length) map.set(wanted[wi++], content.length);
  return spans.map((s) => ({ from: map.get(s.start) ?? content.length, to: map.get(s.end) ?? content.length }));
}
