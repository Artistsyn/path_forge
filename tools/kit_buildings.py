"""Buildings for the settlement kit, drawn from a few measurements: cottages of several builds and
log cabins. Each is a front picture (its gable end, facing the path) and faces: the long side
walls, the slopes of the roof with their eaves, chimneys, a porch. Used by make_kits.py.

Units are metres. In a front picture x runs across the front from its middle and y up from the
ground; faces stand in the building's space, x across the front, y up and z back from the path,
each placed by its picture's top left, top right and bottom left corners as seen from outside,
its picture painted from its own bottom left corner.
"""
import math, random

def r3(v): return round(v, 3)
def rect(x0, y0, x1, y1, col, shade=0.5, glow=0.0):
    p = {"shape": {"Rect": {"min": [r3(x0), r3(y0)], "max": [r3(x1), r3(y1)]}}, "color": col, "shade": shade}
    if glow: p["glow"] = glow
    return p
def ell(cx, cy, rx, ry, col, shade=0.5, glow=0.0):
    p = {"shape": {"Ellipse": {"center": [r3(cx), r3(cy)], "radius": [r3(rx), r3(ry)]}}, "color": col, "shade": shade}
    if glow: p["glow"] = glow
    return p
def poly(pts, col, shade=0.5):
    return {"shape": {"Poly": {"points": [[r3(x), r3(y)] for x, y in pts]}}, "color": col, "shade": shade}
def line(a, b, w, col, shade=0.3):
    return {"shape": {"Line": {"from": [r3(a[0]), r3(a[1])], "to": [r3(b[0]), r3(b[1])], "width": r3(w)}}, "color": col, "shade": shade}
def mix(a, b, t): return [round(a[i] + (b[i] - a[i]) * t) for i in range(3)]
def dark(c, t): return mix(c, [0, 0, 0], t)
def light(c, t): return mix(c, [255, 255, 255], t)
def face(tl, tr, bl, shape, mirror=False):
    f = {"corners": [[r3(v) for v in tl], [r3(v) for v in tr], [r3(v) for v in bl]], "shape": shape}
    if mirror: f["mirror"] = True
    return f

WARM = [255, 196, 112]
GLASS = [46, 56, 66]

# ── Walls ──────────────────────────────────────────────────────────────────
# A wall fills x0..x1 by 0..top, with its openings (doors, windows: (x0, y0, x1, y1)) left for
# them to be drawn over. `clip(x)` is the height the wall reaches at x (a gable's slope), or None
# for a straight top.

def courses(x0, x1, y0, y1, clip, h, col, gap, shade, jitter, rnd, stagger=0.45, joints=True, vary=0.0):
    """Coursed stone, brick or boards: rows h high with joints every `stagger`, offset row to row."""
    out, y, row = [], y0, 0
    while y < y1 - 1e-3:
        yt = min(y + h, y1)
        x = x0 - (stagger * 0.5 if row % 2 else 0.0)
        while x < x1 - 1e-3:
            w = stagger * (1.0 + (rnd.random() - 0.5) * jitter)
            a, b = max(x, x0), min(x + w, x1)
            top = yt
            if clip:
                top = min(yt, clip(a), clip(b))
            if b - a > 0.04 and top - y > 0.04:
                c = mix(col, dark(col, 0.25) if rnd.random() < 0.5 else light(col, 0.18), rnd.random() * vary) if vary else col
                out.append(rect(a + gap * 0.5, y + gap * 0.5, b - gap * 0.5, top - gap * 0.5, c, shade))
            x += w
        y, row = yt, row + 1
    return out

def tri_clip(cx, half, base, apex):
    """The height of a gable at x: rising from `base` at cx +- half to `apex` at cx."""
    return lambda x: base + (apex - base) * max(0.0, 1.0 - abs(x - cx) / half)

