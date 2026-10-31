"""Version 7 shots: map services at other places in the world, zooming out
over the Netherlands (Natura 2000, geomorphology, Topotijdreis, HisGIS),
3D with plain facades, KLIC in 3D under a see-through ground."""
import subprocess, sys, time
import rec
from shots import box, frame_bounds, layers, OUT
from record5 import CLUTTER, seq
from citytest import clear


def run(cmd, t=900):
    t0 = time.time()
    try:
        r = rec.OCS.run(cmd, timeout=t)
        print(f"{cmd[:60]} ({time.time() - t0:.0f} s): {r['ok']} {r['message'][:140]}", flush=True)
        return r
    except Exception as e:
        print(f"{cmd[:60]}: FOUT {str(e)[:200]}", flush=True)
        return None


def clean():
    rec.cancel()
    rec.host("NAVVCUBE")
    rec.host("UCSICON OFF")
    rec.host("CURSORSIZE 1")
    rec.OCS.request("pointer", {"document_id": rec.OCS.document_id(), "events": [{"move": [-50, -50]}]})
    rec.host("-VIEW TOP")
    rec.host("VSCURRENT 2DWIREFRAME")


def stage(name, a, b, n=66):
    d = OUT / name
    d.mkdir(parents=True, exist_ok=True)
    rec.capture(d / "warmup.png", bounds=a)
    rec.capture(d / "warmup.png", bounds=a)
    for i in range(n):
        rec.capture(d / f"{i:04d}.png", bounds=frame_bounds(a, b, i / (n - 1)))
    print(name, flush=True)


def rasters_off():
    d = rec.OCS.document_id()
    hs = [e["handle"] for e in rec.OCS.request("query", {"document_id": d, "type": "RasterImage", "detail": "summary", "limit": 50}).get("entities", [])]
    if hs:
        rec.OCS.request("entities_delete", {"document_id": d, "handles": hs})


PLACES = [
    # name, lon, lat, crs, aerial, cadastre
    ("parijs", 2.3490, 48.8530, "EPSG:2154", "w-fr-ign-bdortho-2154", "w-fr-ign-parcellaireexpress-map-2154"),
    ("wenen", 16.3731, 48.2084, "EPSG:31256", "w-at-w-wien-orthofoto-3857", "w-at-bev-dkm-grundstuecke-31287"),
    ("keulen", 6.9583, 50.9413, "EPSG:25832", "w-de-nw-geobasisnrw-dop-25832", "w-de-nw-geobasisnrw-alkis-25832"),
    ("barcelona", 2.1744, 41.4036, "EPSG:25831", "w-es-ct-icgc-orthophoto-wms-25831", "w-es-catastro-cadastralparcel-wms-25830"),
    ("zurich", 8.5437, 47.3717, "EPSG:2056", "w-ch-swisstopo-swissimage-2056", "w-ch-geodienste-av-liegenschaften-2056"),
]


def world(only=None):
    rec.start(rec.V / "world7.dwg")
    time.sleep(8)
    subprocess.run(["wmctrl", "-i", "-r", rec.window(), "-b", "add,fullscreen"])
    time.sleep(2)
    rec.host("CLEANSCREEN")
    time.sleep(1.5)
    clean()
    for name, lon, lat, crs, aerial, cad in PLACES:
        if only and name not in only:
            continue
        clear()
        r = run(f"GIS_SITE {lon} {lat} 360 260 {crs} {name}")
        if not r:
            continue
        p = r["data"]["projectlocatie"]
        x, y = p["x"], p["y"]
        a, b = box(x, y, 340), box(x, y, 230)
        run(f"GIS_Q_SERVER {aerial} RES=0.2")
        stage(f"w-{name}-0", a, b)
        run(f"GIS_Q_SERVER {cad} RES=0.2")
        stage(f"w-{name}-1", a, b)


