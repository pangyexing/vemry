#!/usr/bin/env python3
"""Ink coverage of an activity-bar icon: how much of the 15×15 frame its strokes paint.

Run: python3 scripts/icon-ink.py 'M13.5 2.5 L6 10 h6.5 L8 15.5' 'M3.5 17.5 h13'
     python3 scripts/icon-ink.py --all      # every icon defined in ActivityBar.svelte

Each argument is one SVG path `d` (the icon's array in ActivityBar.svelte, one path
per argument). The paths are stroked at width 1.6 with round caps on the 20-unit grid
the icons are drawn on, rasterised at 16× and counted inside the [2.5, 17.5]² frame
the conventions give an icon, so the number is "how much of its frame the icon inks".
The family's range is 23%–46% (see the conventions at the top of ActivityBar.svelte);
a candidate outside it is too thin or too heavy for the rail, and one inside it still
has to pass the other three rules — ink measures how much, never what kind.

Supported commands: M L H V A C Q Z (absolute and relative); arcs and curves are
sampled into short segments.
"""
import math
import re
import sys

STROKE = 1.6
FRAME = (2.5, 17.5)
SCALE = 16


def tokens(d):
    for m in re.finditer(r"[MLHVACQZmlhvacqz]|-?\d*\.?\d+(?:e-?\d+)?", d):
        yield m.group(0)


def arc_points(p0, rx, ry, rot, large, sweep, p1, n=24):
    """SVG arc → polyline (the endpoint-to-centre conversion of the SVG spec)."""
    x0, y0 = p0
    x1, y1 = p1
    if rx == 0 or ry == 0 or (x0, y0) == (x1, y1):
        return [p1]
    phi = math.radians(rot)
    cp, sp = math.cos(phi), math.sin(phi)
    dx, dy = (x0 - x1) / 2, (y0 - y1) / 2
    x1p = cp * dx + sp * dy
    y1p = -sp * dx + cp * dy
    rx, ry = abs(rx), abs(ry)
    lam = (x1p / rx) ** 2 + (y1p / ry) ** 2
    if lam > 1:
        rx *= math.sqrt(lam)
        ry *= math.sqrt(lam)
    num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p
    den = rx * rx * y1p * y1p + ry * ry * x1p * x1p
    coef = math.sqrt(max(0.0, num / den)) if den else 0.0
    if large == sweep:
        coef = -coef
    cxp = coef * rx * y1p / ry
    cyp = -coef * ry * x1p / rx
    cx = cp * cxp - sp * cyp + (x0 + x1) / 2
    cy = sp * cxp + cp * cyp + (y0 + y1) / 2

    def ang(ux, uy, vx, vy):
        a = math.atan2(ux * vy - uy * vx, ux * vx + uy * vy)
        return a

    t1 = ang(1, 0, (x1p - cxp) / rx, (y1p - cyp) / ry)
    dt = ang((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry)
    if not sweep and dt > 0:
        dt -= 2 * math.pi
    elif sweep and dt < 0:
        dt += 2 * math.pi
    out = []
    for i in range(1, n + 1):
        t = t1 + dt * i / n
        x = cp * rx * math.cos(t) - sp * ry * math.sin(t) + cx
        y = sp * rx * math.cos(t) + cp * ry * math.sin(t) + cy
        out.append((x, y))
    return out


def bezier_points(pts, n=24):
    """A quadratic (3 points) or cubic (4 points) Bézier → polyline."""
    out = []
    for i in range(1, n + 1):
        t = i / n
        p = list(pts)
        while len(p) > 1:
            p = [((1 - t) * a[0] + t * b[0], (1 - t) * a[1] + t * b[1]) for a, b in zip(p, p[1:])]
        out.append(p[0])
    return out


def segments(d):
    """The stroked polylines of one path (a list of point lists)."""
    toks = list(tokens(d))
    i = 0
    cmd = None
    cur = (0.0, 0.0)
    start = (0.0, 0.0)
    lines = []
    line = []

    def num():
        nonlocal i
        v = float(toks[i])
        i += 1
        return v

    def begin(p):
        nonlocal line
        if line:
            lines.append(line)
        line = [p]

    while i < len(toks):
        if re.match(r"[A-Za-z]", toks[i]):
            cmd = toks[i]
            i += 1
        rel = cmd.islower()
        c = cmd.upper()
        ox, oy = cur if rel else (0.0, 0.0)
        if c == "M":
            cur = (num() + ox, num() + oy)
            start = cur
            begin(cur)
            cmd = "l" if rel else "L"  # subsequent pairs are implicit line-tos
        elif c == "L":
            cur = (num() + ox, num() + oy)
            line.append(cur)
        elif c == "H":
            cur = (num() + ox, cur[1])
            line.append(cur)
        elif c == "V":
            cur = (cur[0], num() + oy)
            line.append(cur)
        elif c == "A":
            rx, ry, rot, large, sweep = num(), num(), num(), num(), num()
            end = (num() + ox, num() + oy)
            line.extend(arc_points(cur, rx, ry, rot, int(large), int(sweep), end))
            cur = end
        elif c == "C":
            p1 = (num() + ox, num() + oy)
            p2 = (num() + ox, num() + oy)
            end = (num() + ox, num() + oy)
            line.extend(bezier_points([cur, p1, p2, end]))
            cur = end
        elif c == "Q":
            p1 = (num() + ox, num() + oy)
            end = (num() + ox, num() + oy)
            line.extend(bezier_points([cur, p1, end]))
            cur = end
        elif c == "Z":
            line.append(start)
            cur = start
        else:
            raise SystemExit(f"unsupported command {cmd} in {d!r}")
    if line:
        lines.append(line)
    return lines


def dist_to_segment(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    if dx == 0 and dy == 0:
        return math.hypot(px - ax, py - ay)
    t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
    return math.hypot(px - (ax + t * dx), py - (ay + t * dy))


def coverage(paths):
    """The share of the frame's area within half a stroke of any segment."""
    segs = []
    for d in paths:
        for line in segments(d):
            segs.extend(zip(line, line[1:]))
    lo, hi = FRAME
    n = int((hi - lo) * SCALE)
    half = STROKE / 2
    inked = 0
    for iy in range(n):
        py = lo + (iy + 0.5) / SCALE
        for ix in range(n):
            px = lo + (ix + 0.5) / SCALE
            if any(dist_to_segment(px, py, a[0], a[1], b[0], b[1]) <= half for a, b in segs):
                inked += 1
    return inked / (n * n)


def icons_from_svelte(path="src/lib/components/ActivityBar.svelte"):
    """Every `name: [ "d", ... ]` and `const nameIcon = [ ... ]` array in the file."""
    src = open(path, encoding="utf-8").read()
    found = {}
    for m in re.finditer(r"(?:^\s*(\w+):\s*\[|const\s+(\w+)\s*=\s*\[)((?:[^\[\]]|\n)*?)\]", src, re.M):
        name = m.group(1) or m.group(2)
        ds = re.findall(r'"([^"]+)"', m.group(3))
        if ds and all(re.match(r"^[Mm]", d) for d in ds):
            found[name] = ds
    return found


def main(argv):
    if argv[:1] == ["--all"]:
        for name, ds in icons_from_svelte().items():
            print(f"{name:12s} {coverage(ds) * 100:5.1f}%  ({len(ds)} path{'s' if len(ds) != 1 else ''})")
        return
    if not argv:
        print(__doc__)
        raise SystemExit(2)
    print(f"{coverage(argv) * 100:.1f}%")


if __name__ == "__main__":
    main(sys.argv[1:])
