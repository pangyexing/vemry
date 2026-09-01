// localStorage persistence bucketed per vault (a root's absolute path → that vault's
// own bucket). Shared by vemry-tree (the file tree's expanded set) and
// vemry-clip-target (the pinned clip destination): the parse fallback, the write
// through and the size trimming are maintained once. The restore ordering (on the
// first beat of a vault switch, restore before writing through, or the old vault's
// leftovers would wipe the new vault's bucket) is each caller's to hold to its own
// structure — the two stores work differently (a guard inside an effect versus one
// batch of assignments in openVault), so they are not forced together.

/** Read the whole bucket table; bad data falls back to an empty one. */
export function loadBucketMap<T>(key: string): Record<string, T> {
  try {
    return JSON.parse(localStorage.getItem(key) ?? "{}") as Record<string, T>;
  } catch {
    return {};
  }
}

/** Write through the current vault's bucket (value=null deletes it) and trim the
 *  stale buckets outside keep(recent).
 *  keep=null means recent has not loaded yet (a startup race, a failed IPC) — the
 *  trimming is skipped then, or an empty keep would wrongly delete every other
 *  vault's bucket in one go. */
export function saveBucket<T>(key: string, root: string, value: T | null, keep: readonly string[] | null) {
  const all = loadBucketMap<T>(key);
  if (value != null) all[root] = value;
  else delete all[root];
  if (keep) for (const r of Object.keys(all)) if (r !== root && !keep.includes(r)) delete all[r];
  try {
    localStorage.setItem(key, JSON.stringify(all));
  } catch {
    /* storage unavailable */
  }
}
