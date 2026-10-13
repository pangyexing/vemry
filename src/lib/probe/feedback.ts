// The "feedback" probe: the feedback dialog driven the way a user would — from the
// palette command and from the settings page, its kind, its draft surviving a close, its
// diagnostics against what the send would attach, its keys. It sends nothing unless the
// plan also says `feedback-send`, and then only to wherever the build talks to: run it
// with VEMRY_API_BASE pointing at a local vemry-api, never at the real service. With
// `feedback-offline` instead, VEMRY_API_BASE points at a port nobody listens on, and the
// send is expected to fail with the draft kept and the mail fallback offered.
import { api } from "$lib/api";
import { executeCommand } from "$lib/commands/registry";
import { t } from "$lib/i18n/i18n.svelte";
import { account } from "$lib/stores/account.svelte";
import { feedback } from "$lib/stores/feedback.svelte";
import { openSettings, settingsUi } from "$lib/stores/settingsUi.svelte";
import { toast } from "$lib/stores/toast.svelte";
import { Probe, sleep } from "./index";

const dialog = () => document.querySelector<HTMLElement>("[role=dialog][aria-labelledby=feedback-title]");
const textarea = () => dialog()?.querySelector<HTMLTextAreaElement>("textarea") ?? null;
const sendButton = () => dialog()?.querySelector<HTMLButtonElement>("footer .btn.primary") ?? null;

/** Type into a field the way the keyboard does: the value, then the input event bind:value listens for. */
function type(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  el.focus();
  el.value = value;
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

function key(k: string, mods: KeyboardEventInit = {}) {
  const target = document.activeElement ?? document.body;
  target.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...mods }));
}

export async function runMain(send: "no" | "yes" | "offline") {
  const p = new Probe();
  p.log(`feedback probe: main window (${account.loggedIn ? "signed in" : "signed out"}, send: ${send})`);
  await sleep(1500);
  const draft = `probe feedback ${new Date().toISOString()}`;

  await p.step("the command opens the dialog with the caret in the text", async () => {
    await executeCommand("send-feedback");
    p.check("the dialog is open", await p.until(() => !!dialog()));
    p.check("the text has the focus", await p.until(() => document.activeElement === textarea()));
    p.check("send waits for a text", sendButton()?.disabled === true);
  });

  await p.step("the diagnostics listed are the ones the send attaches", async () => {
    const want = await api.feedbackDiagnostics();
    p.log(`diagnostics: ${JSON.stringify(want)}`);
    const shown = () => [...(dialog()?.querySelectorAll("dd") ?? [])].map((d) => (d.textContent ?? "").trim());
    p.check("four lines, filled in", await p.until(() => shown().length === 4 && !shown().includes("…")), shown());
    p.check("each line is the value itself", JSON.stringify(shown()) === JSON.stringify([want.app_version, want.os, want.lang, want.model]), shown());
    p.check("the version is the app's", /^\d+\.\d+\.\d+/.test(want.app_version), want.app_version);
  });

  await p.step("the kind sets the prompt", async () => {
    const radios = [...(dialog()?.querySelectorAll<HTMLButtonElement>("[role=radio]") ?? [])];
    p.check("three kinds", radios.length === 3, radios.map((r) => r.textContent?.trim()));
    radios[1]?.click();
    p.check("Idea is chosen", await p.until(() => radios[1]?.getAttribute("aria-checked") === "true" && feedback.kind === "idea"));
    p.check("the prompt follows", textarea()?.placeholder === t("feedback.placeholder.idea"), textarea()?.placeholder);
    radios[0]?.click();
    await p.until(() => feedback.kind === "bug");
  });

  await p.step("the reply address follows the sign-in", async () => {
    const email = dialog()?.querySelector<HTMLInputElement>("input[type=email]");
    if (account.loggedIn) {
      p.check("no address field while signed in", !email);
      const line = dialog()?.querySelector(".reply")?.textContent ?? "";
      p.check("the account's address is named", !!account.me?.email && line.includes(account.me.email), line);
    } else {
      p.check("an address field while signed out", !!email);
      if (email) type(email, "probe@example.com");
    }
  });

  await p.step("a close keeps the draft", async () => {
    const ta = textarea();
    if (!p.check("the text is there", !!ta)) return;
    type(ta!, draft);
    p.check("send is offered once there is a text", await p.until(() => sendButton()?.disabled === false));
    await p.shot("feedback-dialog");
    key("Escape");
    p.check("Escape closes it", await p.until(() => !dialog()));
    await executeCommand("send-feedback");
    p.check("it opens again", await p.until(() => !!dialog()));
    p.check("with the draft", textarea()?.value === draft, textarea()?.value);
    const box = dialog()?.querySelector<HTMLInputElement>("input[type=checkbox]");
    box?.click();
    p.check("diagnostics can be left out", await p.until(() => feedback.diagnostics === false && !!dialog()?.querySelector(".diag.off")));
    box?.click();
    await p.until(() => feedback.diagnostics);
    key("Escape");
    await p.until(() => !dialog());
  });

  await p.step("the settings page opens it above itself, and Escape closes only it", async () => {
    openSettings("general.feedback");
    await p.until(() => settingsUi.open);
    const button = await p.until(() =>
      [...document.querySelectorAll<HTMLButtonElement>("button")].some((b) => b.textContent?.trim() === t("settings.feedbackOpen")),
    );
    if (!p.check("the settings row has the button", button)) return;
    [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === t("settings.feedbackOpen"))!.click();
    p.check("the dialog opens", await p.until(() => !!dialog()));
    await p.shot("feedback-over-settings");
    key("Escape");
    p.check("Escape closed the dialog", await p.until(() => !dialog()));
    await sleep(200);
    p.check("and left the settings open", settingsUi.open);
    settingsUi.open = false;
  });

  if (send === "offline") {
    await p.step("an unreachable server keeps the draft and offers a mail instead", async () => {
      await executeCommand("send-feedback");
      await p.until(() => !!dialog());
      sendButton()?.click();
      const alert = () => dialog()?.querySelector<HTMLElement>("[role=alert]") ?? null;
      p.check("the reason is shown in the dialog", await p.until(() => !!alert(), 25000));
      const head = t("feedback.error.network", { error: "" });
      p.check("in the feedback's own words", (alert()?.textContent ?? "").trim().startsWith(head.trim()), alert()?.textContent);
      p.check("the mail fallback is offered", alert()?.querySelector("button")?.textContent?.trim() === t("feedback.mailInstead"));
      p.check("the draft is kept", feedback.text === draft, feedback.text);
      p.check("send can be tried again", sendButton()?.disabled === false);
      await p.shot("feedback-offline");
      key("Escape");
      await p.until(() => !dialog());
    });
  }

  if (send === "yes") {
    await p.step("⌘/Ctrl+Enter sends it, and the draft goes with it", async () => {
      await executeCommand("send-feedback");
      await p.until(() => !!dialog());
      textarea()?.focus();
      key("Enter", { metaKey: true, ctrlKey: true });
      // Against a local server the "Sending…" state lasts a few milliseconds: the outcome is what is checked
      p.check("the dialog closes once it is taken", await p.until(() => !dialog(), 20000), feedback.text);
      p.check("a thank-you toast", toast.items.some((i) => i.message === t("feedback.sent")), toast.items.map((i) => i.message));
      p.check("the draft is cleared", feedback.text === "");
      await p.shot("feedback-sent");
    });
  }

  await p.done();
}
