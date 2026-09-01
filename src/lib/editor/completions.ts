// Completion sources for the special markers: #tags and > [!callout types.
// The wikilink completion source lives in wikilink.ts (it needs the host); the
// sources sit side by side in setup.ts's autocompletion({override}), and each returns
// null to make way when its matchBefore does not hit.
import type { Completion, CompletionContext, CompletionResult, CompletionSource } from "@codemirror/autocomplete";
import type { EditorView } from "@codemirror/view";
import { CALLOUT_GROUP } from "./livepreview";

/** `#tag` completion: the candidates are the vault's tags (the caller keeps them fresh and sorted by how often they are used). */
export function tagCompletions(tags: () => string[]): CompletionSource {
  return (context: CompletionContext): CompletionResult | null => {
    const before = context.matchBefore(/#[\p{L}\p{N}_/-]*$/u);
    if (!before) return null;
    // TAG_RE's rule: a # must be preceded by the start of a line or whitespace (which rules out a URL anchor, a run of ##, and so on)
    const prev = context.state.sliceDoc(before.from - 1, before.from);
    if (prev && !/\s/.test(prev)) return null;
    // A bare # at the start of a line is most likely a heading being typed: nothing pops up by itself, and only an explicit trigger (⌃Space) offers it
    const line = context.state.doc.lineAt(before.from);
    if (before.from === line.from && before.text === "#" && !context.explicit) return null;
    const options = tags().map((tag): Completion => ({ label: tag, type: "text" }));
    if (!options.length) return null;
    return { from: before.from + 1, options, validFor: /^[\p{L}\p{N}_/-]*$/u };
  };
}

/** Callout type completion for `> [!type` and for a bare `[!type` at the start of a
 *  line; accepting the latter adds the `> ` prefix (a callout only holds inside a
 *  quote block). detail shows the colour group. */
export function calloutCompletions(context: CompletionContext): CompletionResult | null {
  const before = context.matchBefore(/^(\s{0,3})(>\s*)?\[!([\w-]*)$/);
  if (!before) return null;
  const head = /^(\s{0,3})(>\s*)?\[!/.exec(before.text);
  if (!head) return null;
  const quoted = head[2] !== undefined;
  const bracketPos = before.from + head[1].length; // where `> ` is inserted before the `[` when unquoted
  const typeFrom = before.from + before.text.lastIndexOf("[!") + 2;
  const options = Object.keys(CALLOUT_GROUP).map(
    (name): Completion => ({
      label: name,
      detail: CALLOUT_GROUP[name],
      type: "text",
      // closeBrackets has already added the ]: with a ] after the cursor only the type name is inserted
      apply: (view: EditorView, _completion, applyFrom: number, applyTo: number) => {
        const closed = view.state.sliceDoc(applyTo, applyTo + 1) === "]";
        const changes = [{ from: applyFrom, to: applyTo, insert: closed ? name : `${name}]` }];
        if (!quoted) changes.push({ from: bracketPos, to: bracketPos, insert: "> " });
        const shift = quoted ? 0 : 2; // the `> ` inserted in front shifts the cursor target right
        view.dispatch({
          changes,
          selection: { anchor: applyFrom + shift + name.length + 1 }, // land after the ]
        });
      },
    }),
  );
  return { from: typeFrom, options, validFor: /^[\w-]*$/ };
}
