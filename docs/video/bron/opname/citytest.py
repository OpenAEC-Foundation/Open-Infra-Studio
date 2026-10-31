"""CityGML/CityJSON import in my own Open CAD Studio: Cologne (NRW LoD2
CityGML 1.0, UTM32) and Dordrecht (3D BAG CityJSON + CityJSONSeq)."""
import json, os, subprocess, sys, time
import rec
from shots import box
D = rec.V / "citydata"
OUT = rec.V / "citytest"
OUT.mkdir(exist_ok=True)


def run(cmd, t=1800):
    t0 = time.time()
    r = rec.OCS.run(cmd, t)
    print(f"{cmd.split()[0]} ({time.time() - t0:.0f} s): {str(r)[:600]}", flush=True)
    return r


def shots(tag, cx, cy, w):
    rec.host("VSCURRENT GOURAUDSHADED")
    for view in ["SWISO", "NEISO"]:
        rec.host(f"-VIEW {view}")
        time.sleep(1)
        rec.capture(OUT / "warm.png", bounds=box(cx, cy, w))
        rec.capture(OUT / f"{tag}-{view.lower()}.png", bounds=box(cx, cy, w))
    rec.host("-VIEW TOP")


def clear():
    d = rec.OCS.document_id()
    hs, off = [], 0
    while True:
        es = rec.OCS.request("query", {"document_id": d, "detail": "summary", "limit": 5000, "offset": off}).get("entities", [])
        hs += [e["handle"] for e in es]
        off += len(es)
        if len(es) < 5000:
            break
    for i in range(0, len(hs), 1500):
        rec.OCS.request("entities_delete", {"document_id": d, "handles": hs[i:i + 1500]}, timeout=300)


if __name__ == "__main__":
    if "start" in sys.argv:
        rec.start(rec.V / "citytest.dwg")
        time.sleep(8)
    rec.cancel()
    if "koeln" in sys.argv:
        clear()
        run("GIS_SITE 6.9583 50.9413 1000 1000 EPSG:25832 Koeln Dom")
        run(f"CIV_CITYGML {D / 'koeln-356-5645.gml'}")
        shots("koeln", 356500, 5645500, 700)
    if "dordrecht" in sys.argv:
        clear()
        run("GIS_SITE RD 104920 425330 1000 800 Dordrecht binnenstad")
        run(f"CIV_CITYGML {D / 'dordrecht.city.json'}")
        shots("dordrecht-json", 104920, 425330, 600)
        clear()
        run(f"CIV_CITYGML {D / 'dordrecht.city.jsonl'} STIJL=maquette")
        shots("dordrecht-jsonl", 104920, 425330, 600)
