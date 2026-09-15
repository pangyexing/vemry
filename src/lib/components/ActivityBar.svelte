<script lang="ts" module>
  import type { ProjectionView } from "$lib/stores/vault.svelte";

  /** The activity bar's fixed width. The CSS flex-basis and App.svelte's drag limit share
   *  this one number — changing the bar's width changes only this. */
  export const ABAR_W = 44;

  // The one place the view icons are defined (shared by the rail's buttons and TabBar's view
  // tabs).
  // The icon glyph conventions (a codicon-style geometric language, keeping this repo's 20
  // grid): a bounding box of ≈15×15, optically centred (the coordinates fall within
  // [2.5,17.5]); ≤6 paths per icon where possible; only 0/45/90° angles; a node is a closed
  // circle of r=1.8 (as graph draws it), and one path may hold several M subpaths to merge
  // strokes on the same side. An icon is an array of path d strings.
  // What makes a new icon belong to this family (ink coverage alone cannot tell — it measures
  // how much, not what kind; scripts/icon-ink.py measures it):
  // 1. three elements only: straight lines (0/45/90°, the odd steeper one), r=1.8 dots, r=1
  //    rounded squares — no Bézier curves, no free forms such as a drop or a flame;
  // 2. the elements stand apart as a diagram rather than fusing into one silhouette (graph's
  //    three dots and two lines never meet as one shape) — a single closed outline is the
  //    outsider here;
  // 3. open, airy, filling the frame: at most one or two small closed shapes (agenda's two
  //    squares), everything else open, the bounding box close to 15×15;
  // 4. ink coverage inside the rail's range — search at 26% is the lightest, review at 43%
  //    the heaviest, as scripts/icon-ink.py measures them — aiming for the middle (kanban,
  //    graph and timeline sit near 32%).
  export const VIEW_ICONS: Record<ProjectionView, string[]> = {
    agenda: [
      "M4 3.5 h3 a1 1 0 0 1 1 1 v3 a1 1 0 0 1 -1 1 h-3 a1 1 0 0 1 -1 -1 v-3 a1 1 0 0 1 1 -1 Z",
      "M4.6 6.1 l1.2 1.2 2.1 -2.5",
      "M4 11.5 h3 a1 1 0 0 1 1 1 v3 a1 1 0 0 1 -1 1 h-3 a1 1 0 0 1 -1 -1 v-3 a1 1 0 0 1 1 -1 Z",
      "M11 6 h6.5", "M11 14 h6.5",
    ],
    // The columns are not drawn as closed rectangles: three full-height closed columns are
    // the heaviest ink in the bar (58% coverage, where the lightest, search, is only 26%),
    // and the vertical edges account for most of it and cannot be shrunk. So it becomes a top
    // bar plus three columns of unequal height, the bounding box is still 15×15, and the ink
    // falls back into the graph/agenda cluster
    kanban: [
      "M2.5 3.5 h15",
      "M5 3.5 V16.5", "M10 3.5 V12.5", "M15 3.5 V14.5",
    ],
    // A clock (not sharing agenda's "a shape on the left plus lines on the right" composition): the hands point at 12 and 45° down-right
    timeline: [
      "M2.5 10 a7.5 7.5 0 1 0 15 0 a7.5 7.5 0 1 0 -15 0",
      "M10 6 V10 L12.8 12.8",
    ],
    graph: [
      "M8.2 4.5 a1.8 1.8 0 1 0 3.6 0 a1.8 1.8 0 1 0 -3.6 0",
      "M3 15 a1.8 1.8 0 1 0 3.6 0 a1.8 1.8 0 1 0 -3.6 0",
      "M13.4 13.5 a1.8 1.8 0 1 0 3.6 0 a1.8 1.8 0 1 0 -3.6 0",
      "M9.2 6.11 L5.6 13.39", "M10.9 6.06 L14.3 11.94",
    ],
    claims: [
      "M5.5 17.5 V3.5", "M5.5 4.5 h8.5 l-2 3 2 3 h-8.5",
    ],
    evidence: [
      "M4.5 5.5 h11", "M10 3.5 V16.5", "M7.5 16.5 h5",
      // The pans are flat lines rather than an r=2.3 arc: at 22px that arc blurs into a smudge, and flat pans read more like a balance anyway
      "M2.7 10.1 L5 5.5 L7.3 10.1 M2.7 10.1 h4.6",
      "M12.7 10.1 L15 5.5 L17.3 10.1 M12.7 10.1 h4.6",
    ],
    sessions: [
      "M3.5 5 h13", "M3.5 9 h13", "M3.5 13 h6", "M12.8 10.7 l4 2.3 -4 2.3 Z",
    ],
    // Two stacked cards: the front one closed, the back one showing only its top and right
    // edges behind it (a deck, not a single note), with the front's question line
    review: [
      "M3.5 7 h9 a1 1 0 0 1 1 1 v7.5 a1 1 0 0 1 -1 1 h-9 a1 1 0 0 1 -1 -1 v-7.5 a1 1 0 0 1 1 -1 Z",
      "M6.5 4.5 h9 a1 1 0 0 1 1 1 v7.5",
      "M6 10.5 h4",
    ],
  };
