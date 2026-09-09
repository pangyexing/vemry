// Writing one inline field of a block on disk from a projection (a query row's chip),
// span-accurate: the index holds the field's own byte span (`Field.span`, the whole
// `[key:: value]` construct), so the rewrite lands on exactly that construct and
// nothing else of the block. The guard is the same discipline as blockspan.ts: the
// bytes at the span still read as that field with that value, else the note changed
// since the query and nothing is written — the caller requeries and the user retries
// on fresh data. Any block kind: a task's `[status:: x]` and a paragraph's alike (a
// task's due/scheduled dates are not fields to this module; tasks.ts owns those).
import { api, type BlockHit } from "$lib/api";
import { cutSpan, scanFields, setFieldValue } from "$lib/fields";

/** Set (or with null remove) the value of `key` on the block: the field's first
 *  occurrence, at its indexed span. false = a stale span (or the key is not on the
 *  block), and the caller requeries. */
export async function writeBlockField(hit: BlockHit, key: string, value: string | null): Promise<boolean> {
  const lower = key.toLowerCase();
  const field = hit.block.fields?.find((f) => f.key.toLowerCase() === lower);
  if (!field) return false;
  const { content } = await api.noteRead(hit.note);
  const bytes = new TextEncoder().encode(content);
  const { start, end } = field.span;
  if (end > bytes.length) return false;
  const dec = new TextDecoder();
  const construct = dec.decode(bytes.slice(start, end));
  const mark = scanFields(construct)[0];
  if (!mark || mark.key.toLowerCase() !== lower || mark.value !== field.value || mark.from !== 0 || mark.to !== construct.length) return false;
  const before = dec.decode(bytes.slice(0, start));
  const piece = setFieldValue(construct, key, value);
  const after = dec.decode(bytes.slice(end));
  if (piece === construct) return true;
  // a removed bracket field takes the space before it along, so no double space is left
  const out = value === null && piece === "" ? cutSpan(before + after, before.length, before.length) : before + piece + after;
  await api.noteWrite(hit.note, out);
  return true;
}