def wall(sp, x0, x1, top, openings, posts=None, clip=None, base=0.0, rnd=None):
    st = sp["wall"]
    rnd = rnd or random.Random(7)
    s = []
    plinth = sp.get("plinth", 0.0) if base == 0.0 else 0.0
    y0 = base
    def outline(col, shade=0.45):
        if clip:
            n = 24
            xs = [x0 + (x1 - x0) * i / n for i in range(n + 1)]
            return poly([(x0, y0)] + [(x, clip(x)) for x in xs] + [(x1, y0)], col, shade)
        return rect(x0, y0, x1, top, col, shade)
    if st == "timber":
        s.append(outline(sp["fill"]))
        if plinth: s += [rect(x0, 0, x1, plinth, sp["stone"], 0.6)] + [line([x, 0.06], [x + 0.22, plinth - 0.06], 0.04, dark(sp["stone"], 0.35)) for x in frange(x0 + 0.3, x1 - 0.3, 0.9)]
        tim, lo = sp["timber"], max(plinth, y0)
        if clip:
            # A gable: studs either side of the attic window, a strut out to each corner.
            for x in [-0.75, 0.75]:
                s.append(line([x, lo], [x, clip(x) - 0.02], 0.13, tim))
            s += [line([-0.75, lo + 0.1], [x0 + 0.5, lo + 0.9 * (clip(x0 + 0.5) - lo)], 0.11, tim), line([0.75, lo + 0.1], [x1 - 0.5, lo + 0.9 * (clip(x1 - 0.5) - lo)], 0.11, tim)]
            return s
        sill, head = sp["sill"], sp["head"]
        posts = posts or [x0 + 0.08, x1 - 0.08]
        for x in posts: s.append(line([x, lo], [x, top], 0.16, tim))
        def rail(y, w=0.16):
            cuts = sorted((a, b) for a, ya, b, yb in openings if ya + 0.02 < y < yb - 0.02)
            x = x0
            for a, b in cuts:
                if a - x > 0.05: s.append(line([x, y], [a, y], w, tim))
                x = max(x, b)
            if x1 - x > 0.05: s.append(line([x, y], [x1, y], w, tim))
        for y in [lo + 0.03, sill, head, top - 0.05]: rail(y, 0.16 if y in (lo + 0.03, top - 0.05) else 0.13)
        # Braces in the bays with nothing in them, leaning out toward the nearer end of the wall;
        # a cross in the panel under each window.
        mid = (x0 + x1) / 2
        for a, b in zip(posts, posts[1:]):
            busy = [o for o in openings if o[0] < b - 0.05 and o[2] > a + 0.05]
            if not busy:
                inner, outer = (b, a) if (a + b) / 2 < mid else (a, b)
                s.append(line([inner, lo + 0.08], [outer, top - 0.1], 0.12, tim))
            else:
                for ox0, oy0, ox1, oy1 in busy:
                    if oy0 > lo + 0.4 and sp.get("crosses", True):
                        s += [line([ox0, lo + 0.06], [ox1, oy0 - 0.06], 0.09, tim), line([ox1, lo + 0.06], [ox0, oy0 - 0.06], 0.09, tim)]
        return s
    if st in ("stone", "brick", "board", "white", "log"):
        if st == "stone":
            s.append(outline(sp["mortar"], 0.3))
            s += courses(x0, x1, y0, top, clip, 0.27, sp["stone"], 0.05, 0.6, 0.7, rnd, stagger=0.5, vary=0.8)
            if not clip:
                # Quoins: dressed stones at the corners, long and short by turns.
                for i, y in enumerate(frange(y0, top - 0.1, 0.32)):
                    w = 0.5 if i % 2 else 0.32
                    s += [rect(x0, y + 0.02, x0 + w, y + 0.3, sp["quoin"], 0.55), rect(x1 - w, y + 0.02, x1, y + 0.3, sp["quoin"], 0.55)]
        elif st == "brick":
            s.append(outline(sp["mortar"], 0.3))
            s += courses(x0, x1, y0, top, clip, 0.15, sp["brick"], 0.025, 0.4, 0.15, rnd, stagger=0.42, vary=0.5)
            if plinth: s.append(rect(x0, 0, x1, plinth, dark(sp["brick"], 0.3), 0.4))
        elif st == "board":
            s.append(outline(sp["paint"], 0.4))
            if clip:
                for x in frange(x0 + 0.12, x1 - 0.12, 0.28): s.append(line([x, y0], [x, clip(x) - 0.02], 0.05, dark(sp["paint"], 0.22)))
            else:
                for y in frange(y0 + 0.2, top, 0.2): s.append(line([x0, y], [x1, y], 0.035, dark(sp["paint"], 0.3)))
                s += [rect(x0, y0, x0 + 0.13, top, sp["trim"], 0.4), rect(x1 - 0.13, y0, x1, top, sp["trim"], 0.4)]
            if plinth: s.append(rect(x0, 0, x1, plinth, sp["stone"], 0.6))
        elif st == "white":
            s.append(outline(sp["fill"], 0.35))
            if not clip:
                s.append(rect(x0, plinth, x1, plinth + 0.3, mix(sp["fill"], [120, 116, 104], 0.18), 0.3))
                if plinth: s.append(rect(x0, 0, x1, plinth, sp["tar"], 0.4))
        elif st == "log":
            s += logs(sp, x0, x1, y0, top, clip, 0.0)
    return s

def frange(a, b, step):
    out, x = [], a
    while x <= b + 1e-6:
        out.append(x)
        x += step
    return out

def logs(sp, x0, x1, y0, top, clip, ends, offset=0.0):
    """Round logs laid one on another, their tails running `ends` past the corners (their cut
    ends are faces of their own, `log_corners`)."""
    s, h = [], sp["log_h"]
    col, end, gap = sp["log"], sp["log_end"], dark(sp["log"], 0.55)
    y = y0 - offset
    # (The chinking behind runs out to the stubs, so nothing shows through between them.)
    s.append(rect(x0 - ends, max(y0, 0), x1 + ends, top, gap, 0.2) if not clip else poly([(x0, y0)] + [(x0 + (x1 - x0) * i / 16, clip(x0 + (x1 - x0) * i / 16)) for i in range(17)] + [(x1, y0)], gap, 0.2))
    while y < top - 1e-3:
        a, b = max(y, y0), min(y + h, top)
        if b - a > 0.03:
            if clip:
                ya, yb = a + 0.015, b - 0.015
                # The course as wide as the gable is at its height.
                def edge(yy):
                    lo, hi = x0, 0.0
                    for _ in range(30):
                        m = (lo + hi) / 2
                        if clip(m) < yy: lo = m
                        else: hi = m
                    return -lo if x0 < 0 else lo
                wa, wb = abs(edge(ya)), abs(edge(yb))
                if wb > 0.05: s.append(poly([(-wa, ya), (wa, ya), (wb, yb), (-wb, yb)], col, 0.75))
            else:
                s += log_band(x0 - ends, x1 + ends, a, b, col)
        y += h
    return s

def roundness(v):
    """How a round log is shaded across its height, from its underside (v = 0) to its top (1):
    the shadowed groove where it sits on the log below, lit across its upper curve, rolling away
    a little at the top. The renderer's tubes (render::log_shade) shade by the same curve, so a
    painted log and its cylinder tail match."""
    return (0.25 + 0.75 * math.sqrt(max(math.sin(math.pi * v), 0.0))) * (0.8 + 0.4 * v)

def to_lin(c): return [(x / 255 / 12.92) if x / 255 <= 0.04045 else ((x / 255 + 0.055) / 1.055) ** 2.4 for x in c]
def to_srgb(l): return [max(0, min(255, int((v * 12.92 if v <= 0.0031308 else 1.055 * v ** (1 / 2.4) - 0.055) * 255 + 0.5))) for v in l]

def log_band(x0, x1, a, b, col, steps=12):
    """One round log seen from the side, painted flat in `steps` bands by `roundness`, the
    colours worked in linear light as the renderer lights them."""
    lin = to_lin(col)
    out = []
    for i in range(steps):
        v = (i + 0.5) / steps
        f = roundness(v)
        out.append(rect(x0, a + (b - a) * i / steps, x1, a + (b - a) * (i + 1) / steps + 0.002, to_srgb([c * f for c in lin]), 0.0))
    return out

