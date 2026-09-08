// The side-by-side translation's state: the document cut into segments, the
// translation of each as it streams in, and the job on the Rust side that is
// sending. The view renders rows from it; switching mainView is the caller's
// job (vault.enterTranslate / exitTranslate), the same division of labour as the
// reading store — this never imports vault, which avoids a module cycle.
import { api, onTranslateEvent, type TranslateDocView, type TranslateSegment, type TranslateSource } from "$lib/api";
import { t, type MessageKey } from "$lib/i18n/i18n.svelte";

/** How the two texts are laid out. Presentational, kept in localStorage. */
export type TranslateMode = "side" | "stack" | "target";
const MODE_KEY = "vemry-translate-mode";

export interface RowState {
  /** The translation so far (final when `done`); null while nothing has arrived. */
  text: string | null;
  done: boolean;
  /** The provider's message, or "no translation returned", when the segment failed. */
  error: string | null;
}

function loadMode(): TranslateMode {
  try {
    const v = localStorage.getItem(MODE_KEY);
    if (v === "side" || v === "stack" || v === "target") return v;
  } catch {
    // no storage: the default
  }
  return "side";
}

class TranslateStore {
  doc = $state<TranslateDocView | null>(null);
  /** The source being cut and sent, before the document arrives. */
  loading = $state(false);
  error = $state<string | null>(null);
  rows = $state<Record<number, RowState>>({});
  /** Whether the job has nothing pending and nothing in flight. */
  idle = $state(false);
  mode = $state<TranslateMode>(loadMode());
  /** The row the reader clicked: lit on both sides. */
  selected = $state<number | null>(null);
  private listening = false;

  /** The segments the model has to translate (the count line's denominator). */
  get toTranslate(): TranslateSegment[] {
    return (this.doc?.segments ?? []).filter((s) => s.translate && !s.private);
  }
  get doneCount(): number {
    return this.toTranslate.filter((s) => this.rows[s.id]?.done).length;
  }
  get failedIds(): number[] {
    return this.toTranslate.filter((s) => this.rows[s.id]?.error).map((s) => s.id);
  }

  setMode(mode: TranslateMode) {
    this.mode = mode;
    try {
      localStorage.setItem(MODE_KEY, mode);
    } catch {
      // no storage: the choice lives for the session
    }
  }

  /** Cut and start translating a source. A source already open is left as it is. */
  async open(source: TranslateSource): Promise<boolean> {
    if (this.doc && sameSource(this.doc.source, source)) return true;
    await this.listen();
    this.loading = true;
    this.error = null;
    try {
      const doc = await api.translateOpen(source);
      const rows: Record<number, RowState> = {};
      for (const s of doc.segments) {
        if (s.cached != null) rows[s.id] = { text: s.cached, done: true, error: null };
      }
      this.rows = rows;
      this.doc = doc;
      this.idle = false;
      this.selected = null;
      return true;
    } catch (e) {
      this.error = String(e);
      return false;
    } finally {
      this.loading = false;
    }
  }

  /** Leave: the job stops streaming; what landed stays in the Rust cache. */
  close() {
    const job = this.doc?.job;
    this.doc = null;
    this.rows = {};
    this.error = null;
    this.selected = null;
    if (job) void api.translateCancel(job).catch(() => {});
  }

  /** Reopen the same source under the current settings (a changed target language). */
  async reopen() {
    const source = this.doc?.source;
    if (!source) return;
    this.close();
    await this.open(source);
  }

  /** The visible rows first: the view reports what is on screen as it scrolls. */
  request(ids: number[], force = false) {
    const job = this.doc?.job;
    if (!job || !ids.length) return;
    if (force) {
      for (const id of ids) this.rows[id] = { text: null, done: false, error: null };
    }
    this.idle = false;
    void api.translateRequest(job, ids, force).catch((e) => (this.error = String(e)));
  }

  retryFailed() {
    const ids = this.failedIds;
    for (const id of ids) this.rows[id] = { text: null, done: false, error: null };
    this.request(ids);
  }

  /** The translation of every segment, in document order, as plain Markdown (the copy-all text). */
  allText(): string {
    const doc = this.doc;
    if (!doc) return "";
    const parts: string[] = [];
    for (const s of doc.segments) {
      if (s.private) continue;
      const body = s.translate ? (this.rows[s.id]?.done ? this.rows[s.id].text : null) : s.text;
      if (body == null) continue;
      parts.push(s.prefix + body);
    }
    return parts.join("\n\n");
  }

  private async listen() {
    if (this.listening) return;
    this.listening = true;
    await onTranslateEvent((e) => {
      if (!this.doc || e.job !== this.doc.job) return;
      if (e.type === "segment") {
        this.rows[e.id] = { text: e.text, done: e.done, error: null };
      } else if (e.type === "failed") {
        for (const id of e.ids) this.rows[id] = { text: null, done: false, error: e.message };
      } else if (e.type === "idle") {
        this.idle = true;
      }
    });
  }
}

function sameSource(a: TranslateSource, b: TranslateSource): boolean {
  return a.kind === b.kind && a.path === b.path;
}

/** The label of a target language tag: the catalog's name for the ones offered, the tag itself for a custom one. */
export function targetLabel(tag: string): string {
  const key = `translate.lang.${tag}` as MessageKey;
  const s = t(key);
  return s === key ? tag : s;
}

/** The target languages the header offers; a custom tag typed in Settings is shown in addition. */
export const TARGET_TAGS = ["zh-CN", "zh-TW", "en", "ja", "ko"] as const;

export const translate = new TranslateStore();
