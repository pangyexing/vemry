<script lang="ts">
  // The quick capture composer — the whole surface of the capture window: a text area,
  // a strip of attachments, and a foot with the landing spot and the keys.
  //   Enter lands · ⇧Enter is a newline · ⌘Enter picks a note for this one capture · Esc hides.
  // Landing is optimistic: Enter hides the window and empties the composer at once — the
  // landing spot was already written in the foot, so a success has nothing to add and the
  // hand moves on without waiting — and the work runs behind it: the attachments are saved
  // into the target note's bucket (this window has vault IPC), then the text with their
  // paths is relayed to the main window, the only place that can see an unsaved editor
  // buffer (CaptureWindow.svelte). Only a failure comes back: the window reappears with the
  // text, the attachments and the reason under the foot — a thought is never lost silently,
  // and ⌘Enter can try another note. Each landing carries an id so that a second capture
  // typed while the first is still in flight gets its own answer.
  // Attachments arrive as bytes (a screenshot on the clipboard, a DOM paste) or as paths (a
  // file manager's copy, a drop) and are only written at landing time — the destination
  // bucket depends on the note chosen then. A saved attachment remembers its bucket, so a
  // retry after a failure does not write it twice.
  // The note picker is drawn in here (pickNote.ts holds the contract it shares with the
  // palette and the clip toolbar): the first stage lists notes and a "create" row, Tab on
  // a note enters the second stage's sections; the pick is for this capture alone and
  // pins nothing — the standing target is a setting.
  import { onMount, tick } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { api, onCaptureResult, type CaptureAction, type CaptureResult } from "$lib/api";
  import { t } from "$lib/i18n/i18n.svelte";
  import { k } from "$lib/shortcut";
  import { isImeKeydown } from "$lib/ime";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { fileKind, noteBucketOf, pastedName, stampName } from "$lib/stores/vault.svelte";
  import { fuzzyFilter } from "$lib/commands/registry";
  import { fetchHeadings, noteMatches, resolveOrCreateNote, type PickHeading } from "$lib/pickNote";
  import { isFileLikePaste } from "$lib/editor/setup";
  import { defaultCaptureSpot, hotkeyLabel, spotLabel, type CaptureSpot } from "$lib/capture";

  interface Shot {
    id: number;
    /** The file name it lands under (a stamp for bytes, the original name for a path). */
    name: string;
    bytes?: Uint8Array<ArrayBuffer>;
    path?: string;
    /** A preview for bytes (an object URL, revoked on removal); a path outside the vault has none and shows its name. */
    url: string | null;
    /** Written already, into this bucket: reused by a retry that lands in the same note. */
    saved?: { bucket: string; rel: string };
  }
  type Row = { kind: "note"; path: string; title: string } | { kind: "create"; title: string } | { kind: "heading"; text: string | null; level: number };

  /** How many presses show the hint line under the foot before it is retired. */
  const HINT_SHOWS = 3;
  const HINT_KEY = "vemry-capture-hints";
  /** The main window's answer normally takes milliseconds; past this it is reported rather than waited for forever. */
  const REPLY_TIMEOUT_MS = 15000;
  const MAX_ROWS = 8;

  let text = $state("");
  let shots = $state<Shot[]>([]);
  let nextShot = 1;
  /** ⌘Enter's choice for this capture (cleared once it lands). */
  let picked = $state<CaptureSpot | null>(null);
  let stage = $state<"compose" | "notes" | "headings">("compose");
  let query = $state("");
  let selected = $state(0);
  /** The note whose sections the second stage lists. */
  let pickNote = $state<{ path: string; title: string } | null>(null);
  let headings = $state<PickHeading[]>([]);
  let error = $state<string | null>(null);
  let dragging = $state(false);
  let hintShows = $state(readHintShows());
  let textarea = $state<HTMLTextAreaElement | null>(null);
  let pickInput = $state<HTMLInputElement | null>(null);
  let box = $state<HTMLDivElement | null>(null);

  /** Bumped on every show: `defaultCaptureSpot` reads the clock for the journal target (not
   *  reactive) and the settings (an equal value re-read does not re-signal), so the spot is
   *  taken afresh per press rather than once for the session the hidden window lives. */
  let shows = $state(0);
  const spot = $derived.by(() => {
    void shows;
    return picked ?? defaultCaptureSpot();
  });
  const hotkey = $derived(hotkeyLabel(prefs.captureHotkey));

  const rows = $derived.by((): Row[] => {
    const q = query.trim();
    if (stage === "headings") {
      const all: Row[] = [{ kind: "heading", text: null, level: 0 }, ...headings.map((h) => ({ kind: "heading" as const, text: h.text, level: h.level }))];
      return q ? fuzzyFilter(all, q, (r) => (r.kind === "heading" && r.text) || "") : all;
    }
    const { notes, create } = noteMatches(q);
    const out: Row[] = notes.slice(0, MAX_ROWS).map((n) => ({ kind: "note", path: n.id, title: n.title }));
    if (create) out.push({ kind: "create", title: create });
    return out;
  });

  // ---- the window follows the content: every change of the box's height is reported to Rust ----
  onMount(() => {
    const subs = [onCaptureResult((r) => pending.get(r.id)?.(r)), getCurrentWebview().onDragDropEvent((e) => onDrag(e.payload))];
    const ro = new ResizeObserver(() => {
      if (box) void api.captureWindowResize(Math.ceil(box.getBoundingClientRect().height)).catch(() => undefined);
    });
    if (box) ro.observe(box);
    return () => {
      ro.disconnect();
      for (const s of subs) void s.then((un) => un());
    };
  });

  /** CaptureWindow calls this on every capture://shown: the text takes the focus at once.
   *  An error is left standing (it belongs to the text still in the composer) and clears
   *  with the next keystroke or landing. */
  export function shown() {
    // Counted up to one past the limit, where the hint retires (the template shows it while the count is within the limit)
    if (hintShows <= HINT_SHOWS) writeHintShows(++hintShows);
    shows++;
    void tick().then(() => (stage === "compose" ? textarea : pickInput)?.focus());
  }

  function readHintShows(): number {
    try {
      return Number(localStorage.getItem(HINT_KEY) ?? 0) || 0;
    } catch {
      return HINT_SHOWS + 1; // no storage: no hint rather than a hint forever
    }
  }
  function writeHintShows(n: number) {
    try {
      localStorage.setItem(HINT_KEY, String(n));
    } catch {
      /* storage unavailable */
    }
  }

  // ---- the text ----
  /** The modifiers of the last keydown, for the function-key backstop below (a beforeinput carries none). */
  let lastMods = { shift: false, alt: false, meta: false };
  function onTextKey(e: KeyboardEvent) {
    lastMods = { shift: e.shiftKey, alt: e.altKey, meta: e.metaKey };
    if (isImeKeydown(e)) return;
    if (e.key === "Escape") {
      e.preventDefault();
      void dismiss();
    } else if (e.key === "Enter" && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      if (e.metaKey || e.ctrlKey) void openPicker();
      else void land();
    }
  }
  /** One line to eight, then the text scrolls (the window's height follows through the ResizeObserver). */
  function grow() {
    const ta = textarea;
    if (!ta) return;
    ta.style.height = "auto";
    const line = parseFloat(getComputedStyle(ta).lineHeight) || 24;
    ta.style.height = `${Math.min(ta.scrollHeight, line * 8)}px`;
  }
  function onInput() {
    error = null;
    grow();
  }

  // ---- the function-key backstop ----
  // On macOS the panel takes the keyboard without activating the application, and
  // AppKit's keyboard-layout pass — what turns an arrow or a delete key into an editing
  // command — runs only through an activated text input context, which AppKit grants an
  // active application's key window alone. capture_window.rs activates the context by
  // hand for that case; should the pass still yield nothing, AppKit inserts the key's
  // private-use character as text (NSEvent.h's NSLeftArrowFunctionKey and family; the
  // delete key is the ASCII DEL, return the CR) and a box would land per press. Here that
  // character is turned back into the edit it stood for, and any other control or
  // private-use character is refused as text. A working pass never reaches this: it
  // produces commands, not text.
  type FunctionKeyEdit = "left" | "right" | "up" | "down" | "home" | "end" | "backspace" | "delete" | "newline";
  const FUNCTION_KEY_EDITS: Record<string, FunctionKeyEdit> = {
    "\uF702": "left",
    "\uF703": "right",
    "\uF700": "up",
    "\uF701": "down",
    "\uF729": "home",
    "\uF72B": "end",
    "\u007F": "backspace",
    "\uF728": "delete",
    "\r": "newline",
  };
  function onBeforeInput(e: InputEvent) {
    if (e.inputType !== "insertText" || !e.data) return;
    const edit = FUNCTION_KEY_EDITS[e.data];
    if (edit) {
      e.preventDefault();
      applyEdit(edit);
    } else if (/^[\u0000-\u001F\uF700-\uF8FF]+$/.test(e.data)) {
      e.preventDefault();
    }
  }
  /** The edit on the textarea's own selection: arrows move (⇧ extends, ⌥ by word, ⌘ to
   *  the line's ends), up and down go by logical line and keep the column, the delete
   *  keys remove the selection or one character, return breaks the line. */
  function applyEdit(edit: FunctionKeyEdit) {
    const ta = textarea;
    if (!ta) return;
    const v = ta.value;
    const s = ta.selectionStart;
    const en = ta.selectionEnd;
    const backward = ta.selectionDirection === "backward";
    const anchor = backward ? en : s;
    const focus = backward ? s : en;
    const lineStart = (i: number) => v.lastIndexOf("\n", i - 1) + 1;
    const lineEnd = (i: number) => {
      const n = v.indexOf("\n", i);
      return n < 0 ? v.length : n;
    };
    const wordLeft = (i: number) => {
      let j = i;
      while (j > 0 && /\s/.test(v[j - 1])) j--;
      while (j > 0 && !/\s/.test(v[j - 1])) j--;
      return j;
    };
    const wordRight = (i: number) => {
      let j = i;
      while (j < v.length && /\s/.test(v[j])) j++;
      while (j < v.length && !/\s/.test(v[j])) j++;
      return j;
    };
    let target: number | null = null;
    switch (edit) {
      case "left":
        target = lastMods.meta ? lineStart(focus) : lastMods.alt ? wordLeft(focus) : !lastMods.shift && s !== en ? s : Math.max(0, focus - 1);
        break;
      case "right":
        target = lastMods.meta ? lineEnd(focus) : lastMods.alt ? wordRight(focus) : !lastMods.shift && s !== en ? en : Math.min(v.length, focus + 1);
        break;
      case "home":
        target = 0;
        break;
      case "end":
        target = v.length;
        break;
      case "up": {
        const ls = lineStart(focus);
        target = ls === 0 ? 0 : Math.min(lineStart(ls - 1) + (focus - ls), ls - 1);
        break;
      }
      case "down": {
        const le = lineEnd(focus);
        target = le >= v.length ? v.length : Math.min(le + 1 + (focus - lineStart(focus)), lineEnd(le + 1));
        break;
      }
      case "backspace":
        if (s === en && s === 0) return;
        ta.setRangeText("", s === en ? s - 1 : s, en, "end");
        break;
      case "delete":
        if (s === en && en >= v.length) return;
        ta.setRangeText("", s, s === en ? en + 1 : en, "end");
        break;
      case "newline":
        ta.setRangeText("\n", s, en, "end");
        break;
    }
    if (target !== null) {
      if (lastMods.shift) ta.setSelectionRange(Math.min(anchor, target), Math.max(anchor, target), target < anchor ? "backward" : "forward");
      else ta.setSelectionRange(target, target);
    } else {
      // setRangeText fires no input event: the bound text follows by hand, and the height with it
      text = ta.value;
      grow();
    }
  }

  async function dismiss() {
    await api.captureWindowHide().catch(() => undefined);
  }

  // ---- attachments ----
  async function onPaste(e: ClipboardEvent) {
    const dt = e.clipboardData;
    if (!isFileLikePaste(dt)) return; // plain text pastes into the text area as usual
    e.preventDefault();
    const domFiles = Array.from(dt?.files ?? []);
    // Only readable during dispatch: the plain text is the fallback when nothing file-like is
    // actually there (a URL copied from a browser arrives as text/uri-list, which reads as
    // file-like, with no file and no image behind it) — as the editor's paste falls back
    const plain = dt?.getData("text/plain") ?? "";
    try {
      // The native reader first (a file manager's copy as paths, a screenshot as bytes named by
      // the pasteboard's type); the DOM files are the fallback for platforms without one
      const probe = await api.clipboardProbe();
      if (probe.files.length) return addPaths(probe.files);
      if (probe.image) return addBytes(await api.clipboardImage(), `${stampName("capture")}.${probe.image}`);
      for (const f of domFiles) addBytes(new Uint8Array(await f.arrayBuffer()), pastedName(f.name, f.type, "capture"));
      if (!domFiles.length && plain) insertText(plain);
    } catch (err) {
      error = String(err);
    }
  }
  /** Text into the textarea at its selection (setRangeText fires no input event: the bound text and the height follow by hand). */
  function insertText(s: string) {
    const ta = textarea;
    if (!ta) return;
    ta.setRangeText(s, ta.selectionStart, ta.selectionEnd, "end");
    text = ta.value;
    grow();
  }
  function onDrag(p: { type: string; paths?: string[] }) {
    if (p.type === "enter" || p.type === "over") dragging = true;
    else if (p.type === "leave") dragging = false;
    else if (p.type === "drop") {
      dragging = false;
      if (p.paths?.length) addPaths(p.paths);
    }
  }
  function addBytes(bytes: Uint8Array<ArrayBuffer>, file: string) {
    const url = fileKind(file) === "image" ? URL.createObjectURL(new Blob([bytes])) : null;
    shots = [...shots, { id: nextShot++, name: file, bytes, url }];
    error = null;
  }
  function addPaths(paths: string[]) {
    shots = [...shots, ...paths.map((p) => ({ id: nextShot++, name: p.split(/[/\\]/).pop() || p, path: p, url: null }))];
    error = null;
  }
  function removeShot(id: number) {
    const s = shots.find((x) => x.id === id);
    if (s?.url) URL.revokeObjectURL(s.url);
    shots = shots.filter((x) => x.id !== id);
    textarea?.focus();
  }

  // ---- landing ----
  /** The landings in flight, by request id; the main window's receipt resolves the matching one. */
  const pending = new Map<number, (r: CaptureResult) => void>();
  let nextId = 1;
  function relay(a: Omit<CaptureAction, "id">): Promise<CaptureResult> {
    const id = nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(t("capture.noReply")));
      }, REPLY_TIMEOUT_MS);
      pending.set(id, (r) => {
        clearTimeout(timer);
        pending.delete(id);
        resolve(r);
      });
      api.captureToMain({ id, ...a }).catch((e) => {
        clearTimeout(timer);
        pending.delete(id);
        reject(e);
      });
    });
  }

  async function land() {
    const body = text.replace(/^\n+|\s+$/g, "");
    if (!body && !shots.length) return dismiss();
    // Taken out of the composer before anything is awaited: the window hides now, the
    // composer is empty for the next thought, and this capture lands on its own
    const taken = { text: body, shots, picked, spot };
    void api.captureWindowHide().catch(() => undefined);
    text = "";
    shots = [];
    picked = null;
    error = null;
    await tick();
    grow();
    try {
      const bucket = noteBucketOf(taken.spot.path);
      const attachments: string[] = [];
      for (const s of taken.shots) {
        if (s.saved?.bucket !== bucket) {
          const rel = s.bytes ? await api.attachmentSave(s.bytes, `${bucket}/${s.name}`) : (await api.filesImport([s.path!], bucket))[0];
          if (!rel) throw new Error(t("capture.attachFailed", { name: s.name }));
          s.saved = { bucket, rel };
        }
        attachments.push(s.saved.rel);
      }
      const r = await relay({ text: taken.text, target: taken.spot.path, heading: taken.spot.heading, attachments });
      if (!r.ok) throw new Error(r.error ?? t("capture.failed"));
      // Landed (a section that was not found fell back to the end of the note, which the main window reports on its side); nothing to show here
      for (const s of taken.shots) if (s.url) URL.revokeObjectURL(s.url);
    } catch (e) {
      // Back in front with the reason. Anything typed since goes under the failed text, the
      // attachments return to the strip, and a target picked for it is kept for the retry
      text = text ? `${taken.text}\n\n${text}` : taken.text;
      shots = [...taken.shots, ...shots];
      picked = taken.picked ?? picked;
      error = e instanceof Error ? e.message : String(e);
      await tick();
      grow();
      void api.captureWindowShow().catch(() => undefined);
    }
  }

  // ---- the note picker ----
  async function openPicker() {
    stage = "notes";
    query = "";
    selected = 0;
    pickNote = null;
    await tick();
    pickInput?.focus();
  }
  async function closePicker() {
    stage = "compose";
    await tick();
    textarea?.focus();
  }
  async function enterHeadings(path: string, title: string) {
    pickNote = { path, title };
    headings = await fetchHeadings(path).catch(() => []);
    stage = "headings";
    query = "";
    selected = 0;
  }
  async function choose(row: Row | undefined) {
    if (!row) return;
    error = null;
    if (row.kind === "note") picked = { path: row.path, heading: null };
    else if (row.kind === "create") {
      try {
        picked = { path: await resolveOrCreateNote(row.title), heading: null };
      } catch (e) {
        error = String(e);
        return;
      }
    } else if (pickNote) picked = { path: pickNote.path, heading: row.text };
    await closePicker();
  }
  function onPickKey(e: KeyboardEvent) {
    if (isImeKeydown(e)) return;
    const n = rows.length;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      selected = n ? (selected + 1) % n : 0;
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selected = n ? (selected - 1 + n) % n : 0;
    } else if (e.key === "Enter") {
      e.preventDefault();
      void choose(rows[selected]);
    } else if (e.key === "Tab" && stage === "notes") {
      // Into the sections of the highlighted note (the same gesture as the palette's pick-note); on "create" there is nothing to drill into
      e.preventDefault();
      const r = rows[selected];
      if (r?.kind === "note") void enterHeadings(r.path, r.title);
    } else if (e.key === "Escape") {
      e.preventDefault();
      if (stage === "headings") {
        stage = "notes";
        query = "";
        selected = 0;
      } else void closePicker();
    }
  }
  $effect(() => {
    void query;
    selected = 0;
  });
  /** The `#` shorthand of the palette: typing it in the notes stage drills into the highlighted note's sections. */
  function onPickInput() {
    if (stage === "notes" && query.endsWith("#")) {
      query = query.slice(0, -1);
      const r = rows[selected];
      if (r?.kind === "note") void enterHeadings(r.path, r.title);
    }
  }