def end_grain(sp, cx, cy, r, rnd):
    """A log's sawn end, weathered: a rim of bark, growth rings round an off-centre pith, and the
    checks that open as it dries, radiating out from the pith."""
    end = mix(sp["log_end"], sp["log"], 0.3)
    px, py = cx + rnd.uniform(-0.12, 0.12) * r, cy + rnd.uniform(-0.12, 0.12) * r
    s = [ell(cx, cy, r, r, dark(sp["log"], 0.35), 0.5), ell(cx, cy, r * 0.88, r * 0.88, dark(end, 0.12), 0.45)]
    for i, f in enumerate([0.76, 0.6, 0.45, 0.3, 0.16]):
        s.append(ell(px + (cx - px) * f, py + (cy - py) * f, r * f, r * f, dark(end, 0.14) if i % 2 == 0 else light(end, 0.08), 0.35))
    s.append(ell(px, py, r * 0.06, r * 0.06, dark(end, 0.4), 0.2))
    # Checks open from the bark in toward the pith, partway.
    for _ in range(rnd.choice([0, 1, 1, 2])):
        a = rnd.uniform(0, 2 * math.pi)
        f = rnd.uniform(0.3, 0.6)
        s.append(line([cx + 0.88 * r * math.cos(a), cy + 0.88 * r * math.sin(a)], [cx + (0.88 - f) * r * math.cos(a), cy + (0.88 - f) * r * math.sin(a)], r * 0.05, dark(end, 0.55), 0.2))
    return s

def log_corners(sp, hw, D, wy, e, k):
    """The corners of a round-log cabin, saddle-notched, as real cylinders: each wall's logs run
    on past the corner as tails `e` beyond the other wall, the two walls' logs a half log apart
    in height, so the tails stack out of the corner both ways by turns, each with its sawn end.
    The walls' pictures are their outer faces, so a log's axis lies a radius inside them (and a
    hair more, so a tail never ties with the picture it runs behind)."""
    h = sp["log_h"]
    r = h / 2
    a = r + 0.012
    front = [(k + 0.5) * h for k in range(50) if (k + 0.5) * h < wy - 0.04]
    side = [k * h for k in range(0, 50) if k * h < wy - 0.04]
    end = mix(sp["log_end"], sp["log"], 0.25)
    bark = sp["log"]  # (shaded as the painted logs are: `roundness`)
    def tube(p, q):
        return {"from": [r3(v) for v in p], "to": [r3(v) for v in q], "radius": r3(r * 0.97), "color": bark, "end": end, "mirror": True}
    # Under the eaves the roof runs out over the tails (its underside falls `k` a metre outward
    # from the wall's top at `wy`): every tail is cut to one length, short enough for all of them
    # to tuck under it. A log with no room at all (its top above the wall's) ends flush.
    room = [(wy - 0.03 - (y + r)) / k if k > 0 else e for y in front]
    e = min([e] + [m for m in room if m >= 0.08])
    # Each log from a little inside where its wall's picture stops (a log short of the corner),
    # through the corner, and out as its tail; one with no room for a tail ends flush.
    c = h + 0.05
    out = []
    for y, m in zip(front, room):
        t = e if m >= 0.08 else 0.0
        out.append(tube([hw - c, y, a], [hw + t, y, a]))                  # the front wall's, out past the side
        out.append(tube([hw - c, y, D - a], [hw + t, y, D - a]))          # the back wall's
    for y in side:
        out.append(tube([hw - a, y, c], [hw - a, y, -e]))                 # the side wall's, out toward the path
        out.append(tube([hw - a, y, D - c], [hw - a, y, D + e]))          # and out the back
    return out

# ── Openings ───────────────────────────────────────────────────────────────

def window(sp, cx, y0, w, h, lit=True, panes=(2, 2)):
    trim = sp.get("frame", sp.get("timber", [74, 52, 36]))
    s = []
    if sp.get("lintel"): s.append(rect(cx - w / 2 - 0.12, y0 + h, cx + w / 2 + 0.12, y0 + h + 0.18, sp["lintel"], 0.55))
    if sp.get("shutters"):
        for side in [-1, 1]:
            a = cx + side * (w / 2 + 0.02)
            b = a + side * w * 0.48
            s.append(rect(min(a, b), y0 - 0.02, max(a, b), y0 + h + 0.02, sp["shutters"], 0.5))
            for yy in frange(y0 + 0.12, y0 + h - 0.08, 0.14): s.append(line([min(a, b) + 0.04, yy], [max(a, b) - 0.04, yy], 0.02, dark(sp["shutters"], 0.35)))
    s.append(rect(cx - w / 2, y0, cx + w / 2, y0 + h, trim, 0.3))
    f = 0.07
    s.append(rect(cx - w / 2 + f, y0 + f, cx + w / 2 - f, y0 + h - f, WARM if lit else GLASS, 0.0 if lit else 0.5, glow=2.3 if lit else 0.0))
    if not lit: s.append(line([cx - w / 2 + 0.12, y0 + h - 0.15], [cx - w / 2 + 0.3, y0 + h - 0.32], 0.05, light(GLASS, 0.35)))
    cols, rows = panes
    for i in range(1, cols): s.append(line([cx - w / 2 + w * i / cols, y0 + f], [cx - w / 2 + w * i / cols, y0 + h - f], 0.045, trim))
    for j in range(1, rows): s.append(line([cx - w / 2 + f, y0 + h * j / rows], [cx + w / 2 - f, y0 + h * j / rows], 0.045, trim))
    s.append(rect(cx - w / 2 - 0.07, y0 - 0.08, cx + w / 2 + 0.07, y0, sp.get("sill_col", dark(trim, 0.2)), 0.35))
    return s

