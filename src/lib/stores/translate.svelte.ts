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
  /** The segment the background translation stopped before (its budget spent); null = not stopped. */
  pausedAt = $state<number | null>(null);
  /** The segments the view asked for (in and near the viewport): the ones whose placeholder shimmers. */
  requested = $state<Record<number, true>>({});
  /** A save in progress: first translating what is left, then writing. */
  saving = $state(false);
  /** Tokens spent on this document so far (from the provider's usage counts; 0 where none are reported). */
  usage = $state({ prompt: 0, completion: 0 });
  private listening = false;

  /** The segments the model has to translate (the count line's denominator). Derived
   *  rather than a getter: the counters below and the view read it on every streamed
   *  token, and a document runs to thousands of segments. */
  toTranslate: TranslateSegment[] = $derived((this.doc?.segments ?? []).filter((s) => s.translate && !s.private));
  /** Derived, not getters: the view reads all three on every render, and a render
   *  happens on every streamed token. */
  doneCount: number = $derived(this.toTranslate.filter((s) => this.rows[s.id]?.done).length);
  failedIds: number[] = $derived(this.toTranslate.filter((s) => this.rows[s.id]?.error).map((s) => s.id));

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
      this.pausedAt = null;
      this.requested = {};
      this.usage = { prompt: 0, completion: 0 };
      // Everything already in the cache: the job had nothing to send and went idle
      // before the document arrived here, so that event is not coming
      this.idle = doc.segments.every((s) => !s.translate || s.private || s.cached != null);
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
    for (const id of ids) this.requested[id] = true;
    this.idle = false;
    void api.translateRequest(job, ids, force).catch((e) => (this.error = String(e)));
  }

  /** Past the stop: another budget's worth of the document, or all of it. */
  continueBackground(all: boolean) {
    const job = this.doc?.job;
    if (!job) return;
    this.pausedAt = null;
    this.idle = false;
    void api.translateContinue(job, all).catch((e) => (this.error = String(e)));
  }

  /** Save as a note: what is left is translated first (the engine does it), then the note lands. */
  async save(): Promise<string> {
    const job = this.doc?.job;
    if (!job) throw new Error("no document");
    this.saving = true;
    this.pausedAt = null;
    try {
      return await api.translateSave(job);
    } finally {
      this.saving = false;
    }
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
      } else if (e.type === "paused") {
        this.pausedAt = e.at;
      } else if (e.type === "usage") {
        this.usage = { prompt: e.prompt, completion: e.completion };
      } else if (e.type === "idle") {
        this.idle = true;
      }
    });
  }
}

function sameSource(a: TranslateSource, b: TranslateSource): boolean {
  if (a.kind === "note" && b.kind === "note") return a.path === b.path;
  if (a.kind === "page" && b.kind === "page") return a.url === b.url;
  if (a.kind === "pdf" && b.kind === "pdf") return a.path === b.path;
  return false;
}

/** The label of a target language tag: the catalog's name for the ones offered, the tag itself for a custom one. */
export function targetLabel(tag: string): string {
  const key = `translate.lang.${tag}` as MessageKey;
  const s = t(key);
  return s === key ? tag : s;
}

/** A token count in thousands past 1000 ("12.3k", "45k"), the same shape as the chat's gauge. */
export function fmtTokens(n: number): string {
  return n >= 10_000 ? `${Math.round(n / 1000)}k` : n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

/** The target languages the header offers; a custom tag typed in Settings is shown in addition. */
export const TARGET_TAGS = ["zh-CN", "zh-TW", "en", "ja", "ko"] as const;

export const translate = new TranslateStore();