</script>

<div class="box" class:dragging bind:this={box}>
  {#if stage === "compose"}
    <textarea
      bind:this={textarea}
      bind:value={text}
      rows="1"
      placeholder={t("capture.placeholder")}
      spellcheck="false"
      onkeydown={onTextKey}
      onbeforeinput={onBeforeInput}
      oninput={onInput}
      onpaste={onPaste}
    ></textarea>
    {#if shots.length}
      <div class="shots">
        {#each shots as s (s.id)}
          <div class="shot" title={s.name}>
            {#if s.url}
              <img src={s.url} alt={s.name} />
            {:else}
              <span class="shot-name">{s.name}</span>
            {/if}
            <button class="shot-x" aria-label={t("capture.attachmentRemove")} onclick={() => removeShot(s.id)}>×</button>
          </div>
        {/each}
      </div>
    {/if}
    <div class="foot">
      <button class="target" title={t("capture.targetChange")} onclick={() => void openPicker()}>
        <span class="arrow">→</span>{spotLabel(spot)}<span class="chev">▾</span>
      </button>
      <span class="keys">
        <span><kbd>{k("Enter")}</kbd> {t("capture.keyEnter")}</span>
        <span><kbd>{k("Shift+Enter")}</kbd> {t("capture.keyNewline")}</span>
        <span><kbd>{k("Mod+Enter")}</kbd> {t("capture.keyPick")}</span>
        <span><kbd>{k("Escape")}</kbd> {t("capture.keyEsc")}</span>
      </span>
    </div>
    {#if error}
      <div class="error">{error}</div>
    {:else if hintShows <= HINT_SHOWS}
      <div class="hint">{t("capture.firstHint", { target: spotLabel(defaultCaptureSpot()), key: k("Mod+Enter"), hotkey })}</div>
    {/if}
    {#if dragging}
      <div class="drop">{t("capture.dropHere")}</div>
    {/if}
  {:else}
    <div class="pick">
      <input
        bind:this={pickInput}
        bind:value={query}
        placeholder={stage === "headings" && pickNote ? t("capture.pickHeadingPlaceholder", { title: pickNote.title }) : t("capture.pickPlaceholder")}
        spellcheck="false"
        onkeydown={onPickKey}
        oninput={onPickInput}
      />
      <ul>
        {#each rows as row, i (row.kind === "note" ? row.path : row.kind === "create" ? "__create__" : (row.text ?? "__whole__"))}
          <li class:selected={i === selected}>
            <button class="row" tabindex="-1" onclick={() => void choose(row)} onmouseenter={() => (selected = i)}>
              {#if row.kind === "note"}
                <span class="row-title">{row.title}</span><span class="row-hint">{row.path}</span>
              {:else if row.kind === "create"}
                <span class="row-title">{t("palette.createNamed", { title: row.title })}</span>
              {:else}
                <span class="row-title" style:padding-left={row.text ? `${Math.max(0, row.level - 1) * 12}px` : null}>{row.text ?? t("palette.wholeNote")}</span>
              {/if}
            </button>
            {#if row.kind === "note"}
              <button class="drill" tabindex="-1" onclick={() => void enterHeadings(row.path, row.title)} onmouseenter={() => (selected = i)}>
                {t("palette.pickSection")}
              </button>
            {/if}
          </li>
        {:else}
          <li class="empty">{t("palette.noMatch")}</li>
        {/each}
      </ul>
      <div class="hint">{stage === "headings" ? t("capture.pickHeadingHint") : t("capture.pickHint")}</div>
    </div>
  {/if}
</div>

<style>
  /* The box is the window: rounded on a transparent page, the system shadow around it */
  .box {
    position: relative;
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    overflow: hidden;
    font-size: var(--fs-md);
  }
  .box.dragging {
    border-color: var(--accent);
  }
  textarea {
    display: block;
    width: 100%;
    min-height: 84px;
    padding: 14px 16px 6px;
    border: 0;
    outline: 0;
    resize: none;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: 15px;
    line-height: 1.6;
  }
  textarea::placeholder {
    color: var(--fg-muted);
  }
  .shots {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 4px 16px 10px;
  }
  .shot {
    position: relative;
    width: 56px;
    height: 56px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg-panel);
    overflow: visible;
  }
  .shot img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: 5px;
    display: block;
  }
  .shot-name {
    display: block;
    padding: 4px;
    font-size: 11px;
    line-height: 1.25;
    color: var(--fg-muted);
    overflow: hidden;
    word-break: break-all;
    max-height: 100%;
  }
  .shot-x {
    position: absolute;
    top: -6px;
    right: -6px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--fg);
    color: var(--bg);
    font-size: 11px;
    line-height: 16px;
    text-align: center;
    padding: 0;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 34px;
    padding: 6px 12px;
    border-top: 1px solid var(--border);
    background: var(--bg-panel);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .target {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 55%;
    padding: 2px 8px;
    border-radius: 6px;
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--fg);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .target:hover {
    background: var(--bg-hover);
  }
  .target .arrow,
  .target .chev {
    color: var(--fg-faint);
  }
  .keys {
    margin-left: auto;
    display: flex;
    gap: 12px;
    white-space: nowrap;
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--fg);
  }
  .hint,
  .error {
    padding: 5px 12px 7px;
    font-size: var(--fs-xs);
    background: var(--bg-panel);
  }
  .hint {
    color: var(--fg-muted);
  }
  .error {
    color: var(--danger);
  }
  .drop {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 500;
    pointer-events: none;
  }
  /* ---- the picker ---- */
  .pick input {
    width: 100%;
    padding: 12px 16px;
    border: 0;
    border-bottom: 1px solid var(--border);
    border-radius: 0;
    background: transparent;
    font-size: 15px;
  }
  .pick ul {
    list-style: none;
    margin: 0;
    padding: 4px 0;
    max-height: 296px;
    overflow-y: auto;
  }
  .pick li {
    display: flex;
    align-items: stretch;
  }
  .pick li.selected {
    background: var(--accent-soft);
  }
  .pick .row {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 6px 16px;
    text-align: left;
    color: var(--fg);
  }
  .row-title {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row-hint {
    color: var(--fg-muted);
    font-size: var(--fs-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .drill {
    padding: 0 12px;
    color: var(--fg-muted);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .drill:hover {
    color: var(--accent);
  }
  .pick .empty {
    padding: 8px 16px;
    color: var(--fg-muted);
    font-size: var(--fs-sm);
  }
</style>
