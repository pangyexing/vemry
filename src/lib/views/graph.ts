// Graph-view simulation data. d3-force does layout math only; rendering is
// hand-drawn Canvas in GraphView.svelte.
import type { SimulationNodeDatum } from "d3-force";
import type { GraphData } from "$lib/api";

export interface SimNode extends SimulationNodeDatum {
  id: string;
  title: string;
  degree: number;
}

export interface SimLink {
  /** d3-force forceLink mutates these from id strings into node refs */
  source: string | SimNode;
  target: string | SimNode;
  count: number;
}

/**
 * Merge fresh graph data into the existing simulation nodes, keeping each
 * surviving node's position and velocity — the key to a stable layout across
 * incremental refreshes. New nodes spawn near an already-placed neighbor
 * (falling back to the origin) with jitter so they don't stack.
 */
export function mergeGraph(prev: SimNode[], data: GraphData): { nodes: SimNode[]; links: SimLink[] } {
  const prevById = new Map(prev.map((n) => [n.id, n]));
  const degree = new Map<string, number>();
  const neighbors = new Map<string, string[]>();
  const addNeighbor = (a: string, b: string) => {
    const arr = neighbors.get(a);
    if (arr) arr.push(b);
    else neighbors.set(a, [b]);
  };
  for (const e of data.edges) {
    degree.set(e.source, (degree.get(e.source) ?? 0) + 1);
    degree.set(e.target, (degree.get(e.target) ?? 0) + 1);
    addNeighbor(e.source, e.target);
    addNeighbor(e.target, e.source);
  }
  const jitter = () => (Math.random() - 0.5) * 40;
  const nodes: SimNode[] = data.nodes.map((n) => {
    const old = prevById.get(n.id);
    if (old) {
      old.title = n.title;
      old.degree = degree.get(n.id) ?? 0;
      return old;
    }
    const anchor = (neighbors.get(n.id) ?? [])
      .map((id) => prevById.get(id))
      .find((x) => x !== undefined && x.x !== undefined);
    return {
      id: n.id,
      title: n.title,
      degree: degree.get(n.id) ?? 0,
      x: (anchor?.x ?? 0) + jitter(),
      y: (anchor?.y ?? 0) + jitter(),
    };
  });
  const links: SimLink[] = data.edges.map((e) => ({ source: e.source, target: e.target, count: e.count }));
  return { nodes, links };
}

/** Undirected adjacency for hover highlighting. */
export function adjacency(links: SimLink[]): Map<string, Set<string>> {
  const adj = new Map<string, Set<string>>();
  const idOf = (x: string | SimNode) => (typeof x === "string" ? x : x.id);
  for (const l of links) {
    const s = idOf(l.source);
    const t = idOf(l.target);
    (adj.get(s) ?? adj.set(s, new Set()).get(s)!).add(t);
    (adj.get(t) ?? adj.set(t, new Set()).get(t)!).add(s);
  }
  return adj;
}

/** Node display radius in world units. */
export function nodeRadius(n: SimNode): number {
  return 3 + 2 * Math.sqrt(n.degree);
}
