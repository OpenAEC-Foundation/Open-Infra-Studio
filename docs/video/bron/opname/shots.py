"""Record the shots of the promo sample frame by frame (30 fps).

Every shot: a view preset, a visual style, layer states and a camera move
(bounds from A to B, centre eased, size geometric). Frames go to
shots/<name>/0000.png at the viewport size; the montage crops and scales.
"""
import math, sys, time
from pathlib import Path
import rec

FPS = 30
OUT = rec.V / "shots"

# Layers that clutter a 3D view: profile frames, section axes, flat hatches.
FLAT = ["B-XX-AL-*", "N-XX-AL-*", "B-XX-GW-*", "N-XX-GW-HOOGTE_AFWERKHOOGTE-G", "N-XX-GW-MAAIVELD-G", "N-XX-AM-AS-G",
        "B-XX-AM-*", "B-XX-OG-HOOGTELIJN-G", "B-XX-OG-TRIANGULATIE-G", "B-XX-OG-GRENS-G"]
SOLID = ["B-XX-OG-DTM-G", "B-XX-OG-BEBOUWING*", "B-XX-VH-*", "B-XX-GR-*", "B-XX-OG-TERREIN*", "B-XX-OG-WATER*", "B-XX-KW-*", "B-XX-IE-*", "B-XX-GK-*", "N-XX-OG-TRIANGULATIE-G"]


def layers(off=(), on=()):
    for p in off:
        rec.host(f"-LAYER OFF {p}")
    for p in on:
        rec.host(f"-LAYER ON {p}")


def ease(t):
    return t * t * (3 - 2 * t)


def frame_bounds(a, b, t):
    """Bounds between a and b at t: centre eased, size geometric."""
    e = ease(t)
    ca = ((a[0] + a[2]) / 2, (a[1] + a[3]) / 2)
    cb = ((b[0] + b[2]) / 2, (b[1] + b[3]) / 2)
    c = (ca[0] + (cb[0] - ca[0]) * e, ca[1] + (cb[1] - ca[1]) * e)
    wa, wb = a[2] - a[0], b[2] - b[0]
    ha, hb = a[3] - a[1], b[3] - b[1]
    w = wa * math.exp(math.log(wb / wa) * e)
    h = ha * math.exp(math.log(hb / ha) * e)
    return (c[0] - w / 2, c[1] - h / 2, c[0] + w / 2, c[1] + h / 2)


def shoot(name, view, style, a, b, seconds, setup=None):
    d = OUT / name
    d.mkdir(parents=True, exist_ok=True)
    if setup:
        setup()
    rec.host(f"-VIEW {view}")
    rec.host(f"VSCURRENT {style}")
    time.sleep(1.0)
    n = int(seconds * FPS)
    t0 = time.time()
    # One warm-up frame: the first after a view change can be stale.
    rec.capture(d / "warmup.png", bounds=a)
    for i in range(n):
        rec.capture(d / f"{i:04d}.png", bounds=frame_bounds(a, b, i / max(n - 1, 1)))
    print(f"{name}: {n} beelden in {time.time() - t0:.0f} s", flush=True)


def box(cx, cy, w, h=None):
    h = h or w * 9 / 16
    return (cx - w / 2, cy - h / 2, cx + w / 2, cy + h / 2)


SHOTS = {
    # The whole model to the Grote Kerk.
    "stad": lambda: shoot("stad", "SWISO", "GOURAUDSHADED", box(104930, 425330, 1500), box(104885, 425335, 190), 11,
                          setup=lambda: layers(off=FLAT, on=SOLID)),
    # Along the ring road, west to east, with the design surface.
    "rondweg": lambda: shoot("rondweg", "SWISO", "GOURAUDSHADED", box(104560, 424930, 430), box(105230, 424945, 380), 6,
                             setup=lambda: layers(off=FLAT + ["B-XX-OG-BEBOUWING*", "B-XX-VH-*", "B-XX-GR-*", "B-XX-OG-TERREIN*", "B-XX-IE-*", "B-XX-GK-*", "B-XX-KW-*", "B-XX-OG-WATER*"],
                                                  on=["B-XX-OG-DTM-G", "N-XX-OG-TRIANGULATIE-G", "N-XX-VH-*", "N-XX-GR-*", "N-XX-GW-TALUD*", "N-XX-GW-DIJK*"])),
    # Length profiles (south of the model), panning east.
    "lengteprofiel": lambda: shoot("lengteprofiel", "TOP", "2DWIREFRAME", box(104520, 424700, 460), box(105300, 424650, 460), 5,
                                   setup=lambda: layers(off=SOLID + ["B-XX-GW-*"], on=["B-XX-AL-*", "B-XX-AM-*", "N-XX-AL-*", "N-XX-AM-AS-G", "N-XX-GW-*", "B-XX-GW-MAAIVELD-G"])),
    # Cross sections (east of the model), zooming in.
    "dwarsprofielen": lambda: shoot("dwarsprofielen", "TOP", "2DWIREFRAME", box(105760, 425350, 560), box(105700, 425200, 170), 5),
    # Earthworks: cut and fill along road and dike, from above.
    "grondverzet": lambda: shoot("grondverzet", "TOP", "GOURAUDSHADED", box(104900, 425350, 1350), box(104700, 425000, 520), 5,
                                 setup=lambda: layers(off=SOLID + FLAT, on=["B-XX-GW-ONTGRAVINGSVAK-WERK-A", "B-XX-GW-OPHOGING-WERK-A", "B-XX-AM-AS-G", "B-XX-OG-BEBOUWING*", "B-XX-OG-WATER*"])),
    # The NLCS example drawing (open as the active document): the whole sheet
    # to the cables and pipes along the Lange Geldersekade.
    "nlcs": lambda: shoot("nlcs", "TOP", "2DWIREFRAME", box(104928, 425320, 600, 420), box(104864, 425325, 110), 6),
}

if __name__ == "__main__":
    rec.attach()
    for name in (sys.argv[1:] or list(SHOTS)):
        SHOTS[name]()
