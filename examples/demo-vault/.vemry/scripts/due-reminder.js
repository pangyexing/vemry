// A due-task reminder. At the times below it shows a toast with what is due today
// (overdue included), and the same check is a command you can run at any moment.
// It only reads — no write, so it never asks for a grant — and runs in the QuickJS
// sandbox like every script: no filesystem, no network, only the `vemry.*` API.
//
// Nothing here runs on its own: the schedule fires because you installed this script,
// and only while vemry is running. Copy it into your vault's .vemry/scripts/ and
// change the times to your routine.

const pad = (n) => String(n).padStart(2, "0");

function today() {
  const d = new Date();
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** The first line of a task without its checkbox, trimmed to fit a toast. */
function title(hit) {
  const line = hit.block.content.split("\n")[0].replace(/^\s*(?:[-*+]|\d+[.)])\s+\[.\]\s*/, "");
  return line.length > 40 ? `${line.slice(0, 39)}…` : line;
}

function remind() {
  // due_before is "on or before": today's tasks and the overdue ones in one query
  const hits = vemry.query({ kind: "task", due_before: today(), limit: 200 });
  const open = hits.filter((h) => h.block.kind.type === "task" && h.block.kind.state !== "done");
  if (open.length === 0) {
    vemry.toast("Nothing due today");
    return;
  }
  const first = open.slice(0, 3).map(title).join(" · ");
  vemry.toast(`${open.length} due today: ${first}${open.length > 3 ? " …" : ""}`);
}

vemry.commands.register({ id: "due-today", title: "What is due today?" }, remind);

// Standard 5-field cron: a morning and an after-lunch check.
vemry.schedule("0 9 * * *", remind);
vemry.schedule("0 14 * * *", remind);
