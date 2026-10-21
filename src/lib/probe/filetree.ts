// The "filetree" probe: a new note, a new folder and a rename made through the file tree's
// inline input, the way a user makes them. A new item's input stands at the top of its
// folder and the item then sorts by its name; the checks are that the row glides from the
// input's spot to its own instead of jumping there, and stays put when it sorts where it was
// typed. It works in a scratch vault and goes back to the user's vault before it ends.
import { api } from "$lib/api";
import { t } from "$lib/i18n/i18n.svelte";
import { vault } from "$lib/stores/vault.svelte";
import { Probe, sleep } from "./index";

const ROW = 26;

function tree(): HTMLElement | null {
  return document.querySelector<HTMLElement>(".tree");
}

function row(path: string): HTMLElement | undefined {
  return [...(tree()?.querySelectorAll<HTMLElement>("[data-path]") ?? [])].find((el) => el.dataset.path === path);
}

function editInput(): HTMLInputElement | null {
  return tree()?.querySelector<HTMLInputElement>(".edit-row input") ?? null;
}

function headButton(label: string): HTMLButtonElement | undefined {
  return [...document.querySelectorAll<HTMLButtonElement>(".head-actions button")].find((b) => b.getAttribute("aria-label") === label);
}

/** Type a name into the open inline input and press Enter; returns the input's top on screen. */
function commit(name: string): number {
  const input = editInput()!;
  const top = input.parentElement!.getBoundingClientRect().top;
  input.value = name;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  return top;
}

/** The glide running on a row and the offset it starts from, if one runs. */
function glideOf(el: HTMLElement | undefined): { anim: Animation; dy: number } | null {
  for (const a of el?.getAnimations() ?? []) {
    const first = (a.effect as KeyframeEffect | null)?.getKeyframes()[0];
    const m = /translateY\((-?[\d.]+)px\)/.exec(String(first?.transform ?? ""));
    if (m) return { anim: a, dy: Number(m[1]) };
  }
  return null;
}

/** Wait for the row's glide, capture it halfway, let it finish, and check where it set off from. */
async function followGlide(p: Probe, path: string, from: number, shot: string): Promise<number | null> {
  const seen = await p.until(() => glideOf(row(path)) !== null, 4000);
  if (!p.check(`${path} glides into place`, seen)) return null;
  const { anim, dy } = glideOf(row(path))!;
  // The glide and the ground's fade are held at the same moment, so the shot is a real frame
  const all = row(path)!.getAnimations();
  for (const a of all) {
    a.pause();
    a.currentTime = 90;
  }
  await p.shot(shot);
  for (const a of all) a.play();
  await anim.finished;
  const top = row(path)!.getBoundingClientRect().top;
  p.check(`${path} set off from the input's spot`, Math.abs(top + dy - from) < 2, { from, top, dy });
  return dy;
}

export async function runMain() {
  const p = new Probe();
  p.log("filetree probe: main window");
  await sleep(1500);
  const originalVault = vault.root;

  await p.step("scratch vault with folders and notes", async () => {
    const path = await api.probeScratchPath();
    await vault.createVault(path, "probe");
    for (const dir of ["Archive", "Books"]) await api.folderCreate(dir);
    for (const n of ["Alpha", "Beta", "Delta", "Gamma", "Kappa"]) await api.noteWrite(`${n}.md`, `# ${n}\n`);
    await vault.refreshNotes();
    vault.leftOpen = true;
    p.check("the tree shows the notes", await p.until(() => !!row("Kappa.md") && !!row("Books"), 8000));
  });

  await p.step("a new note glides from the input at the top to where it sorts", async () => {
    headButton(t("fileTree.newNote"))?.click();
    p.check("the inline input opens", await p.until(() => editInput() !== null));
    const firstTop = row("Archive")!.getBoundingClientRect().top;
    p.check("it stands above the folders", editInput()!.getBoundingClientRect().top < firstTop);
    const from = commit("Zeta");
    const dy = await followGlide(p, "Zeta.md", from, "filetree-note-glide");
    // Two folders and five notes lie between the top and the end of the list
    p.check("it travelled down past the rows between", dy !== null && dy <= -ROW * 6, dy);
    p.check("the note is open", vault.currentPath === "Zeta.md", vault.currentPath);
    p.check("the input is gone", editInput() === null);
  });

  await p.step("a new folder glides among the folders", async () => {
    headButton(t("fileTree.newFolder"))?.click();
    p.check("the inline input opens", await p.until(() => editInput() !== null));
    const from = commit("Comics");
    const dy = await followGlide(p, "Comics", from, "filetree-folder-glide");
    p.check("it sorted after Archive and Books", dy !== null && Math.abs(dy + ROW * 2) < 2, dy);
  });

  await p.step("a rename glides from the old place to the new one", async () => {
    row("Alpha.md")!.dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true, cancelable: true }));
    p.check("the inline input opens in place", await p.until(() => editInput()?.value === "Alpha.md"), editInput()?.value);
    const from = commit("Omega.md");
    const dy = await followGlide(p, "Omega.md", from, "filetree-rename-glide");
    p.check("it moved down to sort before Zeta", dy !== null && dy < -ROW, dy);
  });

  await p.step("a rename that sorts where it was typed stays still", async () => {
    row("Beta.md")!.dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true, cancelable: true }));
    p.check("the inline input opens in place", await p.until(() => editInput()?.value === "Beta.md"));
    commit("Bets.md");
    p.check("the renamed row is there", await p.until(() => !!row("Bets.md")));
    await sleep(400);
    p.check("no glide runs", glideOf(row("Bets.md")) === null);
  });

  await p.step("restore", async () => {
    if (originalVault) await vault.openVault(originalVault);
  });

  await p.done();
}
