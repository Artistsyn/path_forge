#!/usr/bin/env python3
"""Writes the built-in shape kits kits/settlement, kits/waterside and kits/wildlife (kit.json each).
Shapes are in metres: x sideways from the centre, y up from the ground. Run from the repo root:
    python3 tools/make_kits.py
"""
import json, math, os
import kit_buildings

def rect(x0, y0, x1, y1, col, shade=0.5, glow=0.0):
    return {"shape": {"Rect": {"min": [x0, y0], "max": [x1, y1]}}, "color": col, "shade": shade, **({"glow": glow} if glow else {})}
def ell(cx, cy, rx, ry, col, shade=0.5, glow=0.0, cut=False):
    p = {"shape": {"Ellipse": {"center": [cx, cy], "radius": [rx, ry]}}, "color": col, "shade": shade}
    if glow: p["glow"] = glow
    if cut: p["cut"] = True
    return p
def poly(pts, col, shade=0.5, glow=0.0):
    return {"shape": {"Poly": {"points": [[round(x, 4), round(y, 4)] for x, y in pts]}}, "color": col, "shade": shade, **({"glow": glow} if glow else {})}
def line(a, b, w, col, shade=0.3):
    return {"shape": {"Line": {"from": a, "to": b, "width": w}}, "color": col, "shade": shade}
def mix(a, b, t): return [round(a[i] + (b[i] - a[i]) * t) for i in range(3)]

# ── Settlement ────────────────────────────────────────────────────────────
STONE, MORTAR = [128, 122, 112], [80, 76, 70]
PLASTER, TIMBER = [214, 200, 172], [74, 52, 36]
ROOF, ROOF_DARK = [122, 74, 52], [80, 46, 32]
WOOD, WOOD_DARK = [118, 84, 54], [72, 50, 32]
WARM = [255, 196, 112]

def well():
    s = [rect(-0.85, 0, 0.85, 0.95, STONE, 0.6)]
    for y in [0.3, 0.62]: s.append(line([-0.85, y], [0.85, y], 0.04, MORTAR))
    for i, x in enumerate([-0.55, -0.1, 0.35, 0.7]): s.append(line([x + (0.2 if i % 2 else 0), 0.0], [x + (0.2 if i % 2 else 0), 0.3], 0.04, MORTAR))
    for i, x in enumerate([-0.6, -0.15, 0.3]): s.append(line([x, 0.3], [x, 0.62], 0.04, MORTAR))
    s += [ell(0, 0.95, 0.88, 0.14, mix(STONE, [255, 255, 255], 0.15), 0.5), ell(0, 0.95, 0.68, 0.08, [24, 22, 22], 0.0)]
    s += [rect(-0.82, 0.9, -0.66, 2.3, WOOD_DARK, 0.6), rect(0.66, 0.9, 0.82, 2.3, WOOD_DARK, 0.6)]
    s += [line([-0.74, 1.75], [0.74, 1.75], 0.09, WOOD), line([0, 1.75], [0, 1.35], 0.02, [150, 130, 90]), rect(-0.16, 1.1, 0.16, 1.37, WOOD, 0.6), line([-0.16, 1.37], [0.16, 1.37], 0.04, [60, 60, 64])]
    s.append(poly([(-1.15, 2.2), (1.15, 2.2), (0, 2.95)], ROOF, 0.6))
    s.append(line([-1.18, 2.18], [1.18, 2.18], 0.07, ROOF_DARK))
    return {"description": "A stone well with a little roof, a windlass and a bucket.", "shape": s}