</script>

<script lang="ts">
  import { vault } from "$lib/stores/vault.svelte";
  import { executeCommand } from "$lib/commands/registry";
  import { t } from "$lib/i18n/i18n.svelte";
  import { k } from "$lib/shortcut";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { hotkeyLabel } from "$lib/capture";

  // ---- The vertical activity bar: the visible entry point to the projection views and the
  // settings (reachable without knowing ⌘P).
  // It is a launcher: a view is opened into the tab system through openView (with the
  // preview-slot semantics), and the rail only opens, closes and indicates the current one;
  // pressing the active view again closes its tab.
  // The views are grouped by meaning with an ab-sep between groups: vault-level projections
  // (projections of the whole vault's tasks and links) | research and learning (the products of
  // a research session, and the flashcards due). A later entry point joins the group it belongs to rather than being appended
  // linearly — once a group passes 4 or 5, merging entry points comes first, and the rail
  // does not grow forever.
  // view-reading is not in the bar: it means "return to the most recent projection", it only
  // toasts when there is no history, and a permanently grey icon is more confusing than none
  // — it stays in the command palette and the menu bar.
  // Labels come from `view.<id>` so the rail, the tab bar and the palette all
  // name a view the same way.
  const viewGroups: { cmd: string; view: ProjectionView }[][] = [
    // vault-level projections
    [
      { cmd: "view-agenda", view: "agenda" },
      { cmd: "view-kanban", view: "kanban" },
      { cmd: "view-timeline", view: "timeline" },
      { cmd: "view-graph", view: "graph" },
    ],
    // research and learning
    [
      { cmd: "view-claims", view: "claims" },
      { cmd: "view-evidence", view: "evidence" },
      { cmd: "view-sessions", view: "sessions" },
      { cmd: "view-review", view: "review" },
    ],
  ];
  const searchIcon = ["M8.75 4 a4.75 4.75 0 1 0 0 9.5 a4.75 4.75 0 1 0 0 -9.5", "M12.11 12.11 L16.5 16.5"];
  // Quick capture: a three-stroke zigzag (quick) landing on a baseline (the paper) — "jotted
  // down fast". Two open elements, 15×15, 22% of the frame inked (the lightest on the rail, a
  // hair under search); the zigzag alone would stand as narrow as claims's flag at 17%, and
  // the baseline is what fills the frame
  const captureIcon = ["M13.5 2.5 L6 10 h6.5 L8 15.5", "M3.5 17.5 h13"];
  // The teeth are merged into two paths, "the orthogonal set" and "the diagonal set" (the
  // conventions allow one path to hold several M subpaths), 9 → 3, back to the "≤6 paths per
  // icon where possible" this file sets itself; the ink is unchanged
  const gearIcon = [
    "M7 10 a3 3 0 1 0 6 0 a3 3 0 1 0 -6 0",
    "M10 2.9 v2.2 M10 14.9 v2.2 M2.9 10 h2.2 M14.9 10 h2.2",
    "M5 5 l1.55 1.55 M13.45 13.45 l1.55 1.55 M15 5 l-1.55 1.55 M6.55 13.45 l-1.55 1.55",
  ];
  // The file tree's toggle button: once collapsed, the activity bar is the only permanent way
  // to reopen it (the old floating reopen on the left was folded in here). The shape matches
  // the right column's reopen, a "sidebar rectangle plus a divider line", with the rect drawn
  // as a rounded path so it can reuse the icon snippet; it mirrors when the sides swap, and
  // the divider always faces the file tree's side
  const filesIcon = [
    "M4.25 3.5 h11.5 a1.75 1.75 0 0 1 1.75 1.75 v9.5 a1.75 1.75 0 0 1 -1.75 1.75 h-11.5 a1.75 1.75 0 0 1 -1.75 -1.75 v-9.5 a1.75 1.75 0 0 1 1.75 -1.75 Z",
    "M7.5 3.5 V16.5",
  ];
