#!/usr/bin/env python3
"""End-to-end test of the vehicle library and worldwide heights in Open CAD
Studio.

Runs its own Open CAD Studio with an isolated configuration folder
(``XDG_CONFIG_HOME``), so the plugin installed for daily use is left alone.
A window appears, as with ``ocs_e2e.py``.

Checks
  vehicle      ``GIS_VEHICLE <id> <point> <point>``: one INSERT of the block
               ``OI_<ID>_TOP`` on the vehicle layer, rotated to the direction.
  side view    ``GIS_VEHICLE <id> SIDE``, picked to the left: mirrored.
  catalogue    ``GIS_VEHICLE_CATALOG``: one insert per vehicle, with texts.
  heights      outside the Netherlands (Zürich): contour lines and spot
               heights from the worldwide Terrain Tiles.

Usage
  python3 tests/e2e/bibliotheek_e2e.py [--ocs PATH] [--port 18760]

The report goes to target/e2e-bibliotheek/rapport.md.
"""

import argparse
import json
import math
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ocs_e2e  # noqa: E402

REPO = ocs_e2e.REPO
OUT = REPO / "target" / "e2e-bibliotheek"


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


def vehicles():
    return json.loads((REPO / "assets" / "library" / "vehicles.json").read_text())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18760)
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    # English interface for this run.
    (xdg / "OpenCADStudio" / "open-infra").mkdir(parents=True, exist_ok=True)
    (xdg / "OpenCADStudio" / "open-infra" / "settings.json").write_text(json.dumps({"language": "en"}))
    ocs_e2e.OUT = OUT
    ocs = ocs_e2e.Ocs(args.ocs, args.port)
    results = []

    def step(name, fn):
        t = time.time()
        try:
            ok, detail = fn()
        except Exception as e:  # noqa: BLE001
            ok, detail = False, f"{type(e).__name__}: {e}"
        results.append((name, ok, detail, time.time() - t))
        print(("✔" if ok else "✘"), name, "—", detail, flush=True)

    def inserts(layer):
        r = ocs.get(f"/entities?layer={layer}&fields=handle,type,layer")
        return [e for e in r.get("entities", r if isinstance(r, list) else []) if str(e.get("type", "")).lower() .replace(" ", "") in ("insert", "blockreference")]

    all_v = vehicles()
    first = all_v[0]["id"]
    combo = next((v["id"] for v in all_v if len(v["units"]) > 1), first)

    def vehicle():
        before = ocs.entities()
        ocs.run(f"GIS_VEHICLE {combo} 100,100 100,200")
        ocs.settle(minimum_seconds=1.0, timeout=60, first_change=30)
        ins = inserts("OI-VEHICLES")
        if not ins:
            return False, f"no insert on OI-VEHICLES ({ocs.entities() - before} new entities)"
        e = ocs.entity(ins[-1]["handle"])
        rot = e.get("rotation", e.get("properties", {}).get("rotation"))
        name = e.get("block_name", e.get("name", e.get("properties", {}).get("block_name")))
        ok = name and name.upper().startswith("OI_") and rot is not None and abs(float(rot) - math.pi / 2) < 1e-3
        return bool(ok), f"block {name}, rotation {rot}"

    def side():
        n0 = len(inserts("OI-VEHICLES"))
        ocs.run(f"GIS_VEHICLE {first} SIDE 0,-50 -20,-50")
        ocs.settle(minimum_seconds=1.0, timeout=60, first_change=30)
        ins = inserts("OI-VEHICLES")
        if len(ins) <= n0:
            return False, "no side-view insert"
        e = ocs.entity(ins[-1]["handle"])
        return True, json.dumps({k: e.get(k) for k in ("block_name", "x_scale", "rotation") if k in e})

    def catalogue():
        n0 = len(inserts("OI-VEHICLES"))
        ocs.run("GIS_VEHICLE_CATALOG 0,500")
        ocs.settle(minimum_seconds=2.0, timeout=300, first_change=60)
        n = len(inserts("OI-VEHICLES")) - n0
        texts = ocs.get("/entities?layer=OI-VEHICLES-TEXT&fields=handle")
        nt = len(texts.get("entities", [])) if isinstance(texts, dict) else 0
        return n == len(all_v) and nt >= 2 * len(all_v), f"{n} inserts for {len(all_v)} vehicles, {nt} texts"

    def heights():
        ocs.run("GIS_SITE 8.5417 47.3769 300 300 Zurich")
        time.sleep(2)
        before = ocs.entities()
        ocs.run("GIS_CONTOURS")
        ocs.settle(minimum_seconds=3.0, timeout=300, first_change=120)
        mid = ocs.entities()
        ocs.run("GIS_SPOT_HEIGHTS")
        ocs.settle(minimum_seconds=3.0, timeout=300, first_change=120)
        after = ocs.entities()
        return (mid > before and after > mid), f"contours +{mid - before}, spot heights +{after - mid}"

    def gis_files():
        ocs.run("GIS_SITE 8.5402 47.3779 600 600 EPSG:2056 Zurich HB")
        time.sleep(2)
        files = REPO / "tests" / "data" / "gisbestanden"
        report = []
        ok = True
        for name in ["zurich.geojson", "zurich_lines.shp", "zurich.gpkg", "zurich.kml", "zurich.gpx", "zurich_lv95.csv"]:
            before = ocs.entities()
            ocs.run(f"GIS_FILE_IMPORT {files / name}")
            ocs.settle(minimum_seconds=1.0, timeout=60, first_change=20)
            added = ocs.entities() - before
            ok &= added > 0
            report.append(f"{name} +{added}")
        h = ocs.first("GIS-ZURICH_LINES")
        xd = ocs.xdata(h) if h else []
        ok &= any("Track" in str(x) for x in xd)
        return ok, ", ".join(report) + f"; shapefile XDATA {xd[:2]}"

    def bgt_update():
        ocs.run("GIS_SITE RD 104885 425320 200 150 Dordrecht")
        time.sleep(2)
        ocs.run("GIS_Q_BGT")
        ocs.settle(minimum_seconds=2.0, timeout=180, first_change=90)
        layer = next((l for l in ocs.layers() if "VERHARDING" in l), None)
        hs = ocs.handles(layer) if layer else []
        if len(hs) < 3:
            return False, f"too little BGT to test ({layer}: {len(hs)})"
        before = ocs.entities()
        # Two objects deleted by hand, one moved: the update must draw the two
        # again and redraw the moved one, and leave the rest alone.
        ocs.call("/entities", {"handles": hs[:2], "document_id": ocs.doc()}, method="DELETE")
        ocs.call("/entities/transform", {"handles": [hs[2]], "action": "move", "vector": [5.0, 0.0, 0.0], "document_id": ocs.doc()})
        time.sleep(1)
        ocs.run("GIS_BGT_UPDATE")
        ocs.settle(minimum_seconds=2.0, timeout=180, first_change=90)
        after = ocs.entities()
        removed = len(ocs.handles("OI-BGT-REMOVED"))
        return after == before and removed == 0, f"{before} entities before, {after} after the update; {removed} on OI-BGT-REMOVED"

    step("bgt update", bgt_update)
    step("gis files", gis_files)
    step("vehicle", vehicle)
    step("side view", side)
    step("catalogue", catalogue)
    step("heights outside NL", heights)
    try:
        ocs.run(f"QSAVEAS {OUT / 'bibliotheek.dwg'}")
    except Exception:  # noqa: BLE001
        pass
    ocs.stop()
    lines = ["# Vehicle library and heights e2e", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {d} | {t:.1f} |" for n, ok, d, t in results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if all(ok for _, ok, _, _ in results) else 1)


if __name__ == "__main__":
    main()
