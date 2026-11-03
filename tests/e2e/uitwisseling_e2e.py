#!/usr/bin/env python3
"""End-to-end test of the LandXML export (and the corridor in the IFC 4.3
export) in Open CAD Studio.

Runs its own Open CAD Studio (REST on --port, default 18813) with an
isolated configuration folder (``XDG_CONFIG_HOME``), so the plugin
installed for daily use is left alone. A window appears.

Builds a small civil model in Dordrecht (``GIS_SITE RD 104885 425320 200
150 Dordrecht``):

  tin          CIV_TIN BRON=AHN: the existing ground (status B)
  points       CIV_PUNT: two survey points with code
  axis         CIV_AS PI=… R=…: an alignment from PIs with a curve
  profiles     CIV_LENGTEPROFIEL and CIV_ONTWERPPROFIEL (PVIs, curves)
  corridor     CIV_CORRIDOR SJABLOON=weg: design TIN (N), feature lines
  landxml      CIV_LANDXML_EXPORT … ALLES, validated against the LandXML
               1.2 XSD
  round trip   the file read back with CIV_LANDXML into a new drawing
  ifc          CIV_IFC BESTAND=… (main's IFC 4.3 export), opened and
               validated with ifcopenshell (schema and EXPRESS rules),
               alignment lengths, stations and georeferencing read back; the
               corridor as IfcRoad with road parts, courses and earthwork
               bodies, their volumes against the corridor's own
  selection    only the selected axis is exported
  millimetres  a drawing in mm with a relative site: the same lengths

Validation needs a venv with lxml and ifcopenshell (PyPI) under
target/e2e-uitwisseling/venv (see uitwisseling_check.py). Nothing is
uploaded anywhere.

Usage
  python3 tests/e2e/uitwisseling_e2e.py [--ocs PATH] [--port 18813]

The report goes to target/e2e-uitwisseling/rapport.md.
"""

import argparse
import json
import math
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ocs_e2e  # noqa: E402

REPO = ocs_e2e.REPO
OUT = REPO / "target" / "e2e-uitwisseling"
VENV_PY = OUT / "venv" / "bin" / "python"
SITE = (104885.0, 425320.0)


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


