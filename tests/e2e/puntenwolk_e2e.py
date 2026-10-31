#!/usr/bin/env python3
"""End-to-end test of the point cloud tools in Open CAD Studio.

Runs its own Open CAD Studio with an isolated configuration folder
(``XDG_CONFIG_HOME`` under target/e2e-puntenwolk), so the plugin installed
for daily use is left alone. A window appears, as with ``ocs_e2e.py``.

Dordrecht (AHN is classified), ``GIS_SITE RD 104885 425320 200 150``:
  tin_ahn       ``CIV_TIN BRON=PUNTENWOLK``: a TIN from the AHN5 ground
                points, thinned to 10 cm, on B-…-OG-TRIANGULATIE.
  contours      ``CIV_HOOGTELIJNEN`` on it; analysis and earthwork too.
  profiles      an axis across the site; the length profile over the TIN
                and from a strip of points (``BRON=PUNTENWOLK``) agree;
                cross sections from points.
  mesh_ahn      ``CIV_MESH`` of a footbridge over the Voorstraathaven.
A synthetic site (tests/data/puntenwolk, made by maak_testwolk.py):
  tin_las       ``CIV_TIN BESTAND=onbewerkt.las``: an unclassified scan;
                the ground filter finds the ground (no roof, no crowns).
  ply           ``GIS_BESTAND brug.ply``: a PLY imported as a scan (RCS).
  mesh_ply      ``CIV_MESH BESTAND=brug.ply``: a mesh of the bridge.
  ifc_export    ``CIV_IFC``: the TIN goes out as terrain, the mesh as a civil
                element.

Captures go to target/e2e-puntenwolk/*.png, the drawings and the report
(rapport.md) beside them.

Usage
  python3 tests/e2e/puntenwolk_e2e.py [--ocs PATH] [--port 18811] [--geen-install]
"""

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ocs_e2e  # noqa: E402