def door(sp, cx, w, h, y0=0.0):
    trim, col = sp.get("frame", sp.get("timber", [74, 52, 36])), sp["door"]
    s = []
    if y0 > 0: s.append(rect(cx - w / 2 - 0.15, 0, cx + w / 2 + 0.15, y0, sp.get("step", [128, 122, 112]), 0.6))
    if sp.get("lintel"): s.append(rect(cx - w / 2 - 0.12, y0 + h, cx + w / 2 + 0.12, y0 + h + 0.18, sp["lintel"], 0.55))
    s += [rect(cx - w / 2 - 0.07, y0, cx + w / 2 + 0.07, y0 + h + 0.07, trim, 0.3), rect(cx - w / 2, y0, cx + w / 2, y0 + h, col, 0.5)]
    if sp.get("fanlight"):
        s += [rect(cx - w / 2 + 0.05, y0 + h - 0.3, cx + w / 2 - 0.05, y0 + h - 0.05, WARM, 0.0, glow=1.8), line([cx - w / 2, y0 + h - 0.32], [cx + w / 2, y0 + h - 0.32], 0.05, trim)]
        s += [rect(cx - w / 2 + 0.08, y0 + 0.12, cx + w / 2 - 0.08, y0 + h * 0.45, dark(col, 0.15), 0.4), rect(cx - w / 2 + 0.08, y0 + h * 0.5, cx + w / 2 - 0.08, y0 + h - 0.42, dark(col, 0.15), 0.4)]
    else:
        for x in frange(cx - w / 2 + w / 4, cx + w / 2 - 0.05, w / 4): s.append(line([x, y0 + 0.03], [x, y0 + h - 0.03], 0.025, dark(col, 0.3)))
        s += [line([cx - w / 2, y0 + 0.35 * h], [cx + w / 2, y0 + 0.35 * h], 0.05, dark(col, 0.3)), line([cx - w / 2, y0 + 0.75 * h], [cx + w / 2, y0 + 0.75 * h], 0.05, dark(col, 0.3))]
    s.append(ell(cx + w * 0.32, y0 + h * 0.48, 0.035, 0.035, [40, 36, 30], 0.6))
    return s

# ── Roofs ──────────────────────────────────────────────────────────────────

def roof_picture(sp, W, L, rnd):
    kind, col = sp["roof"], sp["roof_col"]
    s = [rect(0, 0, W, L, col, 0.4)]
    if kind in ("tile", "slate", "shingle"):
        h = {"tile": 0.3, "slate": 0.24, "shingle": 0.2}[kind]
        joint = {"tile": 0.42, "slate": 0.34, "shingle": 0.24}[kind]
        y, k = 0.1, 0
        while y < L - 0.18:
            s.append(rect(0, y, W, y + 0.05, dark(col, 0.45), 0.2))
            x = (joint * 0.5 if k % 2 else 0.0) + (rnd.random() * joint * 0.3 if kind == "shingle" else 0.0)
            while x < W:
                s.append(line([x, y + 0.05], [x, y + h], 0.025, dark(col, 0.35)))
                if rnd.random() < 0.18:
                    s.append(rect(x + 0.03, y + 0.06, min(W, x + joint - 0.03), y + h - 0.01, mix(col, light(col, 0.2) if rnd.random() < 0.5 else dark(col, 0.18), 0.8), 0.4))
                x += joint * (1.0 + ((rnd.random() - 0.5) * 0.5 if kind == "shingle" else 0.0))
            y, k = y + h, k + 1
        if kind == "tile":
            for _ in range(5): s.append(ell(rnd.uniform(0.3, W - 0.3), rnd.uniform(0.2, L * 0.6), rnd.uniform(0.2, 0.45), rnd.uniform(0.08, 0.18), [94, 104, 62], 0.3))
        s += [rect(0, L - 0.16, W, L, dark(col, 0.4), 0.5), rect(0, 0, W, 0.06, dark(col, 0.6), 0.2)]
    elif kind == "thatch":
        for i in range(int(W * L * 5)):
            x, y = rnd.uniform(0, W), rnd.uniform(0, L - 0.4)
            c = mix(col, dark(col, 0.3) if rnd.random() < 0.6 else light(col, 0.15), rnd.uniform(0.3, 1.0))
            s.append(line([x, y], [x + rnd.uniform(-0.04, 0.04), y + rnd.uniform(0.25, 0.5)], 0.035, c))
        # The ridge, its straw bound down in a scalloped pattern.
        s.append(rect(0, L - 0.45, W, L, dark(col, 0.12), 0.5))
        for x in frange(0.0, W, 0.4): s.append(line([x, L - 0.42], [x + 0.2, L - 0.12], 0.04, dark(col, 0.4)))
        for x in frange(0.2, W, 0.4): s.append(line([x, L - 0.12], [x + 0.2, L - 0.42], 0.04, dark(col, 0.4)))
        s.append(rect(0, 0, W, 0.08, dark(col, 0.35), 0.3))
    elif kind == "sod":
        for i in range(int(W * L * 6)):
            x, y = rnd.uniform(0, W), rnd.uniform(0, L)
            s.append(ell(x, y, rnd.uniform(0.08, 0.2), rnd.uniform(0.04, 0.09), mix(col, light(col, 0.25) if rnd.random() < 0.5 else dark(col, 0.25), rnd.random()), 0.5))
        for i in range(int(W * L * 1.2)):
            s.append(ell(rnd.uniform(0, W), rnd.uniform(0.1, L), 0.035, 0.035, rnd.choice([[236, 220, 90], [240, 240, 236], [200, 120, 180]]), 0.4))
        s.append(rect(0, 0, W, 0.1, sp.get("birch", [220, 214, 200]), 0.4))
    return s

def edge_picture(sp, W, H):
    """The eave's edge: the roof's thickness seen end on."""
    kind, col = sp["roof"], sp["roof_col"]
    if kind == "thatch":
        s = [rect(0, 0, W, H, dark(col, 0.15), 0.5)]
        for x in frange(0.03, W, 0.07): s.append(line([x, 0.02], [x, H - 0.03], 0.03, dark(col, 0.4)))
        return s
    if kind == "sod":
        return [rect(0, 0, W, H, [70, 50, 34], 0.4), rect(0, H * 0.55, W, H, sp["roof_col"], 0.5), rect(0, 0, W, H * 0.25, sp.get("birch", [220, 214, 200]), 0.4)]
    return [rect(0, 0, W, H, sp.get("barge", dark(col, 0.45)), 0.4)]

