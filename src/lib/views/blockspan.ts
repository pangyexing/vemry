// Reading and rewriting one block's bytes on disk, shared by every view that edits a
// block in place (tasks, cards). Two rules live here alone:
//   - Spans are the Rust parser's UTF-8 byte offsets, so slicing goes through
//     TextEncoder/TextDecoder and never through UTF-16 string indices.
//   - A rewrite lands only while the slice still is the block the query returned. The
//     shape alone is not enough: a stale span may have shifted wholesale onto another
//     block of the same shape, so each caller's `ok` compares the slice with the
//     content the index held. Stale → null/false, and the caller requeries and lets
//     the user retry on fresh data rather than overwriting an edit made since.
import { api, type BlockHit } from "$lib/api";

/** The note's bytes and this block's span slice, or null when the span is stale. */
export async function readBlockSpan(
  hit: BlockHit,
  ok: (slice: string) => boolean,
): Promise<{ bytes: Uint8Array; slice: string } | null> {
  const { content } = await api.noteRead(hit.note);
  const bytes = new TextEncoder().encode(content);
  const { start, end } = hit.block.span;
  if (end > bytes.length) return null;
  const slice = new TextDecoder().decode(bytes.slice(start, end));
  return ok(slice) ? { bytes, slice } : null;
}

/** `bytes` with the block's span replaced by `text`, as a string ready to write. */
function spliceSpan(bytes: Uint8Array, hit: BlockHit, text: string): string {
  const dec = new TextDecoder();
  return dec.decode(bytes.slice(0, hit.block.span.start)) + text + dec.decode(bytes.slice(hit.block.span.end));
}

/** Rewrite exactly the block's span with `edit(slice)`; false when the span is stale. */
export async function editBlockSpan(
  hit: BlockHit,
  ok: (slice: string) => boolean,
  edit: (slice: string) => string,
): Promise<boolean> {
  const r = await readBlockSpan(hit, ok);
  if (!r) return false;
  await api.noteWrite(hit.note, spliceSpan(r.bytes, hit, edit(r.slice)));
  return true;
}