def check(**files):
    args = [str(VENV_PY), str(REPO / "tests" / "e2e" / "uitwisseling_check.py")]
    for k, v in files.items():
        args += [f"--{k}", str(v)]
    r = subprocess.run(args, capture_output=True, text=True, timeout=900)
    try:
        return json.loads(r.stdout)
    except ValueError:
        raise RuntimeError(f"uitwisseling_check.py: {r.stderr[-800:]}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18813)
    ap.add_argument("--no-install", action="store_true", help="use the plugins already in the isolated folder")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.no_install:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    api_dir = xdg / "OpenCADStudio" / "open-infra" / "api"
    ocs_e2e.OUT = OUT
    ocs = ocs_e2e.Ocs(args.ocs, args.port)
    results = []
    state = {}

    def step(name, fn):
        t = time.time()
        try:
            ok, detail = fn()
        except Exception as e:  # noqa: BLE001
            ok, detail = False, f"{type(e).__name__}: {e}"
        results.append((name, ok, detail, time.time() - t))
        print(("✔" if ok else "✘"), name, "—", detail, flush=True)
        return ok

    def run_id(cmd, rid, timeout=600):
        """Run an Open Infra command with ID= and wait for its JSON result."""
        path = api_dir / f"{rid}.json"
        if path.exists():
            path.unlink()
        ocs.run(f"{cmd} ID={rid}")
        t0 = time.time()
        while time.time() - t0 < timeout:
            if path.exists():
                time.sleep(0.2)
                r = json.loads(path.read_text())
                if not r.get("ok"):
                    raise RuntimeError(r.get("message"))
                return r
            time.sleep(0.5)
        raise RuntimeError(f"{cmd.split()[0]}: no result within {timeout} s")

    def capture(name, bounds=None):
        path = OUT / f"{name}.png"
        if path.exists():
            path.unlink()
        body = {"path": str(path), "document_id": ocs.doc(), "max_dimension": 1400}
        if bounds:
            body.update({"view": "region", "bounds": bounds})
        ocs.call("/capture", body)
        t0 = time.time()
        while not (path.exists() and path.stat().st_size > 0) and time.time() - t0 < 60:
            time.sleep(0.25)
        return path.exists()

    def save(path):
        """Save the drawing as `path`: the automation save, else QSAVE of
        the start drawing and a copy."""
        try:
            ocs.call("/save", {"path": str(path), "document_id": ocs.doc()})
        except Exception:  # noqa: BLE001
            pass
        t0 = time.time()
        while not path.exists() and time.time() - t0 < 10:
            time.sleep(0.5)
        if not path.exists():
            ocs.run("QSAVE")
            time.sleep(3)
            shutil.copy(OUT / "start.dwg", path)
        return path.exists()

    def site():
        ocs.run("GIS_SITE RD 104885 425320 200 150 Dordrecht")
        time.sleep(2)
        return True, "GIS_SITE RD 104885 425320 200 150 Dordrecht"

    def tin():
        r = run_id("CIV_TIN BRON=AHN AFSTAND=5", "ux-tin")
        d = r["data"]
        state["ground"] = (d["hoogte_min"], d["hoogte_max"])
        return d["driehoeken"] > 500 and d["laag"].startswith("B-"), f"{d['driehoeken']} triangles on {d['laag']}, {d['hoogte_min']:.2f}–{d['hoogte_max']:.2f} m"

    def points():
        r = run_id("CIV_PUNT 104850,425300 104920,425340 NR=501 CODE=PUT", "ux-pt")
        return r["data"].get("punten") == 2, r["message"][:160]

    def axis():
        r = run_id("CIV_AS PI=104800,425270;104880,425290;104960,425360 R=60", "ux-as")
        d = r["data"]
        state["axis"] = d["as"]
        state["axis_len"] = d["lengte_m"]
        state["axis_name"] = d["naam"]
        return d["bogen"] == 1 and d["lengte_m"] > 150, f"{d['naam']} handle {d['as']}: {d['lengte_m']:.3f} m, {d['bogen']} curve"

    def profiles():
        r = run_id(f"CIV_LENGTEPROFIEL AS={state['axis']}", "ux-lp")
        lo, hi = r["data"]["maaiveld_min"], r["data"]["maaiveld_max"]
        mid = round((lo + hi) / 2, 2)
        state["pvi"] = [(0.0, mid), (80.0, mid + 0.6), (state["axis_len"], mid - 0.2)]
        pvi = ";".join(f"{s:.3f}:{z:.2f}" for s, z in state["pvi"])
        r2 = run_id(f"CIV_ONTWERPPROFIEL AS={state['axis']} PVI={pvi} BOOG=40", "ux-op")
        n = len(r2["data"]["pvi"])
        return n == 3, f"ground {lo:.2f}–{hi:.2f} m; design {pvi}, {n} PVIs"

    def corridor():
        r = run_id(f"CIV_CORRIDOR AS={state['axis']} SJABLOON=weg FREQUENTIE=10", "ux-cor", 900)
        d = r["data"]
        state["corridor"] = d
        return d["driehoeken"] > 0 and d["kantlijnen"] > 0, f"{d['dwarsprofielen']} sections, {d['driehoeken']} triangles, {d['kantlijnen']} feature lines, cut {d['afgraven_m3']:.0f} m³, fill {d['ophogen_m3']:.0f} m³"

    def landxml():
        path = OUT / "dordrecht.xml"
        r = run_id(f"CIV_LANDXML_EXPORT BESTAND={path} ALLES", "ux-lx")
        rep = check(landxml=path)["landxml"]
        state["landxml"] = rep
        (OUT / "landxml-check.json").write_text(json.dumps(rep, indent=1))
        states = sorted(s["state"] for s in rep["surfaces"])
        al = rep["alignments"][0] if rep["alignments"] else {}
        ok = (
            rep["valid"]
            and rep["epsg"] == "28992"
            and rep["cgpoints"] >= 2
            and states == ["existing", "proposed"]
            and any(s["breaklines"] > 0 for s in rep["surfaces"])
            and al.get("profsurf") == 1
            and al.get("pvi") == 3
            and al.get("curves") == 1
            and abs(al.get("length", 0) - state["axis_len"]) < 1e-3
            and rep["planfeatures"] > 0
        )
        return ok, f"XSD {'valid' if rep['valid'] else 'INVALID ' + str(rep['errors'][:3])}; {rep['cgpoints']} points, surfaces {[(s['name'], s['state'], s['faces']) for s in rep['surfaces']]}, alignment {al}, {rep['planfeatures']} plan features; {r['message'][:120]}"

    def ifc():
        path = OUT / "dordrecht.ifc"
        r = run_id(f"CIV_IFC BESTAND={path}", "ux-ifc", 900)
        state["ifc_result"] = r["data"]
        rep = check(ifc=path)["ifc"]
        state["ifc"] = rep
        (OUT / "ifc-check.json").write_text(json.dumps(rep, indent=1))
        mc = rep.get("map_conversion", {})
        al = rep["alignments"][0] if rep["alignments"] else {}
        fp = al.get("FootPrint_Curve2D", {})
        first_pi = (104800 - SITE[0], 425270 - SITE[1])
        last_pi = (104960 - SITE[0], 425360 - SITE[1])
        geo_ok = isinstance(fp, dict) and math.dist(fp.get("start", [9e9, 9e9])[:2], first_pi) < 1e-3 and math.dist(fp.get("end", [9e9, 9e9])[:2], last_pi) < 1e-3
        ok = (
            rep["error_count"] == 0
            and rep["bodies_failed"] == 0
            and abs(mc.get("eastings", 0) - SITE[0]) < 1e-6
            and abs(mc.get("northings", 0) - SITE[1]) < 1e-6
            and mc.get("crs") == "EPSG:28992"
            and mc.get("vertical") in ("NAP", "EPSG:5709")
            and abs(al.get("horizontal_length", 0) - state["axis_len"]) < 1e-3
            and abs(al.get("vertical_length", 0) - state["axis_len"]) < 1e-3
            and al.get("start_station") == 0.0
            and al.get("horizontal_types", [])[:3] == ["LINE", "CIRCULARARC", "LINE"]
            and "PARABOLICARC" in al.get("vertical_types", [])
            and geo_ok
            and rep["roads"]
            and sorted(rep["road_parts"]) == ["CARRIAGEWAY", "ROADSIDE", "SOFTSHOULDER"]
            and len(rep["courses"]) == 3
            and len(rep["terrain"]) >= 1
        )
        return ok, (
            f"{rep['issue_count']} validation issues, {rep['bodies']} bodies ({rep['bodies_failed']} failed); map conversion {mc}; "
            f"alignment {al.get('horizontal_types')} {al.get('horizontal_length', 0):.3f} m, vertical {al.get('vertical_types')}, "
            f"start station {al.get('start_station')}, footprint {fp.get('start') if isinstance(fp, dict) else fp} → {fp.get('end') if isinstance(fp, dict) else ''}; "
            f"roads {rep['roads']}, parts {rep['road_parts']}, courses {rep['courses']}"
        )

    def earthworks():
        e = state["ifc"]["earthworks"]
        fill = next((v for k, v in e.items() if k.startswith("IfcEarthworksFill")), None)
        cut = next((v for k, v in e.items() if k.startswith("IfcEarthworksCut")), None)
        c = state["corridor"]
        def show(b):
            return "none" if b is None else f"{b['volume']:.1f} m³ (closed {b['closed']}, voids {b['voids']})"
        detail = f"IFC fill {show(fill)}, cut {show(cut)}; corridor sections: fill {c['ophogen_m3']:.1f}, cut {c['afgraven_m3']:.1f} m³"
        ok = True
        for body, ref in ((fill, c["ophogen_m3"]), (cut, c["afgraven_m3"])):
            if ref > 20:
                ok &= body is not None and body["closed"] and abs(body["volume"] - ref) <= 0.15 * ref
        return ok, detail

    def round_trip():
        ocs_e2e.nieuwe_tekening(ocs, "terug")
        ocs.run("GIS_SITE RD 104885 425320 200 150 Dordrecht")
        time.sleep(2)
        r = run_id(f"CIV_LANDXML {OUT / 'dordrecht.xml'}", "ux-rt", 600)
        d = r["data"]
        rep = state["landxml"]
        back = {s["naam"]: s["driehoeken"] for s in d["oppervlakken"]}
        sent = {s["name"]: s["faces"] for s in rep["surfaces"]}
        axes = d.get("assen", [])
        ok = d["punten"] == rep["cgpoints"] and back == sent and len(axes) == 1 and axes[0]["naam"] == state["axis_name"]
        info = run_id("CIV_INFO", "ux-info")["data"]
        lengths = [a["lengte_m"] for a in info.get("assen", [])]
        ok &= any(abs(l - state["axis_len"]) < 1e-3 for l in lengths)
        return ok, f"points {d['punten']}, surfaces {back} (sent {sent}), axes {[a['naam'] for a in axes]} with lengths {lengths}"

    def selection():
        ocs.open(OUT / "uitwisseling.dwg")
        time.sleep(3)
        r = ocs.call("/select", {"handles": [state["axis"]], "document_id": ocs.doc()})
        sel = ocs.call("/get_selection", {"document_id": ocs.doc()})
        path = OUT / "alleen-as.xml"
        run_id(f"CIV_LANDXML_EXPORT BESTAND={path}", "ux-sel")
        rep = check(landxml=path)["landxml"]
        ok = rep["valid"] and len(rep["alignments"]) == 1 and not rep["surfaces"] and rep["cgpoints"] == 0
        return ok, f"select → {str(r)[:80]}, selection {str(sel)[:120]}; exported {len(rep['alignments'])} alignment(s), {len(rep['surfaces'])} surfaces, {rep['cgpoints']} points"

    def millimetres():
        ocs_e2e.nieuwe_tekening(ocs, "mm", "leeg-mm.dwg")
        ocs.run("GIS_SITE RD 104885 425320 200 150 RELATIEF Dordrecht")
        time.sleep(2)
        r = run_id("CIV_AS PI=104800,425270;104880,425290;104960,425360 R=60", "ux-mm-as")
        length = r["data"]["lengte_m"]
        path = OUT / "mm.ifc"
        run_id(f"CIV_IFC BESTAND={path}", "ux-mm-ifc")
        rep = check(ifc=path)["ifc"]
        al = rep["alignments"][0]
        fp = al.get("Axis_Curve2D", {})
        ok = rep["error_count"] == 0 and abs(length - state["axis_len"]) < 1e-3 and abs(al["horizontal_length"] - state["axis_len"]) < 1e-3
        ok &= isinstance(fp, dict) and math.dist(fp["start"][:2], (104800 - SITE[0], 425270 - SITE[1])) < 1e-3
        return ok, f"mm drawing: {length:.3f} m in the drawing, {al['horizontal_length']:.3f} m in IFC, start {fp.get('start') if isinstance(fp, dict) else fp}"

    try:
        step("site", site)
        if step("tin", tin):
            step("points", points)
        if step("axis", axis) and step("profiles", profiles) and step("corridor", corridor):
            ocs.call("/run", {"cmd": "ZOOM EXTENTS", "document_id": ocs.doc()})
            time.sleep(1)
            capture("plan", [104780, 425240, 104990, 425400])
            if step("landxml export", landxml):
                pass
            if step("ifc export", ifc):
                step("earthworks", earthworks)
            save(OUT / "uitwisseling.dwg")
            if "landxml" in state:
                step("round trip", round_trip)
            if (OUT / "uitwisseling.dwg").exists():
                step("selection", selection)
        step("millimetres", millimetres)
    finally:
        ocs.stop()
    lines = ["# LandXML and IFC export e2e", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {str(d).replace('|', '/')} | {t:.1f} |" for n, ok, d, t in results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if all(ok for _, ok, _, _ in results) else 1)


if __name__ == "__main__":
    main()
