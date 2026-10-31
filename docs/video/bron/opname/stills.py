"""Sharp stills for a promo video: top views stitched from
3 x 3 captures into 3840 x 2160 (2x), from my own Open CAD Studio."""
import sys, time, subprocess
from pathlib import Path
from PIL import Image
import rec
from citytest import clear

OUT = rec.V.parent / "stills"
OUT.mkdir(exist_ok=True)
CROP = (176, 30, 1744, 912)          # the viewport without the host's overlays
CW, CH = CROP[2] - CROP[0], CROP[3] - CROP[1]


def stitch(name, cx, cy, wt, jpg=True):
    """A 3840 x 2160 top view of a box `wt` wide around (cx, cy)."""
    s = wt / (3 * CW)                          # world units per captured pixel
    ht = 3 * CH * s
    canvas = Image.new("RGB", (3 * CW, 3 * CH))
    tmp = rec.V / "citytest" / "tile.png"
    for j in range(3):
        for i in range(3):
            xc = cx - wt / 2 + (i + 0.5) * CW * s
            yc = cy + ht / 2 - (j + 0.5) * CH * s
            # The crop's centre lies (509 - 471) px above the viewport's centre.
            yv = yc - (509 - (CROP[1] + CROP[3]) / 2) * s
            b = (xc - 960 * s, yv - 509 * s, xc + 960 * s, yv + 509 * s)
            rec.capture(tmp, bounds=b)
            rec.capture(tmp, bounds=b)
            canvas.paste(Image.open(tmp).convert("RGB").crop(CROP), (i * CW, j * CH))
    im = canvas.resize((3840, 2160), Image.LANCZOS)
    path = OUT / (name + (".jpg" if jpg else ".png"))
    im.save(path, quality=92) if jpg else im.save(path, optimize=True)
    print("still", path.name, flush=True)


def run(cmd, t=900):
    r = rec.OCS.run(cmd, timeout=t)
    print(cmd[:60], r["ok"], r["message"][:100], flush=True)
    return r


def start(dwg):
    rec.start(dwg)
    time.sleep(10)
    rec.cancel()
    subprocess.run(["wmctrl", "-i", "-r", rec.window(), "-b", "add,fullscreen"])
    time.sleep(2)
    rec.host("CLEANSCREEN")
    time.sleep(1.5)
    rec.host("UCSICON OFF")
    rec.host("CURSORSIZE 1")
    rec.OCS.request("pointer", {"document_id": rec.OCS.document_id(), "events": [{"move": [-50, -50]}]})
    rec.host("-VIEW TOP")
    rec.host("VSCURRENT 2DWIREFRAME")


def world():
    start(rec.V / "world7.dwg")
    places = [
        ("wereld-parijs", 2.3490, 48.8530, "EPSG:2154", "w-fr-ign-bdortho-2154", "w-fr-ign-parcellaireexpress-map-2154", 300),
        ("wereld-keulen", 6.9583, 50.9413, "EPSG:25832", "w-de-nw-geobasisnrw-dop-25832", "w-de-nw-geobasisnrw-alkis-25832", 300),
        ("wereld-zurich", 8.5437, 47.3717, "EPSG:2056", "w-ch-swisstopo-swissimage-2056", "w-ch-geodienste-av-liegenschaften-2056", 300),
        ("wereld-barcelona", 2.1744, 41.4036, "EPSG:25831", "w-es-ct-icgc-orthophoto-wms-25831", None, 300),
        ("wereld-wenen", 16.3731, 48.2084, "EPSG:31256", "w-at-w-wien-orthofoto-3857", None, 300),
    ]
    for name, lon, lat, crs, aerial, overlay, w in places:
        clear()
        p = run(f"GIS_SITE {lon} {lat} {w * 1.4:.0f} {w:.0f} {crs} {name}")["data"]["projectlocatie"]
        run(f"GIS_Q_SERVER {aerial} RES=0.1")
        stitch(name + "-luchtfoto", p["x"], p["y"], w)
        if overlay:
            run(f"GIS_Q_SERVER {overlay} RES=0.1")
            stitch(name + "-kadaster", p["x"], p["y"], w)


def nl():
    start(rec.V / "world7.dwg")
    clear()
    run("GIS_SITE RD 109000 423500 32000 20000 Drechtsteden en Biesbosch")
    run("GIS_Q_SERVER pdok-luchtfoto RES=6", 1800)
    stitch("nl-regio-luchtfoto", 109000, 423300, 30000)
    run("GIS_Q_NATURA2000", 1800)
    stitch("nl-natura2000", 109000, 423300, 30000)
    clear()
    run("GIS_Q_SERVER pdok-luchtfoto RES=6", 1800)
    run("GIS_Q_GEOMORFOLOGIE", 1800)
    stitch("nl-geomorfologie", 109000, 423300, 30000)
    clear()
    run("GIS_SITE RD 104900 425300 7000 4500 Dordrecht")
    run("GIS_Q_SERVER JAAR=1900 RES=1.2", 1800)
    stitch("nl-topotijdreis-1900", 104950, 425300, 5000)
    clear()
    run("GIS_Q_SERVER JAAR=1850 RES=1.2", 1800)
    stitch("nl-topotijdreis-1850", 104950, 425300, 5000)
    clear()
    run("GIS_SITE RD 104885 425330 700 450 Grote Kerk")
    run("GIS_Q_HISGIS", 1800)
    stitch("nl-hisgis-1832", 104885, 425330, 420)
    clear()
    run("GIS_Q_SERVER pdok-luchtfoto RES=0.08", 1800)
    stitch("nl-grote-kerk-luchtfoto", 104885, 425330, 300)
    run("GIS_Q_BGT", 1800)
    stitch("nl-grote-kerk-bgt", 104885, 425330, 300)
    run("GIS_Q_BRK", 1800)
    stitch("nl-grote-kerk-bgt-kadaster", 104885, 425330, 300)


def tekening():
    start("/home/aia10/Documenten/Open-Infra/examples/dordrecht-nlcs/dordrecht-grote-kerk-nlcs.dwg")
    stitch("2d-nlcs-tekenblad", 104928, 425320, 640, jpg=False)
    stitch("2d-nlcs-klic-detail", 104864, 425325, 110, jpg=False)
    stitch("2d-nlcs-detail", 104900, 425330, 230, jpg=False)


if __name__ == "__main__":
    for step in sys.argv[1:]:
        {"world": world, "nl": nl, "tekening": tekening}[step]()
