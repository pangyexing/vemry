<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import {
    forceCenter,
    forceCollide,
    forceLink,
    forceManyBody,
    forceSimulation,
    forceX,
    forceY,
    type ForceLink,
    type Simulation,
  } from "d3-force";
  import { t } from "$lib/i18n/i18n.svelte";
  import { api } from "$lib/api";
  import { vault } from "$lib/stores/vault.svelte";
  import { adjacency, mergeGraph, nodeRadius, type SimLink, type SimNode } from "$lib/views/graph";
  import ViewHeader from "./ViewHeader.svelte";

  /** With no more nodes than this every label is drawn: beyond it, zooming and hovering are the way to pick them out. */
  const SMALL_VAULT = 60;
  /** How many characters of a label are drawn at most. A long clipped title can run to forty
   *  characters and more, and drawing it whole both covers half the graph and stretches the
   *  content-fitting bounding box until the main cluster is squashed — what the graph is for
   *  is recognising which note it is, not reading the title. */
  const LABEL_MAX = 18;

  /** The label's font size (constant in world coordinates, scaled by 1/k when drawn). Measuring
   *  and drawing have to use the same number, or the fitting bounding box would not match the
   *  text actually drawn. */
  const LABEL_PX = 11;
  /** Measuring and drawing have to use the same font string, so it is built here alone; the current scale is passed in when drawing. */
  const labelFont = (k = 1) => `${LABEL_PX / k}px system-ui, sans-serif`;

  /** Node id → the truncated label and its half width (in world coordinates). Both are
   *  functions of the node set and are recomputed only when it changes — truncating allocates
   *  strings and measureText is dearer still, while these two values are needed for every node
   *  on every frame. */
  const labels = new Map<string, { text: string; half: number }>();

  /** Measure the labels once. A canvas's 2d context is the only place here that can give a
   *  text width, so it is borrowed for the measuring: the transform is reset first and restored
   *  afterwards, leaving the next frame's drawing unaffected. */
  function measureLabels() {
    const ctx = canvas?.getContext("2d");
    if (!ctx) return;
    ctx.save();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.font = labelFont();
    labels.clear();
    for (const n of nodes) {
      const text = n.title.length > LABEL_MAX ? n.title.slice(0, LABEL_MAX) + "…" : n.title;
      labels.set(n.id, { text, half: ctx.measureText(text).width / 2 });
    }
    ctx.restore();
  }

  let canvas = $state<HTMLCanvasElement | null>(null);
  let container = $state<HTMLDivElement | null>(null);
  let nodeCount = $state(0);
  let edgeCount = $state(0);

  let nodes: SimNode[] = [];
  let links: SimLink[] = [];
  let adj = new Map<string, Set<string>>();
  let sim: Simulation<SimNode, SimLink> | null = null;
  let linkForce: ForceLink<SimNode, SimLink> | null = null;

  // The view transform (world coordinates → screen coordinates): scaling and panning written by hand, without d3-zoom
  // Canvas pan/zoom transform (renamed off `t`, which is the message lookup).
  const view = { x: 0, y: 0, k: 1 };
  let width = 0;
  let height = 0;
  let dpr = 1;
  /** It has fitted to the content once already. On the first frame the nodes have no
   *  coordinates yet, so the ResizeObserver puts the origin at the canvas's centre as a
   *  backstop and hands over to fitToContent afterwards. */
  let fitted = false;
  /** The user has zoomed or panned themselves. Fitting automatically after that would take away
   *  the view they just arranged — only a double click to reset hands it back to automatic. */
  let userMoved = false;

  let hovered: SimNode | null = null;
  let dragging: SimNode | null = null;
  let panning = false;
  let downAt = { x: 0, y: 0 };
  let moved = false;

  interface Palette {
    node: string;
    nodeHot: string;
    edge: string;
    label: string;
    bg: string;
    faint: string;
  }
  let palette: Palette | null = null;

  function readPalette(): Palette {
    const s = getComputedStyle(document.documentElement);
    const v = (name: string, fallback: string) => s.getPropertyValue(name).trim() || fallback;
    // A canvas does not understand CSS variables, so they are read once and cached; they are read again on a resize (a theme change remounts it)
    return {
      node: v("--fg-muted", "#888"),
      nodeHot: v("--accent", "#6d8dff"),
      edge: v("--border", "#ccc"),
      label: v("--fg", "#333"),
      bg: v("--bg", "#fff"),
      faint: v("--fg-faint", "#aaa"),
    };
  }

  /** Fit to the content: put every node's bounding box inside the canvas.
   *  Without this step the transform is fixed at k=1 with the origin centred — and in a
   *  force-directed layout a note with no links is pushed far away by the repulsion; measured,
   *  only 9 of 21 nodes fell inside the canvas and the rest were outside the frame, so the
   *  first thing seen on opening the graph was a small cluster in the middle rather than the
   *  whole.
   *  The upper bound of 1.4: with very few nodes, scaling up without limit only produces a few
   *  huge dots, and the scale itself carries no information; the lower bound is left to the
   *  same 0.2 the wheel uses. */
  function fitToContent() {
    if (width === 0 || height === 0) return;
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    let seen = 0;
    for (const n of nodes) {
      if (n.x === undefined || n.y === undefined) continue;
      seen++;
      const r = nodeRadius(n);
      // The label is drawn centred below the node: half a label's width on each side and one
      // more line below — without counting that into the bounding box, an edge node's name
      // would be drawn off the canvas
      const half = Math.max(r, labels.get(n.id)?.half ?? r);
      minX = Math.min(minX, n.x - half);
      maxX = Math.max(maxX, n.x + half);
      minY = Math.min(minY, n.y - r);
      maxY = Math.max(maxY, n.y + r + 14);
    }
    if (seen === 0) return;
    const pad = 32;
    const bw = Math.max(maxX - minX, 1);
    const bh = Math.max(maxY - minY, 1);
    view.k = Math.min(1.4, Math.max(0.2, Math.min((width - pad * 2) / bw, (height - pad * 2) / bh)));
    view.x = width / 2 - ((minX + maxX) / 2) * view.k;
    view.y = height / 2 - ((minY + maxY) / 2) * view.k;
    fitted = true;
  }

  /** The one exit of the view policy: unless the user has taken over, it fits to the content
   *  and then draws. tick, a resize and a double-click reset all come through here — so "how
   *  long the automatic behaviour lasts" is written in one place. */
  function render() {
    if (!userMoved) fitToContent();
    draw();
  }

  function toWorld(px: number, py: number): { x: number; y: number } {
    return { x: (px - view.x) / view.k, y: (py - view.y) / view.k };
  }

  function hitTest(px: number, py: number): SimNode | null {
    const w = toWorld(px, py);
    let best: SimNode | null = null;
    let bestD = Infinity;
    for (const n of nodes) {
      if (n.x === undefined || n.y === undefined) continue;
      const r = Math.max(nodeRadius(n), 6 / view.k);
      const d = Math.hypot(n.x - w.x, n.y - w.y);
      if (d <= r && d < bestD) {
        best = n;
        bestD = d;
      }
    }
    return best;
  }

  function draw() {
    const ctx = canvas?.getContext("2d");
    if (!ctx || !canvas) return;
    if (!palette) palette = readPalette();
    const p = palette;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    ctx.setTransform(dpr * view.k, 0, 0, dpr * view.k, dpr * view.x, dpr * view.y);

    const hotIds = hovered ? new Set([hovered.id, ...(adj.get(hovered.id) ?? [])]) : null;

    // The edges first
    for (const l of links) {
      const s = l.source as SimNode;
      const tg = l.target as SimNode;
      if (s.x === undefined || tg.x === undefined) continue;
      const hot = hotIds !== null && hotIds.has(s.id) && hotIds.has(tg.id);
      ctx.globalAlpha = hotIds === null ? 0.5 : hot ? 0.8 : 0.1;
      ctx.strokeStyle = hot ? p.nodeHot : p.edge;
      ctx.lineWidth = 0.5 + Math.log(l.count + 1);
      ctx.beginPath();
      ctx.moveTo(s.x!, s.y!);
      ctx.lineTo(tg.x!, tg.y!);
      ctx.stroke();
    }

    // Then the nodes
    for (const n of nodes) {
      if (n.x === undefined || n.y === undefined) continue;
      const hot = hotIds === null || hotIds.has(n.id);
      const isPrivate = vault.privatePaths.has(n.id);
      ctx.globalAlpha = hot ? 1 : 0.15;
      ctx.beginPath();
      ctx.arc(n.x, n.y, nodeRadius(n), 0, Math.PI * 2);
      ctx.fillStyle = n === hovered ? p.nodeHot : isPrivate ? p.faint : p.node;
      ctx.fill();
      if (isPrivate) {
        // A private node: a dashed outline, echoing the file tree's privacy badge
        ctx.setLineDash([2, 2]);
        ctx.strokeStyle = p.label;
        ctx.lineWidth = 1;
        ctx.stroke();
        ctx.setLineDash([]);
      }
    }

    // Labels: drawn for everything while the vault is small enough not to blur into one, or at
    // more than 1.2x zoom, or for a hovered node and its neighbours, or for a highly connected
    // node. The SMALL_VAULT rule is there to work with the content fitting — after fitting, the
    // scale often lands below 1, and the 1.2x gate alone would hide every name
    ctx.font = labelFont(view.k);
    ctx.textAlign = "center";
    // Stroke the background colour around them first and then fill: neighbouring nodes' labels
    // overlap (measured: two titles overlapped into an unreadable smudge), and the stroke keeps
    // the top one legible. The three stroke settings depend on the scale rather than the node,
    // so they are lifted out of the loop — set once per frame rather than once per label
    ctx.lineWidth = 3 / view.k;
    ctx.strokeStyle = p.bg;
    ctx.lineJoin = "round";
    ctx.fillStyle = p.label;
    const labelAll = nodes.length <= SMALL_VAULT;
    const baseline = 12 / view.k;
    for (const n of nodes) {
      if (n.x === undefined || n.y === undefined) continue;
      const show = labelAll || view.k >= 1.2 || (hovered !== null && hotIds!.has(n.id)) || n.degree >= 5;
      if (!show) continue;
      const label = labels.get(n.id);
      if (!label) continue;
      const hot = hotIds === null || hotIds.has(n.id);
      ctx.globalAlpha = hot ? 0.85 : 0.1;
      const y = n.y + nodeRadius(n) + baseline;
      ctx.strokeText(label.text, n.x, y);
      ctx.fillText(label.text, n.x, y);
    }
    ctx.globalAlpha = 1;
  }

  // With events firing in quick succession several graphGet calls run at once and an older response may arrive later: only the newest one lands
  let refreshSeq = 0;

  async function refresh() {
    const seq = ++refreshSeq;
    try {
      const data = await api.graphGet();
      if (seq !== refreshSeq) return;
      const merged = mergeGraph(nodes, data);
      nodes = merged.nodes;
      links = merged.links;
      adj = adjacency(links);
      measureLabels();
      nodeCount = nodes.length;
      edgeCount = links.length;
      if (!sim) {
        linkForce = forceLink<SimNode, SimLink>(links).id((d) => d.id).distance(60);
        sim = forceSimulation(nodes)
          .force("link", linkForce)
          .force("charge", forceManyBody().strength(-140))
          .force("center", forceCenter(0, 0))
          // A centring force: a note with no links at all feels only the repulsion and is pushed
          // far from the main cluster (measured: after fitting to the content the scale was
          // pressed down to 0.35 and the main cluster shrank to a handful of dots). center pulls
          // the centre of mass and cannot reach an individual node, while forceX/Y are the
          // per-node ones.
          // 0.06 is the amount that "pulls the outliers back into frame without squashing the
          // main cluster"
          .force("x", forceX(0).strength(0.06))
          .force("y", forceY(0).strength(0.06))
          .force("collide", forceCollide(12))
          // It fits on every tick until the user moves the view themselves: the layout is
          // force-directed from random initial values, where it converges is not known in
          // advance, and fitting only after convergence would miss the isolated nodes flung out
          // of frame along the way (they are already out of sight, which is the problem).
          // The price is an O(n) bounding box once per frame, the same order as drawing one.
          .on("tick", render);
      } else {
        sim.nodes(nodes);
        linkForce?.links(links);
        sim.alpha(0.3).restart();
      }
      render();
    } catch (e) {
      vault.error = String(e);
    }
  }

  $effect(() => {
    void vault.indexVersion;
    void refresh();
  });

  // A change to the privacy marks shows up at once (privatePaths is $state, and draw does not run in a reactive context)
  $effect(() => {
    void vault.privatePaths;
    draw();
  });

  onMount(() => {
    const ro = new ResizeObserver(() => {
      if (!canvas || !container) return;
      dpr = window.devicePixelRatio || 1;
      width = container.clientWidth;
      height = container.clientHeight;
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
      // On the first frame the simulation has no coordinates yet, so the origin is put at the
      // canvas's centre as a backstop; after that every size change has render fit again
      // (unless the user has already arranged the view). The palette has to be read before
      // render: the other way round, this frame would be drawn with the old colours and a theme
      // change would lag by a frame
      if (width > 0 && !fitted) {
        view.x = width / 2;
        view.y = height / 2;
      }
      palette = readPalette();
      render();
    });
    if (container) ro.observe(container);
    // Svelte 5's onwheel is passive by default and preventDefault would be ignored — so it is attached non-passive by hand
    canvas?.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      ro.disconnect();
      canvas?.removeEventListener("wheel", onWheel);
    };
  });

  onDestroy(() => sim?.stop());

  /** Double-clicking empty space resets it: the view is handed back to automatic fitting. After
   *  zooming and panning there is no way to return to the whole by hand (there is no such thing
   *  as "the original scale"). */
  function resetView() {
    userMoved = false;
    render();
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    const rect = canvas!.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    const w = toWorld(mx, my);
    userMoved = true;
    view.k = Math.min(5, Math.max(0.2, view.k * Math.exp(-e.deltaY * 0.002)));
    view.x = mx - w.x * view.k;
    view.y = my - w.y * view.k;
    draw();
  }

  function onPointerDown(e: PointerEvent) {
    const rect = canvas!.getBoundingClientRect();
    const px = e.clientX - rect.left;
    const py = e.clientY - rect.top;
    downAt = { x: e.clientX, y: e.clientY };
    moved = false;
    const hit = hitTest(px, py);
    if (hit) {
      dragging = hit;
      // Dragging a node counts as taking over the view too: were the automatic fitting still
      // running mid-drag, the transform would move under the pointer — and since fx is a world
      // coordinate converted from a screen coordinate, the node and the pointer would chase
      // each other
      userMoved = true;
      sim?.alphaTarget(0.3).restart();
    } else {
      panning = true;
    }
    canvas!.setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    const rect = canvas!.getBoundingClientRect();
    const px = e.clientX - rect.left;
    const py = e.clientY - rect.top;
    if (Math.hypot(e.clientX - downAt.x, e.clientY - downAt.y) > 4) moved = true;
    if (dragging) {
      const w = toWorld(px, py);
      dragging.fx = w.x;
      dragging.fy = w.y;
      return;
    }
    if (panning) {
      userMoved = true;
      view.x += e.movementX;
      view.y += e.movementY;
      draw();
      return;
    }
    const hit = hitTest(px, py);
    if (hit !== hovered) {
      hovered = hit;
      if (canvas) canvas.style.cursor = hit ? "pointer" : "grab";
      draw();
    }
  }

  function onPointerUp(e: PointerEvent) {
    if (dragging) {
      dragging.fx = null;
      dragging.fy = null;
      sim?.alphaTarget(0);
      if (!moved) void vault.openNote(dragging.id);
      dragging = null;
      return;
    }
    panning = false;
    if (!moved) {
      const rect = canvas!.getBoundingClientRect();
      const hit = hitTest(e.clientX - rect.left, e.clientY - rect.top);
      if (hit) void vault.openNote(hit.id);
    }
  }

  /** The pointer was cancelled by the system (a gesture took over, focus was lost): it fires no
   *  click, but the drag and pan state has to be cleared and the simulation cooled, or the node
   *  would stick to the mouse and sim would keep alphaTarget at 0.3 forever */
  function onPointerCancel() {
    if (dragging) {
      dragging.fx = null;
      dragging.fy = null;
      sim?.alphaTarget(0);
      dragging = null;
    }
    panning = false;
    moved = true;
  }
</script>

<div class="view">
  <ViewHeader title={t("view.graph")} total={t("graph.total", { nodes: nodeCount, edges: edgeCount })} hint={t("graph.hint")} />
  <div class="stage" bind:this={container}>
    <canvas
      bind:this={canvas}
      style="width: 100%; height: 100%; cursor: grab;"
      onpointerdown={onPointerDown}
      onpointermove={onPointerMove}
      onpointerup={onPointerUp}
      onpointercancel={onPointerCancel}
      ondblclick={resetView}
    ></canvas>
    {#if nodeCount === 0}
      <p class="empty">{t("graph.empty")}</p>
    {/if}
  </div>
</div>

<style>
  .view {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--fg-muted);
    font-size: var(--fs-sm);
    pointer-events: none;
  }
</style>