def market_stall():
    s = []
    for x in [-1.45, 1.45]: s.append(rect(x - 0.06, 0, x + 0.06, 2.35, WOOD_DARK, 0.6))
    s += [rect(-1.5, 0.82, 1.5, 1.0, WOOD, 0.5), rect(-1.4, 0.0, -1.3, 0.82, WOOD_DARK, 0.5), rect(1.3, 0.0, 1.4, 0.82, WOOD_DARK, 0.5), rect(-1.45, 0.1, 1.45, 0.82, mix(WOOD, [0, 0, 0], 0.35), 0.3)]
    # Goods heaped on the counter: apples, oranges, loaves, a crate of greens.
    for i in range(7): s.append(ell(-1.2 + i * 0.13, 1.06 + (0.06 if i % 2 else 0), 0.075, 0.075, [190, 40, 34], 0.8))
    for i in range(5): s.append(ell(-0.2 + i * 0.14, 1.06 + (0.06 if i % 2 else 0), 0.075, 0.075, [236, 140, 34], 0.8))
    s += [ell(0.75, 1.08, 0.16, 0.09, [200, 150, 90], 0.8), ell(1.05, 1.08, 0.16, 0.09, [190, 140, 84], 0.8), rect(0.6, 1.0, 1.3, 1.02, WOOD_DARK)]
    # The striped awning, sloping down towards you, its scalloped edge.
    cols = [[196, 44, 44], [236, 228, 210]]
    n = 8
    for i in range(n):
        x0, x1 = -1.65 + 3.3 * i / n, -1.65 + 3.3 * (i + 1) / n
        s.append(poly([(x0 * 0.92, 2.65), (x1 * 0.92, 2.65), (x1, 2.15), (x0, 2.15)], cols[i % 2], 0.5))
        s.append(ell((x0 + x1) / 2, 2.15, (x1 - x0) / 2, 0.11, cols[i % 2], 0.5))
    return {"description": "A market stall: a counter heaped with fruit and bread under a striped awning.", "shape": s}

def picket_fence():
    white = [228, 224, 212]
    s = [rect(-0.95, 0.32, 0.95, 0.4, white, 0.4), rect(-0.95, 0.74, 0.95, 0.82, white, 0.4)]
    for i in range(10):
        x = -0.85 + i * 0.19
        s.append(poly([(x - 0.06, 0), (x + 0.06, 0), (x + 0.06, 0.98), (x, 1.06), (x - 0.06, 0.98)], white, 0.5))
    return {"description": "A white picket fence, two metres to a section: space its layer every 1.9 m for an unbroken fence.", "shape": s}

def haybale():
    straw = [214, 176, 92]
    s = [rect(-0.55, 0, 0.55, 0.62, straw, 0.6)]
    for y in [0.12, 0.25, 0.38, 0.5]: s.append(line([-0.55, y], [0.55, y + 0.02], 0.02, mix(straw, [0, 0, 0], 0.25)))
    s += [line([-0.25, 0], [-0.25, 0.62], 0.03, [120, 90, 50]), line([0.25, 0], [0.25, 0.62], 0.03, [120, 90, 50])]
    return {"description": "A bale of hay, tied with twine.", "shape": s}

def cart():
    s = [rect(-1.1, 0.55, 0.9, 1.15, WOOD, 0.5)]
    for x in [-0.6, -0.1, 0.4]: s.append(line([x, 0.6], [x, 1.1], 0.04, WOOD_DARK))
    s += [line([0.9, 0.7], [1.9, 0.55], 0.08, WOOD_DARK), line([0.9, 0.95], [1.9, 0.62], 0.06, WOOD_DARK)]
    s += [ell(-0.25, 0.5, 0.5, 0.5, [60, 44, 30], 0.4), ell(-0.25, 0.5, 0.42, 0.42, [0, 0, 0], 0.0, cut=True)]
    for k in range(8):
        a = math.pi * k / 4
        s.append(line([-0.25, 0.5], [-0.25 + 0.44 * math.cos(a), 0.5 + 0.44 * math.sin(a)], 0.04, [70, 52, 36]))
    s.append(ell(-0.25, 0.5, 0.07, 0.07, [50, 46, 42], 0.6))
    for i in range(4): s.append(ell(-0.8 + i * 0.4, 1.22, 0.18, 0.12, [214, 176, 92], 0.6))
    return {"description": "A wooden hand cart loaded with hay.", "shape": s}

def woodpile():
    end, bark = [196, 160, 112], [86, 62, 44]
    s = [rect(-1.0, 0, 1.0, 1.0, bark, 0.4)]
    for row in range(4):
        for i in range(6 - (row % 2)):
            x = -0.85 + i * 0.34 + (0.17 if row % 2 else 0)
            y = 0.13 + row * 0.24
            s += [ell(x, y, 0.15, 0.12, end, 0.5), ell(x, y, 0.05, 0.04, mix(end, [0, 0, 0], 0.25), 0.2)]
    return {"description": "Split logs stacked for the winter, their cut ends showing.", "shape": s}

