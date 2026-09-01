import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import AgentWindow from "./AgentWindow.svelte";
import ClipBar from "./ClipBar.svelte";
import { initI18n } from "$lib/i18n/i18n.svelte";
import { isAgentWindow, isClipbarWindow } from "$lib/window";

// Language before mount so the first paint is usually already right; not awaited
// because `t()` is reactive and a late answer just relabels on the next frame.
void initI18n();

// One bundle mounted in three kinds of window: the main window mounts the full
// application, the detached AI chat window mounts the conversation side alone, and the
// clip toolbar at the bottom of a browser window mounts a 40px bar
const app = mount(isClipbarWindow ? ClipBar : isAgentWindow ? AgentWindow : App, {
  target: document.getElementById("app")!,
});

export default app;
