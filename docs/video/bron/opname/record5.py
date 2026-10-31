"""Shots for the 30 s feature video, from the clean showcase drawing."""
import sys, time
import rec
from shots import box, frame_bounds, layers, OUT

CLUTTER = ["GIS-*", "B-XX-OG-TRIANGULATIE-G", "B-XX-OG-GRENS-G", "B-XX-AL-*", "N-XX-AL-*", "N-XX-GW-HOOGTE_AFWERKHOOGTE-G", "N-XX-GW-MAAIVELD-G", "N-XX-AM-AS-G", "B-XX-AM-*"]
BUILD = ["B-XX-OG-BEBOUWING*"]


def seq(name, view, style, a, b, seconds, first=0, last=None):
    d = OUT / name
    d.mkdir(parents=True, exist_ok=True)
    rec.host(f"-VIEW {view}")
    rec.host(f"VSCURRENT {style}")
    time.sleep(0.8)
    n = int(seconds * 30)
    last = n - 1 if last is None else last
    rec.capture(d / "warmup.png", bounds=frame_bounds(a, b, first / (n - 1)))
    for i in range(first, last + 1):
        rec.capture(d / f"{i:04d}.png", bounds=frame_bounds(a, b, i / (n - 1)))
    print(name, last - first + 1, "beelden", flush=True)


def all_shots():
    layers(off=CLUTTER, on=["B-XX-*", "N-XX-OG-TRIANGULATIE-G", "N-XX-VH-*", "N-XX-GR-*", "N-XX-GW-TALUD*"])
    layers(off=CLUTTER)
    seq("f-intro", "SWISO", "GOURAUDSHADED", box(104920, 425300, 1000), box(104910, 425320, 780), 4.5)
    # The city builds up: ground first, then the buildings (same camera path).
    A, B = box(104900, 425330, 820), box(104895, 425330, 600)
    layers(off=BUILD)
    seq("f-opbouw0", "SWISO", "GOURAUDSHADED", A, B, 2.8, 0, 55)
    layers(on=BUILD)
    seq("f-opbouw1", "SWISO", "GOURAUDSHADED", A, B, 2.8, 30, 83)
    seq("f-kerk", "SWISO", "GOURAUDSHADED", box(104885, 425335, 250), box(104882, 425335, 185), 2.6)
    # The new design in a design colour, with its axis and stationing.
    d = rec.OCS.document_id()
    for e in rec.OCS.request("query", {"document_id": d, "layer": "N-XX-OG-TRIANGULATIE-G", "detail": "summary", "limit": 50}).get("entities", []):
        rec.OCS.request("set_properties", {"document_id": d, "collection": "entities", "handle": e["handle"], "updates": [{"path": "/common/color", "value": {"Rgb": {"r": 52, "g": 140, "b": 235}}}]})
    layers(on=["B-XX-AM-AS-G", "B-XX-AM-KILOMETRERING-*", "N-XX-VH-*", "N-XX-GR-*", "N-XX-GW-TALUD*"])
    seq("f-straat", "SWISO", "GOURAUDSHADED", box(105000, 425615, 300), box(105015, 425630, 215), 2.6)
    layers(off=["B-XX-*", "N-XX-*"], on=["B-XX-AL-*", "N-XX-AL-*", "N-XX-GW-*", "N-XX-AM-AS-G", "B-XX-GW-MAAIVELD-G"])
    seq("f-profiel", "TOP", "2DWIREFRAME", box(105507, 425626, 240), box(105495, 425650, 130), 2.6)


def wipe_model():
    from record2 import WA, WB, WN
    layers(off=CLUTTER + ["N-XX-*"], on=["B-XX-*"])
    layers(off=CLUTTER + ["B-XX-AM-*"])
    d = OUT / "f-veeg-model"
    d.mkdir(parents=True, exist_ok=True)
    rec.host("-VIEW TOP")
    rec.host("VSCURRENT GOURAUDSHADED")
    time.sleep(0.8)
    rec.capture(d / "warmup.png", bounds=WA)
    for i in range(WN):
        rec.capture(d / f"{i:04d}.png", bounds=frame_bounds(WA, WB, i / (WN - 1)))
    print("f-veeg-model", WN, flush=True)


if __name__ == "__main__":
    rec.attach()
    for s in sys.argv[1:]:
        {"alles": all_shots, "veeg": wipe_model}[s]()