settlement = {"name": "Settlement", "description": "A village by the road, drawn from shapes: cottages of several builds and log cabins with lit windows and smoking chimneys, a well, a market stall, picket fences, hay bales, a cart and a woodpile. Use as \"@settlement#<name>\".",
              "props": {"well": well(), "market_stall": market_stall(), "picket_fence": picket_fence(), "haybale": haybale(), "cart": cart(), "woodpile": woodpile()}}

# ── Faces: what a prop looks like from the side and above, once it stands along the street ──
# Corners are in the prop's space (x across its front, y up, z back from the path): the
# picture's top left, top right and bottom left as seen from the side it faces. Pictures are
# painted in metres from their bottom left corner.
def face(tl, tr, bl, shape, mirror=False):
    f = {"corners": [tl, tr, bl], "shape": shape}
    if mirror: f["mirror"] = True
    return f
def size(tl, tr, bl): return math.dist(tl, tr), math.dist(tl, bl)

def stall_faces():
    D = 1.6
    cols = [[196, 44, 44], [236, 228, 210]]
    side = [rect(D - 0.14, 0, D - 0.02, 2.95, WOOD_DARK, 0.6), rect(0, 0.1, 0.6, 1.0, mix(WOOD, [0, 0, 0], 0.35), 0.3), rect(0, 0.82, 0.62, 1.0, WOOD, 0.5)]
    for i in range(4):
        a, b = D * i / 4, D * (i + 1) / 4
        ya, yb = 2.65 + 0.3 * a / D, 2.65 + 0.3 * b / D
        side.append(poly([(a, ya), (b, yb), (b, yb - 0.12), (a, ya - (0.5 if i == 0 else 0.12))], cols[i % 2], 0.5))
    W, H = 3.3, math.hypot(0.3, D)
    awning = []
    for i in range(8): awning.append(rect(W * i / 8, 0, W * (i + 1) / 8, H, cols[i % 2], 0.45))
    awning.append(rect(0, 0, W, 0.05, mix(cols[0], [0, 0, 0], 0.4), 0.2))
    goods = [rect(0, 0, 3.0, 0.6, WOOD, 0.5)]
    for i in range(9): goods.append(ell(0.25 + i * 0.13, 0.15 + (i % 3) * 0.13, 0.07, 0.07, [190, 40, 34], 0.8))
    for i in range(7): goods.append(ell(1.35 + i * 0.13, 0.15 + (i % 3) * 0.13, 0.07, 0.07, [236, 140, 34], 0.8))
    goods += [ell(2.3, 0.3, 0.16, 0.1, [200, 150, 90], 0.8), ell(2.65, 0.25, 0.16, 0.1, [190, 140, 84], 0.8)]
    return [face([1.45, 2.95, 0], [1.45, 2.95, D], [1.45, 0, 0], side, mirror=True),
            face([-1.65, 2.95, D], [1.65, 2.95, D], [-1.65, 2.65, 0], awning),
            face([-1.5, 1.0, 0.6], [1.5, 1.0, 0.6], [-1.5, 1.0, 0], goods)]

def cart_faces():
    D = 1.2
    end = [rect(0, 0, D, 0.6, WOOD, 0.5)] + [line([0, y], [D, y], 0.04, WOOD_DARK) for y in [0.15, 0.3, 0.45]]
    hay = [rect(0, 0, 2.0, D, mix(WOOD_DARK, [0, 0, 0], 0.2), 0.2)]
    for i in range(5):
        for j in range(3): hay.append(ell(0.2 + i * 0.4, 0.2 + j * 0.4, 0.26, 0.22, [214, 176, 92], 0.6))
    wheel = [ell(0.5, 0.5, 0.5, 0.5, [60, 44, 30], 0.4), ell(0.5, 0.5, 0.42, 0.42, [0, 0, 0], 0.0, cut=True)]
    for k in range(8):
        a = math.pi * k / 4
        wheel.append(line([0.5, 0.5], [0.5 + 0.44 * math.cos(a), 0.5 + 0.44 * math.sin(a)], 0.04, [70, 52, 36]))
    wheel.append(ell(0.5, 0.5, 0.07, 0.07, [50, 46, 42], 0.6))
    return [face([0.9, 1.15, 0], [0.9, 1.15, D], [0.9, 0.55, 0], end),
            face([-1.1, 1.15, D], [-1.1, 1.15, 0], [-1.1, 0.55, D], end),
            face([-1.1, 1.15, D], [0.9, 1.15, D], [-1.1, 1.15, 0], hay),
            face([0.25, 1.0, D], [-0.75, 1.0, D], [0.25, 0.0, D], wheel)]