def under_picture(sp, W, H):
    s = [rect(0, 0, W, H, [58, 44, 34], 0.3)]
    for x in frange(0.3, W, 0.6): s.append(line([x, 0], [x, H], 0.08, [40, 30, 24]))
    return s

def masonry_picture(sp, W, H, rnd, cap=True):
    kind = sp.get("chimney_kind", "brick")
    if kind == "stone":
        s = [rect(0, 0, W, H, [96, 92, 86], 0.3)] + courses(0, W, 0, H, None, 0.24, sp.get("chimney_col", [140, 134, 122]), 0.04, 0.6, 0.8, rnd, stagger=0.36, vary=0.8)
    elif kind == "plaster":
        s = [rect(0, 0, W, H, sp.get("chimney_col", [228, 222, 206]), 0.4)]
    else:
        s = [rect(0, 0, W, H, [118, 104, 92], 0.3)] + courses(0, W, 0, H, None, 0.15, sp.get("chimney_col", [140, 74, 56]), 0.025, 0.4, 0.15, rnd, stagger=0.3, vary=0.5)
    if cap: s.append(rect(0, H - 0.12, W, H, [96, 90, 84], 0.5))
    return s

def box_faces(sp, x0, x1, z0, z1, y0, y1, rnd, skip=()):
    """A chimney stack: its four sides and its top with the flue."""
    w, dz, h = x1 - x0, z1 - z0, y1 - y0
    out = []
    if "front" not in skip: out.append(face([x0, y1, z0], [x1, y1, z0], [x0, y0, z0], masonry_picture(sp, w, h, rnd)))
    if "back" not in skip: out.append(face([x1, y1, z1], [x0, y1, z1], [x1, y0, z1], masonry_picture(sp, w, h, rnd)))
    if "right" not in skip: out.append(face([x1, y1, z0], [x1, y1, z1], [x1, y0, z0], masonry_picture(sp, dz, h, rnd)))
    if "left" not in skip: out.append(face([x0, y1, z1], [x0, y1, z0], [x0, y0, z1], masonry_picture(sp, dz, h, rnd)))
    out.append(face([x0, y1, z1], [x1, y1, z1], [x0, y1, z0], [rect(0, 0, w, dz, [96, 90, 84], 0.5), rect(0.12, 0.12, w - 0.12, dz - 0.12, [24, 20, 18], 0.0)]))
    return out

# ── A building ─────────────────────────────────────────────────────────────

def building(sp, seed):
    rnd = random.Random(seed)
    hw, E, R, D = sp["hw"], sp["E"], sp["R"], sp["D"]
    o, oz, t = sp.get("o", 0.5), sp.get("oz", 0.15), sp.get("t", 0.16)
    wy = E + 0.05
    k = (R - wy) / hw
    ex = hw + o
    ey = wy - o * k
    tv = t * math.hypot(1.0, k)
    tail = sp.get("ends", 0.0)
    ends = 0.0  # (the pictures stop at the corners: a log cabin's tails are tubes)
    # The front: the gable end, with its door and windows.
    openings = [(cx - w / 2, y0, cx + w / 2, y0 + h) for (cx, y0, w, h, *_ ) in sp["front_windows"]]
    dcx, dw, dh, dy = sp["door_at"]
    openings.append((dcx - dw / 2, dy, dcx + dw / 2, dy + dh))
    # (A log wall's picture stops a log short of each corner: there the logs themselves, tubes,
    # pass through one another.)
    c = sp["log_h"] if sp["wall"] == "log" else 0.0
    s = wall(sp, -hw + c, hw - c, E, openings, posts=sp.get("front_posts"), rnd=rnd)
    gable = tri_clip(0.0, hw, wy, R)
    gsp = dict(sp, wall=sp.get("gable", sp["wall"]))
    s += wall(gsp, -hw, hw, R, [], clip=gable, base=E - 0.01, rnd=rnd)
    for (cx, y0, w, h, *rest) in sp["front_windows"]: s += window(sp, cx, y0, w, h, lit=rest[0] if rest else True, panes=sp.get("panes", (2, 2)))
    s += door(sp, dcx, dw, dh, dy)
    if sp.get("attic"):
        ay = (E + R) / 2 - 0.1
        if sp["attic"] == "round":
            trim = sp.get("frame", sp.get("timber", [74, 52, 36]))
            s += [ell(0, ay, 0.32, 0.32, trim, 0.3), ell(0, ay, 0.25, 0.25, WARM, 0.0, glow=2.0), line([-0.25, ay], [0.25, ay], 0.04, trim), line([0, ay - 0.25], [0, ay + 0.25], 0.04, trim)]
        else:
            s += window(sp, 0, ay - 0.35, 0.6, 0.7, lit=sp.get("attic_lit", True), panes=(2, 2))
    # The roof's edges over the gable.
    barge = sp.get("barge", dark(sp["roof_col"], 0.45))
    for side in [-1, 1]:
        s.append(poly([(side * ex, ey), (0, R), (0, R + tv), (side * ex, ey + tv)], barge, 0.4))
        if sp["roof"] == "sod":
            for i in range(14):
                f = i / 13
                s.append(ell(side * ex * (1 - f), ey + tv + (R - ey) * f, 0.13, 0.07, sp["roof_col"], 0.5))
    # Faces: the long walls, the roof's slopes, their eaves and undersides.
    W = D + 2 * oz
    L = math.hypot(ex, R - ey)
    side_open = [(z - w / 2, y0, z + w / 2, y0 + h) for (z, y0, w, h, *_) in sp["side_windows"]]
    ssp = dict(sp)
    if sp["wall"] == "log":
        side = [shift(p, -c) for p in logs(sp, c, D - c, 0, wy, None, 0.0, offset=sp["log_h"] / 2)]
    else:
        side = wall(ssp, 0, D, wy, side_open, posts=sp.get("side_posts"), rnd=rnd)
    for (z, y0, w, h, *rest) in sp["side_windows"]: side += [shift(p, -c) for p in window(sp, z, y0, w, h, lit=rest[0] if rest else True, panes=sp.get("panes", (2, 2)))]
    faces = [face([hw, wy, c], [hw, wy, D - c], [hw, 0, c], side, mirror=True),
             face([0, R + tv, -oz], [0, R + tv, D + oz], [ex, ey + tv, -oz], roof_picture(sp, W, L, rnd), mirror=True),
             face([ex, ey + tv, -oz], [ex, ey + tv, D + oz], [ex, ey, -oz], edge_picture(sp, W, tv), mirror=True),
             face([hw, wy, -oz], [hw, wy, D + oz], [ex, ey, -oz], under_picture(sp, W, math.hypot(o, wy - ey)), mirror=True)]
    tubes = log_corners(sp, hw, D, wy, tail, k) if sp["wall"] == "log" else []
    smoke = None
    for ch in sp.get("chimneys", []):
        x, z, w, dz, top = ch["x"], ch["z"], ch["w"], ch["dz"], ch["top"]
        if ch.get("outside"):
            # A stack built up outside a side wall, from the ground.
            faces += box_faces(dict(sp, chimney_kind=ch.get("kind", "stone")), hw, hw + w, z - dz / 2, z + dz / 2, 0.0, top, rnd, skip=("left",))
            sx = hw + w / 2
        else:
            low = R - k * (abs(x) + w / 2) - 0.25
            faces += box_faces(dict(sp, chimney_kind=ch.get("kind", sp.get("chimney_kind", "brick"))), x - w / 2, x + w / 2, z - dz / 2, z + dz / 2, low, top, rnd)
            sx = x
        if smoke is None and ch.get("smoke", True):
            smoke = {"enabled": True, "at": [r3(sx), r3(top + 0.05)], "back": r3(z), "color": [150, 148, 150], "amount": 0.55, "rise": 4.0, "width": 0.9}
    faces += sp.get("extra", lambda: [])()
    d = {"description": sp["description"], "shape": s, "facing": "Path", "depth": D,
         "light": {"enabled": True, "color": [255, 186, 110], "intensity": 0.45, "radius": 6.0, "at": 0.3, "flicker": 0.15}, "faces": faces}
    if smoke: d["smoke"] = smoke
    if tubes: d["tubes"] = tubes
    return d

