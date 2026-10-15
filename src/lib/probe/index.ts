// The development probe (src-tauri/src/probe.rs): a debug build started with
// VEMRY_PROBE=<name>[,exit] runs one scripted pass over a feature through the real stores,
// IPC and DOM, and reports every check to the terminal that launched it. Nothing here runs
// otherwise: `startProbe` asks Rust first, and a release build always answers null. The
// report is for the developer, in English, and never reaches the interface.
import { api } from "$lib/api";

export const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

export class Probe {
  passed = 0;
  failed = 0;

  log(line: string) {
    void api.probeReport(line).catch(() => {});
  }

  /** One check: PASS or FAIL with what was seen. Returns whether it held. */
  check(name: string, ok: boolean, seen?: unknown): boolean {
    if (ok) this.passed++;
    else this.failed++;
    const detail = seen === undefined ? "" : ` — ${typeof seen === "string" ? seen : JSON.stringify(seen)}`;
    this.log(`${ok ? "PASS" : "FAIL"} ${name}${detail}`);
    return ok;
  }

  /** A step of the pass: a throw inside it is a failed check, and the pass goes on. */
  async step(name: string, run: () => Promise<void>) {
    this.log(`— ${name}`);
    try {
      await run();
    } catch (e) {
      this.check(`${name}: threw`, false, String(e));
    }
  }

  /** Wait until `cond` holds, up to `ms`; whether it came to hold. */
  async until(cond: () => boolean, ms = 5000): Promise<boolean> {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      try {
        if (cond()) return true;
      } catch {
        /* not there yet */
      }
      await sleep(50);
    }
    return false;
  }

  /** Bring this window forward and capture it (when VEMRY_PROBE_SHOTS is set). */
  async shot(name: string) {
    const file = await api.probeShot(name).catch(() => null);
    this.log(file ? `SHOT ${file}` : `SHOT ${name} (not captured)`);
  }

  async done() {
    await api.probeDone(this.passed, this.failed).catch(() => {});
  }
}

/** Run the probe the plan names, for this window's part of it. */
export async function startProbe(where: "main" | "agent") {
  const plan = await api.probePlan().catch(() => null);
  if (!plan) return;
  const names = plan.split(",").map((n) => n.trim());
  if (names.includes("models")) {
    const m = await import("./models");
    await (where === "main" ? m.runMain() : m.runAgent());
  }
  if (names.includes("table") && where === "main") {
    const m = await import("./table");
    await m.runMain();
  }
  if (names.includes("feedback") && where === "main") {
    const m = await import("./feedback");
    await m.runMain(names.includes("feedback-send") ? "yes" : names.includes("feedback-offline") ? "offline" : "no");
  }
}