def woodpile_faces():
    D = 0.9
    end, bark = [196, 160, 112], [86, 62, 44]
    side = [rect(0, 0, D, 1.0, mix(bark, [0, 0, 0], 0.4), 0.2)]
    for row in range(4):
        y = 0.13 + row * 0.24
        side.append(rect(0, y - 0.11, D, y + 0.11, bark, 0.7))
        side.append(line([0, y + 0.02], [D, y + 0.04], 0.015, mix(bark, [0, 0, 0], 0.3)))
        side.append(ell(0.02, y, 0.04, 0.11, end, 0.5))
    top = [rect(0, 0, 2.0, D, mix(bark, [0, 0, 0], 0.4), 0.2)]
    for i in range(6):
        x = 0.15 + i * 0.34
        top.append(rect(x - 0.14, 0, x + 0.14, D, bark, 0.7))
        top.append(line([x - 0.03, 0.05], [x + 0.02, D - 0.05], 0.015, mix(bark, [0, 0, 0], 0.3)))
    return [face([1.0, 1.0, 0], [1.0, 1.0, D], [1.0, 0, 0], side, mirror=True),
            face([-1.0, 1.0, D], [1.0, 1.0, D], [-1.0, 1.0, 0], top)]

# Buildings and long things stand along the street, their fronts to it, as in a real street:
# seen down the path, their side walls and roofs show; round things face the camera from any
# side. Their faces are the rest of them, seen from the side and above. (The fence keeps facing
# the camera until it turns: fences[] draws joined fences along the path.)
for name, depth, faces in [("market_stall", 1.6, stall_faces()), ("picket_fence", 0.0, []),
                           ("cart", 1.2, cart_faces()), ("woodpile", 0.9, woodpile_faces())]:
    settlement["props"][name]["facing"] = "Path" if faces else "PathWhenTurned"
    if depth > 0: settlement["props"][name]["depth"] = depth
    if faces: settlement["props"][name]["faces"] = faces
# Buildings (kit_buildings.py): cottages of five builds and three log cabins, each a group that
# picks one per house so no two neighbours need match.
houses, huts = kit_buildings.cottages(), kit_buildings.cabins()
settlement["props"].update(houses)
settlement["props"].update(huts)
settlement["props"]["cottage"] = {"description": "A cottage of the village, each one built differently: timber-framed, stone, whitewashed under thatch, two storeys of brick or weatherboarded; lit windows and a smoking chimney, lighting the ground before it.",
                                  "pick": sorted(houses), "facing": "Path", "depth": 6.5}
settlement["props"]["log_cabin"] = {"description": "A log cabin, each one different: a small one with a stone chimney up its side, one with a front porch, one with a turf roof.",
                                    "pick": sorted(huts), "facing": "Path", "depth": 5.5}

# ── Waterside ─────────────────────────────────────────────────────────────
def rowboat():
    paint, inner = [70, 110, 140], [120, 86, 58]
    s = [poly([(-1.75, 0.55), (1.95, 0.62), (1.55, 0.12), (-1.45, 0.08)], paint, 0.6), poly([(-1.75, 0.55), (1.95, 0.62), (1.9, 0.5), (-1.7, 0.45)], inner, 0.4)]
    s += [line([-1.75, 0.56], [1.95, 0.63], 0.06, [230, 224, 210]), line([-0.2, 0.57], [-0.2, 0.18], 0.04, mix(paint, [0, 0, 0], 0.4)), line([0.7, 0.6], [0.7, 0.2], 0.04, mix(paint, [0, 0, 0], 0.4))]
    s += [line([-1.2, 0.85], [0.9, 0.35], 0.05, WOOD), poly([(0.8, 0.4), (1.25, 0.15), (1.3, 0.25), (0.9, 0.46)], WOOD, 0.4), line([1.95, 0.6], [2.5, 0.0], 0.02, [170, 150, 110])]
    return {"description": "A rowing boat tied up, an oar across it: set it on the water beside a pier or bank.", "shape": s}