def shift(part, dx):
    """A part moved right by dx (a side wall's picture starts at its log ends)."""
    if dx == 0: return part
    sh = dict(part["shape"])
    (kind, g), = sh.items()
    g = dict(g)
    if kind == "Rect": g["min"], g["max"] = [r3(g["min"][0] + dx), g["min"][1]], [r3(g["max"][0] + dx), g["max"][1]]
    elif kind == "Ellipse": g["center"] = [r3(g["center"][0] + dx), g["center"][1]]
    elif kind == "Poly": g["points"] = [[r3(x + dx), y] for x, y in g["points"]]
    elif kind == "Line": g["from"], g["to"] = [r3(g["from"][0] + dx), g["from"][1]], [r3(g["to"][0] + dx), g["to"][1]]
    return dict(part, shape={kind: g})

# ── The cottages ───────────────────────────────────────────────────────────

STONE, MORTAR = [128, 122, 112], [80, 76, 70]
TIMBER = [74, 52, 36]

def cottages():
    out = {}
    out["cottage_timber"] = building({
        "description": "A timber-framed cottage: plastered panels between oak posts, braced in the bays without windows; a tiled roof, a brick chimney on the ridge, lit windows.",
        "hw": 2.45, "E": 2.85, "R": 5.1, "D": 6.5, "o": 0.55, "t": 0.14,
        "wall": "timber", "fill": [214, 200, 172], "timber": TIMBER, "stone": STONE, "plinth": 0.45, "sill": 1.05, "head": 2.25,
        "front_posts": [-2.37, -1.0, 1.0, 2.37], "side_posts": [0.08, 1.3, 2.6, 3.9, 5.2, 6.42],
        "front_windows": [(-1.68, 1.05, 0.9, 1.2), (1.68, 1.05, 0.9, 1.2)], "door_at": (0.0, 0.85, 1.75, 0.45),
        "side_windows": [(1.95, 1.05, 0.85, 1.2), (4.55, 1.05, 0.85, 1.2, False)],
        "attic": "round", "door": [118, 84, 54], "roof": "tile", "roof_col": [122, 74, 52],
        "chimneys": [{"x": 0.0, "z": 3.6, "w": 0.65, "dz": 0.65, "top": 5.75, "kind": "brick"}],
    }, 11)
    out["cottage_stone"] = building({
        "description": "A stone cottage: rubble walls with dressed corner stones, white-framed windows under stone lintels, a green door and a slate roof with a stone chimney at the back.",
        "hw": 2.7, "E": 2.6, "R": 4.95, "D": 7.0, "o": 0.35, "t": 0.12,
        "wall": "stone", "stone": [150, 142, 128], "mortar": [104, 100, 92], "quoin": [176, 170, 156], "lintel": [168, 160, 146],
        "frame": [232, 230, 222], "sill_col": [168, 160, 146], "panes": (2, 3),
        "front_windows": [(-1.6, 0.95, 0.85, 1.15), (1.6, 0.95, 0.85, 1.15, False)], "door_at": (0.0, 0.9, 1.9, 0.12),
        "side_windows": [(1.6, 0.95, 0.8, 1.1), (3.6, 0.95, 0.8, 1.1, False), (5.5, 0.95, 0.8, 1.1)],
        "attic": "square", "attic_lit": False, "door": [58, 96, 72], "step": [150, 142, 128],
        "roof": "slate", "roof_col": [86, 90, 102], "barge": [62, 64, 72],
        "chimneys": [{"x": 0.0, "z": 6.55, "w": 0.75, "dz": 0.6, "top": 5.65, "kind": "stone"}],
    }, 12)
    out["cottage_thatch"] = building({
        "description": "A whitewashed cottage under a deep thatched roof: tarred base, small deep windows with blue shutters, a blue door and a white chimney in the middle of the ridge.",
        "hw": 2.3, "E": 2.25, "R": 5.0, "D": 6.0, "o": 0.6, "oz": 0.3, "t": 0.42,
        "wall": "white", "fill": [234, 228, 212], "tar": [42, 40, 38], "plinth": 0.3, "frame": [238, 236, 230], "shutters": [62, 104, 152],
        "front_windows": [(-1.45, 0.95, 0.65, 0.85), (1.45, 0.95, 0.65, 0.85)], "door_at": (0.0, 0.85, 1.8, 0.1),
        "side_windows": [(1.7, 0.95, 0.65, 0.85), (4.3, 0.95, 0.65, 0.85, False)],
        "attic": "square", "door": [62, 104, 152], "panes": (2, 2),
        "roof": "thatch", "roof_col": [172, 142, 86], "barge": [150, 122, 72],
        "chimneys": [{"x": 0.0, "z": 3.0, "w": 0.6, "dz": 0.6, "top": 5.6, "kind": "plaster"}],
        "chimney_col": [232, 226, 210],
    }, 13)
    out["cottage_brick"] = building({
        "description": "A tall brick house of two storeys: sash windows under brick lintels, a black door with a lit fanlight, a slate roof with a chimney at each end of the ridge.",
        "hw": 2.25, "E": 4.9, "R": 6.9, "D": 6.0, "o": 0.3, "t": 0.12,
        "wall": "brick", "brick": [150, 72, 54], "mortar": [176, 164, 150], "plinth": 0.4, "lintel": [120, 56, 42],
        "frame": [236, 234, 228], "sill_col": [196, 190, 178], "panes": (2, 3), "fanlight": True,
        "front_windows": [(-1.2, 0.9, 0.85, 1.4), (-1.2, 3.0, 0.85, 1.35), (1.2, 3.0, 0.85, 1.35, False)], "door_at": (1.1, 0.95, 2.2, 0.25),
        "side_windows": [(1.6, 0.9, 0.8, 1.4), (4.4, 0.9, 0.8, 1.4, False), (1.6, 3.0, 0.8, 1.35, False), (4.4, 3.0, 0.8, 1.35)],
        "door": [30, 30, 34], "step": [176, 170, 160],
        "roof": "slate", "roof_col": [80, 84, 96], "barge": [56, 58, 66],
        "chimneys": [{"x": 0.0, "z": 0.5, "w": 0.9, "dz": 0.55, "top": 7.6}, {"x": 0.0, "z": 5.5, "w": 0.9, "dz": 0.55, "top": 7.6, "smoke": False}],
        "chimney_col": [146, 70, 52],
    }, 14)
    out["cottage_board"] = building({
        "description": "A weatherboarded cottage painted sage green: white corner boards and window frames, dark green shutters, a red door and a shingle roof with a brick chimney.",
        "hw": 2.4, "E": 2.7, "R": 4.85, "D": 6.5, "o": 0.45, "t": 0.12,
        "wall": "board", "paint": [124, 146, 116], "trim": [236, 234, 226], "stone": STONE, "plinth": 0.3, "frame": [236, 234, 226], "shutters": [46, 76, 56],
        "front_windows": [(-1.55, 1.0, 0.8, 1.15), (1.55, 1.0, 0.8, 1.15)], "door_at": (0.0, 0.85, 1.85, 0.3),
        "side_windows": [(1.4, 1.0, 0.8, 1.15), (3.25, 1.0, 0.8, 1.15, False), (5.1, 1.0, 0.8, 1.15)],
        "attic": "square", "door": [156, 54, 44], "step": [150, 142, 128], "panes": (2, 2),
        "roof": "shingle", "roof_col": [112, 92, 74], "barge": [236, 234, 226],
        "chimneys": [{"x": 0.0, "z": 2.4, "w": 0.6, "dz": 0.6, "top": 5.5}],
    }, 15)
    return out