</script>

<!-- The label capsule that reads on hover. A native title takes one or two seconds to appear
  on macOS and cannot be swept across a whole bar — and a flag meaning claims or a balance
  meaning evidence cannot be guessed from the shape, so failing to read the label means
  failing to recognise it.
  The visual language follows app.css's .cm-lp-copy::after (an inverted little capsule with a
  0.15s delay); ::before/::after on .ab-btn are already taken by the indicator bar and the
  hover tile, so this uses a real element -->
{#snippet tip(text: string, hint: string)}
  <span class="tip">{text}{#if hint}<span class="tip-key">{hint}</span>{/if}</span>
{/snippet}

{#snippet icon(paths: string[], mirror: boolean = false)}
  <svg
    width="22"
    height="22"
    viewBox="0 0 20 20"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    style:transform={mirror ? "scaleX(-1)" : null}
  >
    {#each paths as d (d)}<path {d} />{/each}
  </svg>
{/snippet}

<nav class="abar" class:swapped={vault.swapSides} aria-label={t("activityBar.label")}>
  <button
    class="ab-btn"
    class:on={vault.leftOpen}
    aria-pressed={vault.leftOpen}
    aria-label={t("activityBar.fileTree")}
    onclick={() => (vault.leftOpen = !vault.leftOpen)}
  >
    {@render icon(filesIcon, vault.swapSides)}
    {@render tip(vault.leftOpen ? t("activityBar.collapseFileTree") : t("activityBar.openFileTree"), k("Mod+B"))}
  </button>
  <!-- "Find something": the palette it opens has two spaces, notes and the web, switched at
    its head — so the browser needs no launcher of its own on the rail -->
  <button class="ab-btn" aria-label={t("activityBar.find")} onclick={() => void executeCommand("search")}>
    {@render icon(searchIcon)}
    {@render tip(t("activityBar.find"), k("Mod+Shift+F"))}
  </button>
  <!-- Quick capture: the window a global shortcut shows from any app; the capsule carries that shortcut, the one place a hand sweeping the rail learns it -->
  <button class="ab-btn" aria-label={t("cmd.quick-capture.title")} onclick={() => void executeCommand("quick-capture")}>
    {@render icon(captureIcon)}
    {@render tip(t("cmd.quick-capture.title"), hotkeyLabel(prefs.captureHotkey))}
  </button>
  {#each viewGroups as group, gi (gi)}
    <span class="ab-sep"></span>
    {#each group as v (v.cmd)}
      <button
        class="ab-btn"
        class:active={vault.mainView === v.view}
        aria-label={t(`view.${v.view}`)}
        onclick={() => (vault.mainView === v.view ? void vault.closeActiveView() : void executeCommand(v.cmd))}
      >
        {@render icon(VIEW_ICONS[v.view])}
        {@render tip(t(`view.${v.view}`), vault.mainView === v.view ? t("activityBar.clickAgainToClose") : "")}
      </button>
    {/each}
  {/each}
  <div class="ab-spacer"></div>
  <button class="ab-btn" aria-label={t("cmd.settings.title")} onclick={() => void executeCommand("settings")}>
    {@render icon(gearIcon)}
    {@render tip(t("cmd.settings.title"), k("Mod+,"))}
  </button>
</nav>

<style>
  .abar {
    /* 44 = the ABAR_W this file's script module exports (App.svelte's drag limit comes from
       there); the CSS side is still a literal, so changing the bar's width changes two places */
    flex: 0 0 44px;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    padding: 4px 0;
    background: var(--bg-panel);
    border-right: 1px solid var(--border-strong);
  }
  .abar.swapped {
    border-right: none;
    border-left: 1px solid var(--border-strong);
  }
  /* The bar and the icon are 2:1: a 44-wide bar carries a 22 icon, and the button is a full-width square */
  .ab-btn {
    position: relative;
    height: 44px;
    display: grid;
    place-items: center;
    color: var(--fg-muted);
    /* The indicator bar and the icon are not animated (switching views is an instant event); only the hover colouring is softened */
    transition: color 0.12s ease;
  }
  /* Hover and active share one 32×32 rounded tile (::after, inset 6 lining a 22 icon);
     ::before is left to the edge indicator, and the svg is raised to float above the tile. The
     button itself no longer takes a background colour */
  .ab-btn::after {
    content: "";
    position: absolute;
    inset: 6px;
    border-radius: 8px;
    transition: background-color 0.12s ease;
  }
  .ab-btn svg {
    position: relative;
    z-index: 1;
  }
  .ab-btn:hover {
    color: var(--fg);
  }
  .ab-btn:hover::after {
    background: var(--bg-hover);
  }
  /* The file tree button is a switch rather than a view: it only brightens and does not take the accent's lit-up language */
  .ab-btn.on {
    color: var(--fg);
  }
  /* The view's active "lit up" state: accent colouring plus an accent-soft tile (the same
     active language as the file tree's selected row and the settings navigation), and hover
     does not fall back to a grey ground */
  .ab-btn.active,
  .ab-btn.active:hover {
    color: var(--accent);
  }
  .ab-btn.active::after,
  .ab-btn.active:hover::after {
    background: var(--accent-soft);
  }
  /* The 2px full-height indicator on the outer edge marks the current main view alone; the
     file tree button is a switch rather than a view and only brightens */
  .ab-btn.active::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--accent);
  }
  .abar.swapped .ab-btn.active::before {
    left: auto;
    right: 0;
  }
  /* The capsule has to cross the 44px bar and lie over the file tree: .left is
     position:relative too and comes after the activity bar in the DOM, so at the same layer
     the later one wins by document order and would cover it without a raised z-index. What is
     raised is the capsule itself rather than the whole bar (.ab-btn has no z-index and forms
     no layer, so the 50 here lands directly on the root layer), which keeps the whole bar from
     floating above the sash (30) and the reopen (40) */
  .tip {
    position: absolute;
    left: calc(100% + 4px);
    top: 50%;
    transform: translateY(-50%);
    z-index: 50;
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 4px 9px;
    border-radius: 5px;
    background: var(--fg);
    color: var(--bg);
    font-size: var(--fs-xs);
    line-height: 1.4;
    white-space: nowrap;
    pointer-events: none;
    opacity: 0;
    transition: opacity 0.12s;
  }
  /* Once the sides swap the bar hugs the right edge and the capsule flies out to the left */
  .abar.swapped .tip {
    left: auto;
    right: calc(100% + 4px);
  }
  /* It has to appear for the keyboard too, or tabbing through gives no feedback at all */
  .ab-btn:hover .tip,
  .ab-btn:focus-visible .tip {
    opacity: 1;
    transition-delay: 0.15s;
  }
  .tip-key {
    opacity: 0.6;
    font-family: var(--font-mono);
  }
  .ab-sep {
    flex-shrink: 0;
    height: 1px;
    margin: 4px 12px;
    background: var(--border);
  }
  .ab-spacer {
    margin-top: auto;
  }
</style>
