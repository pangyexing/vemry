import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import AgentWindow from "./AgentWindow.svelte";
import BrowserBar from "./BrowserBar.svelte";
import { initI18n } from "$lib/i18n/i18n.svelte";
import { isAgentWindow, isBrowserbarWindow } from "$lib/window";

// Language before mount so the first paint is usually already right; not awaited
// because `t()` is reactive and a late answer just relabels on the next frame.
void initI18n();

// One bundle mounted in three kinds of webview: the main window mounts the full
// application, the detached AI chat window mounts the conversation side alone, and the
// chrome bar at the top of the detached browser window mounts the browser chrome
const app = mount(isBrowserbarWindow ? BrowserBar : isAgentWindow ? AgentWindow : App, {
  target: document.getElementById("app")!,
});

export default app;
