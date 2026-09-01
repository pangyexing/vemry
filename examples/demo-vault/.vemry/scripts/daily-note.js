// A user script. It runs in a QuickJS sandbox on the Rust side: no filesystem, no
// network, no processes — only the `vemry.*` API. Saving this file hot-reloads it.
//
// Nothing here runs on its own. The command runs when you pick it from the command
// palette; the schedule runs because you installed this script. The first write
// asks for permission once, and every write is recorded in .vemry/script-log.jsonl.
//
// The `vemry.*` calls are synchronous: they cross into Rust and come straight back.

const pad = (n) => String(n).padStart(2, "0");

function today() {
  const d = new Date();
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Create today's journal note if it does not exist yet, and report either way. */
function openOrCreateToday() {
  const path = `journal/${today()}.md`;
  const exists = vemry.list("journal").some((n) => n.id === path);
  if (exists) {
    vemry.toast(`${path} already exists`);
    return;
  }
  vemry.write(path, `---\ntitle: ${today()}\ntags: [journal]\n---\n\n# ${today()}\n\n`);
  vemry.toast(`Created ${path}`);
}

// Registers into the command palette, alongside every built-in command — this is
// what "everything is a command" means: a script's command is not a second class.
vemry.commands.register({ id: "daily-note", title: "Open today's journal note" }, openOrCreateToday);

// A vault event trigger. Writes made by a script cannot re-trigger a workflow, so
// this cannot cascade — the loop prevention is structural, not a guard clause.
vemry.events.on("note_added", (e) => {
  if (e.id && e.id.startsWith("journal/")) console.log(`new journal note: ${e.id}`);
});

// Standard 5-field cron. Fires only while vemry is running.
vemry.schedule("0 9 * * *", openOrCreateToday);
