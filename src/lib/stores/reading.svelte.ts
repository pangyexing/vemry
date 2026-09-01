// The reading projection's state (v0.5 P2). The projection keeps its own history and
// does not mix into the document navStack — following v0.4's position that "browsing
// history belongs to the webview itself": the projection is a projection of browsing,
// and is independent for the same reason.
// Switching mainView is the caller's job (App's event wiring, or ReadingView), and
// this never touches the vault store, which avoids a module cycle.
import { api, trimLoneSurrogate, type PageContext, type ReadPage } from "$lib/api";
import { clips } from "$lib/stores/clips.svelte";
import { domainOf } from "$lib/markdown";

const HISTORY_CAP = 20;
/** The injection limit of "add to context" (the same number as PAGE_CONTEXT_MAX_CHARS on the Rust side). */
const CTX_MAX = 20_000;

class ReadingStore {
  current = $state<ReadPage | null>(null);
  /** The URL being fetched; clicking citations in quick succession only honours the last (the same semantics as generation on the Rust side). */
  loading = $state<string | null>(null);
  error = $state<{ url: string; message: string } | null>(null);

  /** The projection's page history (with the content cached): back and forward refetch nothing. */
  private stack = $state<ReadPage[]>([]);
  private index = $state(-1);
  /** URL → scroll position, stored and read as ReadingView leaves and returns to a page. */
  private scroll = new Map<string, number>();

  get canGoBack(): boolean {
    return this.index > 0;
  }
  get canGoForward(): boolean {
    return this.index >= 0 && this.index < this.stack.length - 1;
  }

  /** Open the projection. Opening a new page from the middle of the history truncates the forward side (browser semantics). */
  async open(url: string) {
    this.error = null;
    this.loading = url;
    try {
      const page = await api.pageRead(url);
      if (this.loading !== url) return; // superseded by a newer click, ignored silently
      // null = this request was superseded by generation on the Rust side. The order in
      // which two requests complete is not guaranteed: a later click may take its number
      // first and be superseded by the earlier request, in which case no reply will come
      // to clear loading — so it is put down right here rather than leaving the spinner
      // turning
      this.loading = null;
      if (page) this.push(page);
    } catch (e) {
      if (this.loading !== url) return;
      this.loading = null;
      this.error = { url, message: String(e) };
    }
  }

  /** Assembling the PageContext of "add to context" (purely in the frontend: the
   *  projection's body is already in hand).
   *  ReadingView's button and the @ picker share this one. The truncation counts UTF-16
   *  units and may cut through a surrogate pair (see trimLoneSurrogate in api.ts). */
  pageContext(): PageContext | null {
    const cur = this.current;
    if (!cur) return null;
    const truncated = cur.markdown.length > CTX_MAX;
    const markdown = truncated ? trimLoneSurrogate(cur.markdown.slice(0, CTX_MAX)) : cur.markdown;
    return { title: cur.title || domainOf(cur.url), url: cur.url, markdown, truncated };
  }

  /** Retry the current error page (another way out besides the escape hatch). */
  retry() {
    if (this.error) void this.open(this.error.url);
  }

  private push(page: ReadPage) {
    const stack = this.stack.slice(0, this.index + 1);
    if (stack[stack.length - 1]?.url === page.url) stack[stack.length - 1] = page;
    else stack.push(page);
    while (stack.length > HISTORY_CAP) stack.shift();
    this.stack = stack;
    this.index = stack.length - 1;
    this.current = page;
    void clips.lookup([page.url]);
  }

  back() {
    if (!this.canGoBack) return;
    this.loading = null;
    this.error = null;
    this.index--;
    this.current = this.stack[this.index];
  }

  forward() {
    if (!this.canGoForward) return;
    this.loading = null;
    this.error = null;
    this.index++;
    this.current = this.stack[this.index];
  }

  scrollFor(url: string): number {
    return this.scroll.get(url) ?? 0;
  }

  saveScroll(url: string, pos: number) {
    this.scroll.set(url, pos);
  }
}

export const reading = new ReadingStore();
