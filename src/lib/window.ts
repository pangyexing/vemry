// Which window this webview belongs to. One frontend bundle is mounted in three kinds
// of window: `main` (the application's main window), `agent` (the detached AI chat
// window) and `clipbar-N` (the clip toolbar at the bottom of a browser window, batch
// D). main.ts picks the root component from it, and the rest of the code only
// branches on actions that "have to happen back in the main window" (see host.ts).
// The label comes synchronously from the runtime metadata Tauri injects, so no URL
// parameter is needed (in dev and in production every window loads the same URL).
import { getCurrentWindow } from "@tauri-apps/api/window";

export const windowLabel: string = (() => {
  try {
    return getCurrentWindow().label;
  } catch {
    return "main"; // outside Tauri (svelte-check, unit tests) it counts as the main window
  }
})();

export const isAgentWindow = windowLabel === "agent";
export const isClipbarWindow = windowLabel.startsWith("clipbar-");

// The `clipbar-N` ↔ `browser-N` pairing is a load-bearing routing convention (the
// toolbar has to know which browser window it is stuck to, and the main window has to
// know which toolbar to send a receipt back to). On the Rust side it is collected in
// clipbar.rs::clipbar_label_of / browser_label_of, and on the TS side here —
// knowledge of a label belongs to this module and should not be scattered across
// components as regexes.
export const clipbarLabelOf = (browserLabel: string) =>
  `clipbar-${browserLabel.replace(/^browser-/, "")}`;
export const browserLabelOf = (clipbarLabel: string) =>
  `browser-${clipbarLabel.replace(/^clipbar-/, "")}`;
