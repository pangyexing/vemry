// Which webview this bundle is mounted in. One frontend bundle is mounted in three
// kinds of webview: `main` (the application's main window), `agent` (the detached AI
// chat window) and `browserbar` (the chrome bar at the top of the detached browser
// window, a child webview beside the browser tabs). main.ts picks the root component
// from it, and the rest of the code only branches on actions that "have to happen
// back in the main window" (see host.ts).
// The label is the **webview's**, not the window's: the bar's window is `browserwin`,
// and a window-label check would mount the whole application inside a 72px bar. It
// comes synchronously from the runtime metadata Tauri injects, so no URL parameter is
// needed (in dev and in production every webview loads the same URL).
import { getCurrentWebview } from "@tauri-apps/api/webview";

export const windowLabel: string = (() => {
  try {
    return getCurrentWebview().label;
  } catch {
    return "main"; // outside Tauri (svelte-check, unit tests) it counts as the main window
  }
})();

export const isMainWindow = windowLabel === "main";
export const isAgentWindow = windowLabel === "agent";
export const isBrowserbarWindow = windowLabel === "browserbar";