REPO = ocs_e2e.REPO
OUT = REPO / "target" / "e2e-puntenwolk"
DATA = REPO / "tests" / "data" / "puntenwolk"
SITE = [104785, 425245, 104985, 425395]
TEST_SITE = [105380, 425230, 105500, 425370]


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18811)
    ap.add_argument("--geen-install", action="store_true", help="use the plugins already installed in the test folder")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.geen_install:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    cfg = xdg / "OpenCADStudio" / "open-infra"
    cfg.mkdir(parents=True, exist_ok=True)
    (cfg / "settings.json").write_text(json.dumps({"language": "nl"}))
    api = cfg / "api"
    ocs_e2e.OUT = OUT
    ocs = ocs_e2e.Ocs(args.ocs, args.port)
    results = []
    data = {}

    def step(name, fn):
        t = time.time()
        try:
            ok, detail = fn()
        except Exception as e:  # noqa: BLE001
            ok, detail = False, f"{type(e).__name__}: {e}"
        results.append((name, ok, detail, time.time() - t))
        print(("✔" if ok else "✘"), name, f"({time.time() - t:.0f} s) —", detail, flush=True)

    def civ(cmd, rid, timeout=900):
        """Run a command with ID= and wait for its JSON answer."""
        f = api / f"{rid}.json"
        if f.exists():
            f.unlink()
        ocs.run(f"{cmd} ID={rid}")
        t = time.time()
        while time.time() - t < timeout:
            if f.exists():
                try:
                    r = json.loads(f.read_text())
                    data[rid] = r
                    return r
                except ValueError:
                    pass
            time.sleep(1)
        raise TimeoutError(f"{cmd.split()[0]}: no answer in {timeout} s")

    def capture(name, bounds=None):
        path = OUT / f"{name}.png"
        if path.exists():
            path.unlink()
        body = {"path": str(path), "document_id": ocs.doc(), "max_dimension": 1600}
        if bounds:
            body.update({"view": "region", "bounds": bounds})
        ocs.call("/capture", body, timeout=180)
        t = time.time()
        while not (path.exists() and path.stat().st_size > 0) and time.time() - t < 60:
            time.sleep(0.25)
        time.sleep(0.5)
        return path.exists()

    def plan(name, bounds):
        ocs.run("-VIEW TOP")
        ocs.run("VSCURRENT WIREFRAME2D")
        return capture(name, bounds)

    def view3d(name, window=None):
        ocs.run("-VIEW SWISO")
        ocs.run("VSCURRENT GOURAUDSHADED")
        if window:
            ocs.run(f"ZOOM W {window[0]},{window[1]} {window[2]},{window[3]}")
        else:
            ocs.run("ZOOM EXTENTS")
        time.sleep(2)
        return capture(name)

    def layer_with(part):
        return [n for n in ocs.layers() if part in n]

    def object_data(layer):
        h = ocs.first(layer)
        return ocs.xdata(h) if h else []

    # ── Dordrecht: AHN ─────────────────────────────────────────────────────
    def site():
        r = civ("GIS_SITE RD 104885 425320 200 150 Dordrecht", "site", 120)
        return r["ok"], r["message"]

    def tin_ahn():
        r = civ("CIV_TIN BRON=PUNTENWOLK", "tin_ahn")
        d = r.get("data", {})
        if not r["ok"]:
            return False, r["message"]
        layer = d.get("laag", "")
        xd = object_data(layer)
        ok = (
            d.get("driehoeken", 0) > 2000
            and layer.startswith("B-") and "-OG-TRIANGULATIE-" in layer
            and not d.get("grondfilter")
            and -3 < d.get("hoogte_min", 99) < d.get("hoogte_max", -99) < 15
            and (d.get("max_fout_m", 1) <= 0.1001 or d.get("begrensd"))
            and any("bron=AHN5-puntenwolk" in str(x) for x in xd)
        )
        return ok, f"{r['message']} | XDATA {xd[:3]}"

    def contours():
        r = civ("CIV_HOOGTELIJNEN INTERVAL=0.5", "hoogtelijnen", 600)
        n = sum(len(ocs.handles(l)) for l in layer_with("OG-HOOGTELIJN"))
        return r["ok"] and n > 10, f"{r['message']} | {n} on OG-HOOGTELIJN"

    def analysis_and_volume():
        a = civ("CIV_ANALYSE", "analyse", 600)
        v = civ("CIV_VOLUME ONTWERP=PEIL PEIL=1.5", "volume", 600)
        d = v.get("data", {})
        ok = a["ok"] and v["ok"] and d.get("afgraven_m3", 0) + d.get("ophogen_m3", 0) > 100
        return ok, f"{a['message']} | {v['message']}"

    def profiles():
        a = civ("CIV_AS PI=104795,425255;104975,425385 NAAM=Puntenwolkas", "as", 300)
        h = a.get("data", {}).get("as")
        if not (a["ok"] and h):
            return False, a["message"]
        tin = civ(f"CIV_LENGTEPROFIEL AS={h}", "lp_tin", 600)
        cloud = civ(f"CIV_LENGTEPROFIEL AS={h} BRON=PUNTENWOLK STROOK=1", "lp_wolk", 900)
        sec = civ(f"CIV_DWARSPROFIELEN AS={h} BRON=PUNTENWOLK OM=50 LINKS=10 RECHTS=10", "dp_wolk", 900)
        if not (tin["ok"] and cloud["ok"] and sec["ok"]):
            return False, f"{tin['message']} | {cloud['message']} | {sec['message']}"
        dt, dc = tin["data"], cloud["data"]
        lo, hi = abs(dt["maaiveld_min"] - dc["maaiveld_min"]), abs(dt["maaiveld_max"] - dc["maaiveld_max"])
        ok = lo < 0.5 and hi < 0.5 and "puntenwolk" in cloud["message"]
        return ok, f"TIN {dt['maaiveld_min']:.2f}–{dt['maaiveld_max']:.2f} m, points {dc['maaiveld_min']:.2f}–{dc['maaiveld_max']:.2f} m | {cloud['message']} | {sec['message']}"

    def mesh_ahn():
        # A footbridge over the Voorstraathaven (BGT overbruggingsdeel).
        r = civ("CIV_MESH GEBIED=104790,425275;104813,425297 SOORT=brug NAAM=Brug_Voorstraathaven", "mesh_ahn", 900)
        d = r.get("data", {})
        if not r["ok"]:
            return False, r["message"]
        layer = d.get("laag", "")
        xd = object_data(layer)
        ok = d.get("driehoeken", 0) > 500 and "-KW-KUNSTWERK_BRUG-" in layer and any("driehoeken=" in str(x) for x in xd)
        return ok, f"{r['message']} | XDATA {xd[:4]}"

    def capture_tin():
        a = plan("dordrecht_tin_plan", SITE)
        b = view3d("dordrecht_tin_3d")
        return a and b, "dordrecht_tin_plan.png, dordrecht_tin_3d.png"

    def capture_contours():
        return plan("dordrecht_hoogtelijnen", SITE), "dordrecht_hoogtelijnen.png"

    def capture_bridge():
        a = plan("dordrecht_brug_plan", [104780, 425265, 104823, 425305])
        b = view3d("dordrecht_brug_3d", [104780, 425265, 104823, 425305])
        return a and b, "dordrecht_brug_plan.png, dordrecht_brug_3d.png"

    def capture_profiles():
        ocs.run("-VIEW TOP")
        ocs.run("VSCURRENT WIREFRAME2D")
        ocs.run("ZOOM EXTENTS")
        time.sleep(2)
        return capture("dordrecht_profielen"), "dordrecht_profielen.png"

    def save_dordrecht():
        path = OUT / "puntenwolk_dordrecht.dwg"
        if path.exists():
            path.unlink()
        r = ocs.run(f"SAVEAS {path}")
        time.sleep(2)
        return path.exists(), f"{path} ({r.get('status')})"

    # ── Synthetic data: unclassified LAS, PLY ──────────────────────────────
    def new_drawing():
        ocs_e2e.nieuwe_tekening(ocs, "testwolk_start")
        time.sleep(3)
        r = civ("GIS_SITE RD 105440 425300 120 140 Testwolk", "site2", 120)
        return r["ok"], r["message"]

    def tin_las():
        r = civ(f"CIV_TIN BESTAND={DATA / 'onbewerkt.las'}", "tin_las", 600)
        d = r.get("data", {})
        # True ground 0.5–1.9 m; roof 7 m higher, crowns 3–8 m higher.
        ok = r["ok"] and d.get("grondfilter") is True and 0.3 < d.get("hoogte_min", 0) and d.get("hoogte_max", 99) < 2.5
        return ok, r["message"]

    def contours_las():
        r = civ("CIV_HOOGTELIJNEN INTERVAL=0.1", "hoogtelijnen_las", 600)
        return r["ok"], r["message"]

    def ply_import():
        before = ocs.entities()
        r = civ(f"GIS_BESTAND {DATA / 'brug.ply'}", "ply", 300)
        d = r.get("data", {})
        rcs = Path(d.get("rcs", "/nonexistent"))
        time.sleep(2)
        added = ocs.entities() - before
        ok = r["ok"] and d.get("punten") == 20648 and rcs.exists() and added >= 1
        return ok, f"{r['message']} | {rcs.name} {rcs.stat().st_size if rcs.exists() else 0} B | +{added} entity"

    def mesh_ply():
        r = civ(f"CIV_MESH BESTAND={DATA / 'brug.ply'} GEBIED=105405,425265;105435,425280 SOORT=brug NAAM=Testbrug", "mesh_ply", 600)
        d = r.get("data", {})
        ok = r["ok"] and d.get("driehoeken", 0) > 500 and d.get("afwijking_p95_m", 1) < 0.15
        return ok, r["message"]

    def ifc_export():
        # Main's IFC 4.3 export picks up the point cloud TIN (terrain) and the mesh (civil element).
        path = OUT / "puntenwolk_testwolk.ifc"
        r = civ(f"CIV_IFC BESTAND={path}", "ifc", 600)
        k = r.get("data", {}).get("klassen", {})
        ok = r["ok"] and path.exists() and k.get("IfcGeographicElement", 0) >= 1 and k.get("IfcCivilElement", 0) >= 1
        if ok:
            text = path.read_text(errors="replace")
            ok = "'Testbrug'" in text and ".TERRAIN." in text
        return ok, f"{r['message']} | {k}"

    def capture_test_tin():
        return view3d("testwolk_tin_3d"), "testwolk_tin_3d.png"

    def capture_test_contours():
        return plan("testwolk_hoogtelijnen", [105395, 425295, 105485, 425365]), "testwolk_hoogtelijnen.png"

    def capture_test_bridge():
        return view3d("testwolk_brug_3d", [105405, 425262, 105435, 425282]), "testwolk_brug_3d.png"

    def capture_test_scan():
        return view3d("testwolk_brug_scan", [105405, 425262, 105435, 425282]), "testwolk_brug_scan.png"

    def save_test():
        path = OUT / "puntenwolk_testwolk.dwg"
        if path.exists():
            path.unlink()
        r = ocs.run(f"SAVEAS {path}")
        time.sleep(2)
        return path.exists(), f"{path} ({r.get('status')})"

    try:
        step("site", site)
        step("tin_ahn", tin_ahn)
        step("capture_tin", capture_tin)
        step("contours", contours)
        step("capture_contours", capture_contours)
        step("mesh_ahn", mesh_ahn)
        step("capture_bridge", capture_bridge)
        step("analysis_volume", analysis_and_volume)
        step("profiles", profiles)
        step("capture_profiles", capture_profiles)
        step("save_dordrecht", save_dordrecht)
        step("new_drawing", new_drawing)
        step("tin_las", tin_las)
        step("capture_test_tin", capture_test_tin)
        step("contours_las", contours_las)
        step("capture_test_contours", capture_test_contours)
        step("mesh_ply", mesh_ply)
        step("capture_test_bridge", capture_test_bridge)
        step("ply_import", ply_import)
        step("capture_test_scan", capture_test_scan)
        step("ifc_export", ifc_export)
        step("save_test", save_test)
    finally:
        ocs.stop()
    lines = ["# Point cloud e2e", "", "| Step | OK | s | Detail |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {s:.0f} | {d.replace('|', '/')} |" for n, ok, d, s in results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    (OUT / "results.json").write_text(json.dumps(data, indent=1, ensure_ascii=False))
    failed = [n for n, ok, _, _ in results if not ok]
    print(f"\n{len(results) - len(failed)}/{len(results)} steps passed" + (f"; failed: {', '.join(failed)}" if failed else ""))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