def lily_pads():
    g, g2 = [60, 116, 52], [86, 140, 64]
    s = []
    for (x, rx, notch) in [(-0.6, 0.32, 1), (0.05, 0.4, -1), (0.62, 0.26, 1), (-0.15, 0.2, 1)]:
        s.append(ell(x, 0.04, rx, rx * 0.22, g, 0.4))
        s.append(ell(x - 0.15 * rx, 0.05, rx * 0.6, rx * 0.12, g2, 0.2))
        s.append(poly([(x, 0.04), (x + notch * rx, 0.04 + rx * 0.2), (x + notch * rx, 0.04 - rx * 0.05)], [0, 0, 0]) | {"cut": True})
    # A water lily open on one.
    for k in range(7):
        a = math.pi * (k + 0.5) / 7
        s.append(poly([(0.05, 0.08), (0.05 + 0.13 * math.cos(a) - 0.03, 0.08 + 0.12 * math.sin(a)), (0.05 + 0.15 * math.cos(a), 0.08 + 0.16 * math.sin(a)), (0.05 + 0.13 * math.cos(a) + 0.03, 0.08 + 0.12 * math.sin(a))], [246, 214, 226], 0.4))
    s.append(ell(0.05, 0.1, 0.035, 0.03, [250, 210, 80], 0.5))
    return {"description": "Lily pads floating on still water, a lily open on one.", "shape": s}

def mooring_post():
    s = [rect(-0.12, 0, 0.12, 1.3, WOOD_DARK, 0.6), ell(0, 1.3, 0.12, 0.05, mix(WOOD_DARK, [255, 255, 255], 0.2), 0.4)]
    for y in [0.7, 0.85]: s.append(rect(-0.14, y, 0.14, y + 0.07, [170, 150, 110], 0.5))
    s.append(rect(-0.13, 0, 0.13, 0.25, [60, 80, 60], 0.3))
    return {"description": "A wooden mooring post with rope wound on it, green where the water laps.", "shape": s}

def buoy():
    s = [poly([(-0.3, 0.05), (0.3, 0.05), (0.16, 0.8), (-0.16, 0.8)], [200, 40, 34], 0.6), rect(-0.24, 0.3, 0.24, 0.48, [236, 232, 220], 0.6), rect(-0.03, 0.8, 0.03, 1.15, [60, 60, 64], 0.4), ell(0, 1.17, 0.07, 0.07, [255, 230, 140], 0.0, glow=2.0)]
    return {"description": "A red and white buoy with a light on top.", "shape": s, "light": {"enabled": True, "color": [255, 220, 140], "intensity": 0.25, "radius": 3.0, "at": 1.0, "flicker": 0.6}}

waterside = {"name": "Waterside", "description": "Things on and by the water, drawn from shapes: a rowing boat, lily pads, mooring posts and a buoy. Use as \"@waterside#<name>\".",
             "props": {"rowboat": rowboat(), "lily_pads": lily_pads(), "mooring_post": mooring_post(), "buoy": buoy()}}

