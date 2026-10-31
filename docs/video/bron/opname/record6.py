"""Version 6: the program itself (full window, ribbon), and the 2D GIS
sources stacked on one camera path."""
import subprocess, sys, time
import rec
from shots import box, frame_bounds, layers, OUT
from record5 import CLUTTER


def window_seq(name, view, style, a, b, seconds):
    """Full-window frames (ribbon and panels) while the camera moves."""
    d = OUT / name
    d.mkdir(parents=True, exist_ok=True)
    rec.host(f"-VIEW {view}")
    rec.host(f"VSCURRENT {style}")
    time.sleep(0.8)
    n = int(seconds * 30)
    for i in range(n):
        rec.capture(rec.V / "warm.png", bounds=frame_bounds(a, b, i / (n - 1)))
        rec.grab_window("Open CAD Studio", d / f"{i:04d}.png")
    print(name, n, flush=True)


def ui():
    rec.start(rec.V / "showcase.dwg")
    time.sleep(8)
    rec.cancel()
    subprocess.run(["wmctrl", "-i", "-r", rec.window(), "-b", "add,fullscreen"])
    time.sleep(2)
    # The real program: ribbon and panels on, view cube on.
    if rec.OCS.request("state").get("viewport_size") == [1920.0, 1018.0]:
        rec.host("CLEANSCREEN")
        time.sleep(1.5)
    rec.host("NAVVCUBE")
    rec.host("UCSICON OFF")
    rec.host("CURSORSIZE 1")
    rec.OCS.request("pointer", {"document_id": rec.OCS.document_id(), "events": [{"move": [-50, -50]}]})
    layers(off=CLUTTER)
    rec.action("ribbon_tab", "openaec.open-infra-civiel")
    window_seq("u-civiel", "SWISO", "GOURAUDSHADED", box(104930, 425330, 700), box(104900, 425330, 430), 4.0)
    r = rec.OCS.request("open", {"path": "/home/aia10/Documenten/Open-Infra/examples/dordrecht-nlcs/dordrecht-grote-kerk-nlcs.dwg"})
    time.sleep(6)
    rec.OCS.request("pointer", {"document_id": rec.OCS.document_id(), "events": [{"move": [-50, -50]}]})
    rec.action("ribbon_tab", "openaec.open-infra")
    window_seq("u-gis", "TOP", "2DWIREFRAME", box(104928, 425320, 640, 420), box(104880, 425330, 330), 4.0)




GA, GB = box(104885, 425330, 300), box(104880, 425332, 205)


def stage(name):
    d = OUT / name
    d.mkdir(parents=True, exist_ok=True)
    rec.capture(d / "warmup.png", bounds=GA)
    for i in range(66):
        rec.capture(d / f"{i:04d}.png", bounds=frame_bounds(GA, GB, i / 65))
    print(name, flush=True)


def gis():
    rec.attach()
    r = rec.OCS.request("open", {"path": str(rec.V / "gisdemo.dwg")})
    time.sleep(6)
    rec.host("CLEANSCREEN")
    time.sleep(1.5)
    rec.host("NAVVCUBE")
    rec.host("UCSICON OFF")
    rec.OCS.request("pointer", {"document_id": rec.OCS.document_id(), "events": [{"move": [-50, -50]}]})
    rec.host("-VIEW TOP")
    rec.host("VSCURRENT 2DWIREFRAME")
    run = lambda c: print(c, rec.OCS.run(c, timeout=600)["message"][:120], flush=True)
    run("GIS_SITE RD 104885 425330 320 260 Grote Kerk")
    run("GIS_Q_LUCHTFOTO")
    stage("g-luchtfoto")
    run("GIS_Q_BGT")
    stage("g-bgt")
    run("GIS_Q_BRK")
    stage("g-brk")
    layers(off=["B-*"])
    run("GIS_Q_OSM")
    stage("g-osm")
    d = rec.OCS.document_id()
    rasters = [e["handle"] for e in rec.OCS.request("query", {"document_id": d, "type": "RasterImage", "detail": "summary", "limit": 20}).get("entities", [])]
    rec.OCS.request("entities_delete", {"document_id": d, "handles": rasters})
    layers(off=["OSM-*", "GIS-OSM*"])
    run("GIS_Q_AHN")
    run("GIS_HOOGTELIJNEN")
    stage("g-ahn")


if __name__ == "__main__":
    {"ui": ui, "gis": gis}[sys.argv[1]]()