def nl():
    rec.start(rec.V / "world7.dwg")
    time.sleep(8)
    subprocess.run(["wmctrl", "-i", "-r", rec.window(), "-b", "add,fullscreen"])
    time.sleep(2)
    rec.host("CLEANSCREEN")
    time.sleep(1.5)
    clean()
    clear()
    # Wide: Dordrecht, the Biesbosch and the rivers.
    run("GIS_SITE RD 109000 423500 32000 20000 Drechtsteden en Biesbosch")
    near, far = box(104900, 425330, 2600), box(109000, 423000, 30000)
    run("GIS_Q_LUCHTFOTO", 1800)
    stage("n-zoomuit", near, far, 90)
    run("GIS_Q_NATURA2000", 1800)
    stage("n-natura", far, box(110500, 422500, 22000))
    rasters_off()
    run("GIS_Q_GEOMORFOLOGIE", 1800)
    stage("n-geo", box(110500, 422500, 22000), box(106500, 424300, 12000))
    rasters_off()
    run("GIS_SITE RD 104900 425300 7000 4500 Dordrecht")
    run("GIS_Q_SERVER JAAR=1900 RES=1.5", 1800)
    stage("n-tijdreis", box(105200, 425200, 6400), box(104920, 425320, 2400))
    rasters_off()
    run("GIS_SITE RD 104885 425330 700 450 Grote Kerk")
    run("GIS_Q_HISGIS", 1800)
    stage("n-hisgis", box(104900, 425330, 650), box(104885, 425330, 330))


def ui_and_street():
    from record6 import window_seq
    rec.start(rec.V / "showcase7.dwg")
    time.sleep(10)
    rec.cancel()
    subprocess.run(["wmctrl", "-i", "-r", rec.window(), "-b", "add,fullscreen"])
    time.sleep(2)
    rec.host("UCSICON OFF")
    rec.host("CURSORSIZE 1")
    rec.OCS.request("pointer", {"document_id": rec.OCS.document_id(), "events": [{"move": [-50, -50]}]})
    KL = ["B-XX-KL-*", "B-XX-MO-*", "*-ONDERZOEK*"]
    layers(off=CLUTTER + KL)
    rec.action("ribbon_tab", "openaec.open-infra-civiel")
    window_seq("u-civiel", "SWISO", "GOURAUDSHADED", box(104930, 425330, 700), box(104900, 425330, 430), 4.0)
    rec.action("ribbon_tab", "openaec.open-infra")
    window_seq("u-gis", "SWISO", "GOURAUDSHADED", box(104900, 425330, 430), box(104885, 425335, 260), 4.0)
    # The street redesign, in the new model.
    rec.host("CLEANSCREEN")
    time.sleep(1.5)
    d = rec.OCS.document_id()
    for e in rec.OCS.request("query", {"document_id": d, "layer": "N-XX-OG-TRIANGULATIE-G", "detail": "summary", "limit": 50}).get("entities", []):
        rec.OCS.request("set_properties", {"document_id": d, "collection": "entities", "handle": e["handle"], "updates": [{"path": "/common/color", "value": {"Rgb": {"r": 52, "g": 140, "b": 235}}}]})
    layers(on=["B-XX-AM-AS-G", "B-XX-AM-KILOMETRERING-*", "N-XX-VH-*", "N-XX-GR-*", "N-XX-GW-TALUD*"])
    seq("f-straat", "SWISO", "GOURAUDSHADED", box(105000, 425615, 300), box(105015, 425630, 215), 2.6)


def ui_2d():
    from record6 import window_seq
    rec.start("/home/aia10/Documenten/Open-Infra/examples/dordrecht-nlcs/dordrecht-grote-kerk-nlcs.dwg")
    time.sleep(10)
    rec.cancel()
    subprocess.run(["wmctrl", "-i", "-r", rec.window(), "-b", "add,fullscreen"])
    time.sleep(2)
    rec.host("UCSICON OFF")
    rec.host("CURSORSIZE 1")
    rec.OCS.request("pointer", {"document_id": rec.OCS.document_id(), "events": [{"move": [-50, -50]}]})
    time.sleep(5)
    rec.action("ribbon_tab", "openaec.open-infra")
    time.sleep(2)
    rec.action("ribbon_tab", "openaec.open-infra")
    time.sleep(1)
    window_seq("u-2d", "TOP", "2DWIREFRAME", box(104928, 425320, 640, 420), box(104880, 425330, 330), 4.0)