# ── Log cabins ─────────────────────────────────────────────────────────────

def porch(sp, hw, P, H, roof_at):
    """A porch along the front: a plank deck, posts, a rail with a gap for the steps, a lean-to roof."""
    w, y = hw + 0.15, 0.35
    wood, dk = sp["log"], dark(sp["log"], 0.35)
    deck = [rect(0, 0, 2 * w, P, light(wood, 0.08), 0.4)] + [line([0, z], [2 * w, z], 0.025, dk) for z in frange(0.15, P, 0.15)]
    fascia = [rect(0, 0, 2 * w, y, dk, 0.4)]
    post = [rect(0, 0, 0.16, H - y, wood, 0.6)]
    rail = [line([0, 0.75], [w - 0.75, 0.75], 0.07, wood), line([0, 0.1], [w - 0.75, 0.1], 0.06, wood)]
    rail += [line([x, 0.1], [x, 0.75], 0.04, wood) for x in frange(0.15, w - 0.8, 0.18)]
    lean = math.hypot(roof_at - H - 0.1, P + 0.3)
    roofp = roof_picture(dict(sp, roof="shingle"), 2 * w + 0.4, lean, random.Random(3))
    zf = -P
    joist = [rect(0, 0, P, y, dk, 0.4), rect(0, y - 0.06, P, y, light(wood, 0.05), 0.4)] + [rect(x, 0, x + 0.12, y - 0.06, dark(wood, 0.5), 0.3) for x in frange(0.1, P - 0.1, 0.6)]
    edge = [rect(0, 0, lean, 0.1, dk, 0.4)]
    step = [rect(0, 0, 1.1, 0.35, light(wood, 0.05), 0.4)] + [line([0, zz], [1.1, zz], 0.02, dk) for zz in (0.12, 0.24)]
    rise = [rect(0, 0, 1.1, 0.18, dk, 0.4)]
    return [face([-w, y, 0], [w, y, 0], [-w, y, zf], deck),
            face([-w, y, zf], [w, y, zf], [-w, 0, zf], fascia),
            # Its thickness at the ends, a step up to it, and the edges of its roof.
            face([w, y, zf], [w, y, 0], [w, 0, zf], joist, mirror=True),
            face([-0.55, 0.18, zf], [0.55, 0.18, zf], [-0.55, 0.18, zf - 0.35], step),
            face([-0.55, 0.18, zf - 0.35], [0.55, 0.18, zf - 0.35], [-0.55, 0.0, zf - 0.35], rise),
            face([0.55, 0.18, zf - 0.35], [0.55, 0.18, zf], [0.55, 0.0, zf - 0.35], [rect(0, 0, 0.35, 0.18, dk, 0.4)], mirror=True),
            face([w + 0.2, H + 0.1, zf - 0.3], [w + 0.2, roof_at, 0], [w + 0.2, H, zf - 0.3], edge, mirror=True),
            face([w - 0.18, H, zf + 0.04], [w - 0.02, H, zf + 0.04], [w - 0.18, y, zf + 0.04], post, mirror=True),
            face([w - 0.02, H, zf + 0.04], [w - 0.02, H, zf + 0.2], [w - 0.02, y, zf + 0.04], post, mirror=True),
            face([0.6, y + 0.85, zf + 0.06], [w - 0.18, y + 0.85, zf + 0.06], [0.6, y, zf + 0.06], rail, mirror=True),
            face([-w - 0.2, roof_at, 0], [w + 0.2, roof_at, 0], [-w - 0.2, H + 0.1, zf - 0.3], roofp),
            # Under it a ceiling of boards, and a board along its front edge.
            face([-w - 0.2, roof_at - 0.1, 0], [w + 0.2, roof_at - 0.1, 0], [-w - 0.2, H, zf - 0.3],
                 [rect(0, 0, 2 * w + 0.4, lean, dark(wood, 0.25), 0.3)] + [line([x, 0], [x, lean], 0.03, dark(wood, 0.5)) for x in frange(0.2, 2 * w + 0.3, 0.2)]),
            face([-w - 0.2, H + 0.1, zf - 0.3], [w + 0.2, H + 0.1, zf - 0.3], [-w - 0.2, H - 0.05, zf - 0.3], [rect(0, 0, 2 * w + 0.4, 0.15, dk, 0.4)])]

