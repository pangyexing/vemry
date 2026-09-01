// Wikilink support for CodeMirror: decoration (resolved vs unresolved),
// completion after `[[`, and click / Cmd/Ctrl+click navigation.
import {
  Decoration,
  type DecorationSet,
  EditorView,
  MatchDecorator,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import { type EditorState } from "@codemirror/state";
import { type CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { linkClickHandlers, matchAt, type LinkHit } from "./clickopen";

export interface WikilinkHost {
  /** lowercase set of known stems/aliases, kept fresh by the caller */
  isKnown(target: string): boolean;
  targets(): string[];
  follow(target: string): void;
}

/** The one wikilink grammar. livepreview.ts hides brackets with the same
 *  regex — two copies would let clickable and hidden ranges drift apart. */
export const LINK_RE = /\[\[([^\[\]\n]+)\]\]/g;

function targetOf(inner: string): string {
  return inner.split("|")[0].split("#")[0].trim();
}

/** The wikilink at pos (an inline scan, on the same LINK_RE as the decoration).
 *  Mod-Enter uses it too.
 *  The `![[img]]` embed marker sets embed: in the rendered state it is an image
 *  widget and a click belongs to the widget; follow would take an image target that
 *  is not in the index for an unresolved note and create it, which an ordinary click
 *  must not walk into. */
export function wikiLinkAt(state: EditorState, pos: number): LinkHit | null {
  const line = state.doc.lineAt(pos);
  const m = matchAt(line, LINK_RE, pos);
  if (!m) return null;
  const target = targetOf(m[1]);
  const from = line.from + m.index;
  const embed = m.index > 0 && line.text[m.index - 1] === "!";
  return target ? { target, from, to: from + m[0].length, embed } : null;
}

export function wikilinkExtension(host: WikilinkHost, singleClick: () => boolean) {
  const decorator = new MatchDecorator({
    regexp: LINK_RE,
    decoration: (match) => {
      const known = host.isKnown(targetOf(match[1]).toLowerCase());
      return Decoration.mark({ class: known ? "cm-wikilink" : "cm-wikilink cm-wikilink-unresolved" });
    },
  });

  const plugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = decorator.createDeco(view);
      }
      update(update: ViewUpdate) {
        this.decorations = decorator.updateDeco(update, this.decorations);
      }
    },
    { decorations: (v) => v.decorations },
  );

  // invert means nothing for a note (there is only one way to open it), so it is ignored
  const clickHandler = linkClickHandlers(
    (view, pos) => wikiLinkAt(view.state, pos),
    (target) => host.follow(target),
    singleClick,
  );

  function completions(context: CompletionContext): CompletionResult | null {
    const before = context.matchBefore(/\[\[([^\]\n]*)$/);
    if (!before) return null;
    const from = before.from + 2;
    return {
      from,
      options: host.targets().map((name) => ({
        label: name,
        type: "text",
        // closeBrackets completes [[ into [[]]: with a ]] already after the cursor
        // nothing more is inserted, or a static apply would double-close into [[name]]]]
        apply: (view: EditorView, _completion, applyFrom: number, applyTo: number) => {
          const closed = view.state.sliceDoc(applyTo, applyTo + 2) === "]]";
          view.dispatch({
            changes: { from: applyFrom, to: applyTo, insert: closed ? name : `${name}]]` },
            selection: { anchor: applyFrom + name.length + 2 }, // land after the ]]
          });
        },
      })),
      validFor: /^[^\]\n]*$/,
    };
  }

  return { extension: [plugin, clickHandler], completions };
}