def geo():
    rec.start(rec.V / "world7.dwg")
    time.sleep(8)
    subprocess.run(["wmctrl", "-i", "-r", rec.window(), "-b", "add,fullscreen"])
    time.sleep(2)
    rec.host("CLEANSCREEN")
    time.sleep(1.5)
    clean()
    clear()
    run("GIS_SITE RD 109000 423500 32000 20000 Drechtsteden en Biesbosch")
    run("GIS_Q_LUCHTFOTO", 1800)
    run("GIS_Q_GEOMORFOLOGIE", 1800)
    stage("n-geo", box(110500, 422500, 22000), box(106500, 424300, 12000))


def three_d():
    rec.start(rec.V / "showcase7.dwg")
    time.sleep(10)
    subprocess.run(["wmctrl", "-i", "-r", rec.window(), "-b", "add,fullscreen"])
    time.sleep(2)
    rec.host("CLEANSCREEN")
    time.sleep(1.5)
    clean()
    KL = ["B-XX-KL-*", "B-XX-MO-*", "B-OI-KL-*", "*-ONDERZOEK*"]
    layers(off=CLUTTER + KL)
    layers(off=CLUTTER + KL)
    BUILD = ["B-XX-OG-BEBOUWING*"]
    seq("f-intro", "SWISO", "GOURAUDSHADED", box(104920, 425300, 1000), box(104910, 425320, 780), 4.5)
    A, B = box(104900, 425330, 820), box(104895, 425330, 600)
    layers(off=BUILD)
    seq("f-opbouw0", "SWISO", "GOURAUDSHADED", A, B, 2.8, 0, 55)
    layers(on=BUILD)
    seq("f-opbouw1", "SWISO", "GOURAUDSHADED", A, B, 2.8, 30, 83)
    seq("f-kerk", "SWISO", "GOURAUDSHADED", box(104885, 425335, 250), box(104882, 425335, 185), 2.6)


def klic():
    rec.attach()
    clean()
    layers(on=["B-XX-KL-*", "B-OI-KL-*", "B-XX-MO-*", "*-ONDERZOEK*"])
    a, b = box(104868, 425300, 170), box(104872, 425306, 115)
    seq("k-dicht", "SWISO", "GOURAUDSHADED", a, b, 3.0)
    run("CIV_DOORZICHT PCT=78")
    seq("k-doorzicht", "SWISO", "GOURAUDSHADED", a, b, 3.0)
    # Down and under: orbit from above to below the street, a few pixels a frame.
    d = OUT / "k-onder"
    d.mkdir(parents=True, exist_ok=True)
    doc = rec.OCS.document_id()
    rec.host("-VIEW SWISO")
    time.sleep(0.8)
    fixed = box(104866, 425300, 125)
    rec.capture(d / "warmup.png", bounds=fixed)
    n = 120
    for i in range(n):
        ev = [{"move": [700, 400]}, {"press": "middle"}, {"move": [700 + 1.2, 400 - 3.2]}, {"release": "middle"}]
        rec.OCS.request("pointer", {"document_id": doc, "events": ev, "modifiers": "shift"})
        rec.capture(d / f"{i:04d}.png", bounds=fixed)
    print("k-onder", n, flush=True)
    run("CIV_DOORZICHT UIT")


if __name__ == "__main__":
    step = sys.argv[1]
    if step == "world":
        world(sys.argv[2:] or None)
    else:
        {"nl": nl, "geo": geo, "ui": ui_and_street, "ui2d": ui_2d, "3d": three_d, "klic": klic}[step]()
