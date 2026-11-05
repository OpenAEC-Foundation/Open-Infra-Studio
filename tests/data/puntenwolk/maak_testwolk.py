#!/usr/bin/env python3
"""Make the synthetic point clouds of the point cloud tests (standard
library only, deterministic).

  onbewerkt.las   an unclassified airborne-like scan of 80 x 60 m in RD
                  (every point class 1): rolling ground, a building of
                  20 x 15 m with a flat roof 7 m up, and three trees. The
                  ground filter must find the ground under them.
  brug.ply        a footbridge of 20 x 4 m sampled on every face (binary
                  PLY, doubles): a deck 4.0-4.6 m up on two piers.

The ground height is z = 0.8 + 0.01 (x - X0) + 0.3 sin((x - X0) / 12)
cos((y - Y0) / 9), so a test can check a TIN against it.

    python3 tests/data/puntenwolk/maak_testwolk.py
"""

import math
import random
import struct
from pathlib import Path

HERE = Path(__file__).resolve().parent
X0, Y0 = 105400.0, 425300.0
W, H = 80.0, 60.0
BUILDING = (X0 + 30.0, Y0 + 20.0, X0 + 50.0, Y0 + 35.0, 7.0)
TREES = [(X0 + 12.0, Y0 + 45.0, 3.0), (X0 + 65.0, Y0 + 12.0, 3.5), (X0 + 68.0, Y0 + 48.0, 2.5)]


def ground(x, y):
    return 0.8 + 0.01 * (x - X0) + 0.3 * math.sin((x - X0) / 12.0) * math.cos((y - Y0) / 9.0)


def scan():
    rnd = random.Random(7)
    pts = []
    step = 0.4
    y = Y0
    while y < Y0 + H:
        x = X0
        while x < X0 + W:
            px, py = x + rnd.random() * 0.25, y + rnd.random() * 0.25
            g = ground(px, py) + rnd.uniform(-0.03, 0.03)
            b = BUILDING
            if b[0] < px < b[2] and b[1] < py < b[3]:
                pts.append((px, py, ground(px, py) + b[4] + rnd.uniform(-0.02, 0.02)))
            elif any(math.hypot(px - t[0], py - t[1]) < t[2] for t in TREES):
                pts.append((px, py, g if rnd.random() < 0.3 else ground(px, py) + 3.0 + rnd.random() * 5.0))
            else:
                pts.append((px, py, g))
            x += step
        y += step
    # Some points on the walls.
    b = BUILDING
    for k in range(200):
        t = k / 200.0
        for (px, py) in ((b[0] + (b[2] - b[0]) * t, b[1]), (b[0] + (b[2] - b[0]) * t, b[3]), (b[0], b[1] + (b[3] - b[1]) * t), (b[2], b[1] + (b[3] - b[1]) * t)):
            pts.append((px, py, ground(px, py) + rnd.random() * b[4]))
    return pts


def write_las(path, pts):
    """LAS 1.2, point format 0, every point class 1 (unclassified)."""
    scale = 0.001
    off = (X0, Y0, 0.0)
    xs, ys, zs = [p[0] for p in pts], [p[1] for p in pts], [p[2] for p in pts]
    header = bytearray(227)
    header[0:4] = b"LASF"
    header[24:26] = bytes([1, 2])
    header[26:58] = b"Open Infra".ljust(32, b"\0")
    header[58:90] = b"maak_testwolk.py".ljust(32, b"\0")
    struct.pack_into("<HH", header, 90, 1, 2026)
    struct.pack_into("<H", header, 94, 227)
    struct.pack_into("<I", header, 96, 227)
    struct.pack_into("<I", header, 100, 0)
    header[104] = 0
    struct.pack_into("<H", header, 105, 20)
    struct.pack_into("<I", header, 107, len(pts))
    struct.pack_into("<5I", header, 111, len(pts), 0, 0, 0, 0)
    struct.pack_into("<3d", header, 131, scale, scale, scale)
    struct.pack_into("<3d", header, 155, *off)
    struct.pack_into("<6d", header, 179, max(xs), min(xs), max(ys), min(ys), max(zs), min(zs))
    body = bytearray()
    for (x, y, z) in pts:
        body += struct.pack(
            "<iiiHBBbBH",
            round((x - off[0]) / scale), round((y - off[1]) / scale), round((z - off[2]) / scale),
            1000, 0b00001001, 1, 0, 0, 0,
        )
    path.write_bytes(bytes(header) + bytes(body))


def bridge():
    """A footbridge sampled every 0.12 m: deck 20 x 4 m, 0.6 m thick, its
    underside 4.0 m up, on two piers of 0.8 x 4 m."""
    bx, by = X0 + 10.0, Y0 - 30.0
    s = 0.12
    pts = []

    def face(o, u, v, nu, nv):
        for i in range(nu + 1):
            for j in range(nv + 1):
                pts.append(tuple(o[k] + u[k] * i / nu + v[k] * j / nv for k in range(3)))

    def box(x0, y0, z0, dx, dy, dz):
        n = lambda d: max(1, round(d / s))
        face((x0, y0, z0), (dx, 0, 0), (0, dy, 0), n(dx), n(dy))
        face((x0, y0, z0 + dz), (dx, 0, 0), (0, dy, 0), n(dx), n(dy))
        face((x0, y0, z0), (dx, 0, 0), (0, 0, dz), n(dx), n(dz))
        face((x0, y0 + dy, z0), (dx, 0, 0), (0, 0, dz), n(dx), n(dz))
        face((x0, y0, z0), (0, dy, 0), (0, 0, dz), n(dy), n(dz))
        face((x0 + dx, y0, z0), (0, dy, 0), (0, 0, dz), n(dy), n(dz))

    box(bx, by, 4.0, 20.0, 4.0, 0.6)
    box(bx + 4.0, by, 0.0, 0.8, 4.0, 4.0)
    box(bx + 15.2, by, 0.0, 0.8, 4.0, 4.0)
    return pts


def write_ply(path, pts):
    head = "ply\nformat binary_little_endian 1.0\ncomment Open Infra test bridge, RD New (EPSG:28992)\n"
    head += f"element vertex {len(pts)}\nproperty double x\nproperty double y\nproperty double z\nend_header\n"
    body = b"".join(struct.pack("<3d", *p) for p in pts)
    path.write_bytes(head.encode() + body)


if __name__ == "__main__":
    pts = scan()
    write_las(HERE / "onbewerkt.las", pts)
    b = bridge()
    write_ply(HERE / "brug.ply", b)
    print(f"onbewerkt.las: {len(pts)} points; brug.ply: {len(b)} points")