def cabins():
    out = {}
    base = {"wall": "log", "log_h": 0.3, "frame": [70, 50, 34], "door": [96, 70, 46], "panes": (2, 2), "chimney_kind": "stone", "chimney_col": [138, 132, 120]}
    out["log_cabin_small"] = building(dict(base, **{
        "description": "A small log cabin: round logs saddle-notched at the corners, their sawn tails stacking out both ways by turns; one lit window and a plank door, a shingle roof and a stone chimney built up outside its side wall.",
        "hw": 2.1, "E": 2.3, "R": 4.2, "D": 5.0, "o": 0.4, "t": 0.12, "ends": 0.24,
        "log": [126, 90, 58], "log_end": [190, 152, 104],
        "front_windows": [(-1.1, 0.9, 0.75, 0.85)], "door_at": (0.75, 0.85, 1.8, 0.0),
        "side_windows": [(1.6, 0.9, 0.7, 0.8), (3.6, 0.9, 0.7, 0.8, False)],
        "gable": "board", "paint": [112, 82, 56], "trim": [90, 64, 44],
        "roof": "shingle", "roof_col": [98, 82, 68],
        "chimneys": [{"x": 0, "z": 2.5, "w": 0.75, "dz": 1.0, "top": 4.6, "outside": True}],
    }), 21)
    pz = 1.8
    sp = dict(base, **{
        "description": "A log cabin with a front porch: a plank deck under a lean-to roof on posts, a railing either side of the steps, two lit windows and a stone chimney on the ridge.",
        "hw": 2.6, "E": 2.5, "R": 4.7, "D": 6.0, "o": 0.45, "t": 0.12, "ends": 0.24,
        "log": [108, 76, 50], "log_end": [180, 142, 96],
        "front_windows": [(-1.55, 0.95, 0.8, 0.9), (1.55, 0.95, 0.8, 0.9)], "door_at": (0.0, 0.9, 1.9, 0.35),
        "side_windows": [(2.0, 0.95, 0.75, 0.85), (4.2, 0.95, 0.75, 0.85, False)],
        "gable": "board", "paint": [104, 76, 52], "trim": [84, 60, 40],
        "roof": "shingle", "roof_col": [88, 84, 78], "step": [104, 76, 52],
        "chimneys": [{"x": 0.0, "z": 3.8, "w": 0.75, "dz": 0.75, "top": 5.3}],
    })
    sp["extra"] = lambda: porch(sp, 2.6, pz, 2.15, 2.75)
    out["log_cabin_porch"] = building(sp, 22)
    out["log_cabin_sod"] = building(dict(base, **{
        "description": "A log cabin with a turf roof in the northern way: dark logs, white window frames, grass and wild flowers growing over a low roof edged with birch bark.",
        "hw": 2.3, "E": 2.2, "R": 3.9, "D": 5.5, "o": 0.35, "t": 0.22, "ends": 0.24,
        "log": [92, 64, 44], "log_end": [170, 132, 92], "frame": [236, 234, 228],
        "front_windows": [(-1.25, 0.9, 0.7, 0.8), (1.25, 0.9, 0.7, 0.8)], "door_at": (0.0, 0.8, 1.75, 0.15),
        "side_windows": [(1.8, 0.9, 0.7, 0.8, False), (3.8, 0.9, 0.7, 0.8)],
        "gable": "log", "door": [70, 50, 34], "step": [120, 114, 104],
        "roof": "sod", "roof_col": [92, 120, 58], "birch": [222, 216, 202], "barge": [70, 50, 34],
        "chimneys": [{"x": 0.0, "z": 2.6, "w": 0.6, "dz": 0.6, "top": 4.5}],
    }), 23)
    return out