# ── Wildlife ──────────────────────────────────────────────────────────────
FUR, BELLY = [138, 94, 58], [196, 166, 128]
def deer(grazing=False, stag=False):
    s = []
    legs = [(-0.42, -0.5), (-0.3, -0.36), (0.38, 0.46), (0.5, 0.58)]
    for i, (x0, x1) in enumerate(legs):
        c = mix(FUR, [0, 0, 0], 0.25 if i % 2 == 0 else 0.0)
        s += [line([x0, 0.95], [x0 + 0.02, 0.5], 0.08, c), line([x0 + 0.02, 0.5], [x1 * 0.85, 0.02], 0.05, c)]
    s += [ell(0, 1.05, 0.62, 0.28, FUR, 0.6), ell(0.0, 0.95, 0.45, 0.12, BELLY, 0.3), ell(-0.6, 1.15, 0.08, 0.12, [236, 226, 210], 0.4)]
    if grazing:
        s += [poly([(0.45, 1.2), (0.62, 1.05), (0.85, 0.35), (0.72, 0.3)], FUR, 0.5), ell(0.86, 0.26, 0.11, 0.17, FUR, 0.6), ell(0.8, 0.45, 0.04, 0.08, FUR, 0.5)]
    else:
        s += [poly([(0.4, 1.15), (0.62, 1.1), (0.78, 1.7), (0.62, 1.75)], FUR, 0.5), ell(0.82, 1.78, 0.2, 0.11, FUR, 0.6), ell(0.98, 1.74, 0.04, 0.035, [30, 24, 20], 0.3)]
        s += [ell(0.66, 1.92, 0.04, 0.1, FUR, 0.5), ell(0.74, 1.93, 0.04, 0.1, mix(FUR, [0, 0, 0], 0.2), 0.5), ell(0.86, 1.8, 0.025, 0.025, [20, 16, 14], 0.0)]
        if stag:
            antler = [90, 74, 56]
            for side, dx in [(1, 0.0), (-1, -0.08)]:
                b = [0.72 + dx, 1.86]
                t = [0.6 + dx, 2.45]
                s.append(line(b, t, 0.035, antler))
                for f in [0.35, 0.6, 0.85]:
                    p = [b[0] + (t[0] - b[0]) * f, b[1] + (t[1] - b[1]) * f]
                    s.append(line(p, [p[0] + 0.18, p[1] + 0.12], 0.025, antler))
    return s

wildlife = {"name": "Wildlife", "description": "Animals by the way, drawn from shapes: deer standing and grazing, a stag, a rabbit and a heron. Use as \"@wildlife#<name>\". (Birds, bats and leaping fish move: see `wildlife` in the scene.)",
            "props": {
                "deer": {"description": "A deer standing, head up, watching.", "shape": deer()},
                "deer_grazing": {"description": "A deer grazing, head down.", "shape": deer(grazing=True)},
                "stag": {"description": "A stag with branching antlers.", "shape": deer(stag=True)},
                "rabbit": {"description": "A rabbit sitting up.", "shape": [ell(0, 0.13, 0.15, 0.12, [150, 120, 90], 0.6), ell(0.12, 0.25, 0.07, 0.06, [150, 120, 90], 0.6), ell(0.09, 0.36, 0.022, 0.08, [140, 110, 84], 0.5), ell(0.13, 0.36, 0.022, 0.08, [150, 120, 90], 0.5), ell(-0.15, 0.12, 0.04, 0.04, [236, 230, 220], 0.4), ell(0.16, 0.26, 0.012, 0.012, [20, 16, 14], 0.0)]},
                "heron": {"description": "A grey heron standing in the shallows.", "shape": [line([0.0, 0.0], [0.02, 0.55], 0.025, [200, 170, 90]), line([0.1, 0.0], [0.08, 0.55], 0.025, [190, 160, 80]), ell(0.05, 0.75, 0.22, 0.14, [150, 156, 166], 0.6), poly([(-0.15, 0.78), (-0.4, 0.6), (-0.1, 0.68)], [110, 116, 126], 0.4), poly([(0.15, 0.8), (0.22, 1.2), (0.28, 1.2), (0.24, 0.8)], [200, 204, 210], 0.4), ell(0.28, 1.24, 0.07, 0.05, [220, 224, 228], 0.5), poly([(0.33, 1.25), (0.55, 1.22), (0.34, 1.21)], [220, 180, 60], 0.3), line([0.24, 1.27], [0.1, 1.3], 0.015, [30, 30, 34])]},
            }}

for name, kit in [("settlement", settlement), ("waterside", waterside), ("wildlife", wildlife)]:
    os.makedirs(f"kits/{name}", exist_ok=True)
    # (Written compactly: the kits are compiled into the program; edit this script, not them.)
    with open(f"kits/{name}/kit.json", "w") as f: json.dump(kit, f, separators=(",", ":"))
    print(name, len(kit["props"]), "props")
