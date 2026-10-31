#!/usr/bin/env python3
"""End-to-end test of clothoids, design criteria, rail and sight distance in
Open CAD Studio (Dordrecht).

Runs its own Open CAD Studio with an isolated configuration folder
(``XDG_CONFIG_HOME`` under ``target/e2e-as-spoor``), so the plugin installed
for daily use is left alone. A window appears, as with ``ocs_e2e.py``.

Steps
  site + BGT     ``GIS_SITE RD 106000 423300 …`` (Dordrecht), the BGT
                 (buildings for the sight check in plan).
  road           ``CIV_AS PI=… R=150 A=100``: a clothoid–arc–clothoid; the
                 main points BO/BC/EC/EO, the clothoid records in XDATA.
  offset         ``CIV_AS_OFFSET`` beside the clothoids; ``CIV_STATION``.
  terrain        ``CIV_TIN BRON=AHN``: the existing ground for the sight check.
  profile        length and design profile of the road.
  criteria       ``CIV_ONTWERPCRITERIA`` ROA 2017 niet-hoofdbaan 70 km/h on
                 the axis; ``CIV_AS_TOETS``: exactly one violation, the
                 deliberate R 150 < 160 m, with its station and a marker.
  sight          ``CIV_ZICHT`` (NOA 80 km/h: 103 m) both ways: a deficit where
                 the building on the inside of the bend blocks the view.
  rail           ``CIV_SPOOR`` two tracks, V 80, R 600, clothoids of 80 m:
                 cant per curve, ramps and the cant diagram.
  turnout        ``CIV_WISSEL`` 1:9 to the right: the diverging track.
  IFC            ``CIV_IFC``: clothoids and cant in IFC 4.3, checked with
                 tests/ifc_validate.py when IfcOpenShell is in target/venv.
  captures       PNGs of the plan, the diagrams and the turnout.

Usage
  python3 tests/e2e/as_spoor_e2e.py [--ocs PATH] [--port 18815]

The report goes to target/e2e-as-spoor/rapport.md.
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
OUT = REPO / "target" / "e2e-as-spoor"

# Project location: Dordrecht (Sterrenburg), open ground between buildings.
SITE = (106000, 423300, 650, 450)
# Road: north-north-east, then a 92° right bend (R 150, A 100) with a
# building on the inside of the bend.
ROAD = "105830,423203;105925,423397;106229,423231"
BEND = (105930, 423340)
# Rail: south of the road, a gentle bend (R 600, clothoids of 80 m).
RAIL = "105720,423120;105980,423150;106250,423100"


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18815)
    ap.add_argument("--no-install", action="store_true", help="use the plugins already installed in the isolated folder")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.no_install:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    # Dutch interface: the drawing texts and codes as Dutch engineers read them.
    (xdg / "OpenCADStudio" / "open-infra").mkdir(parents=True, exist_ok=True)
    (xdg / "OpenCADStudio" / "open-infra" / "settings.json").write_text(json.dumps({"language": "nl"}))
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

    def result(rid, cmd, timeout=240):
        """Run a command with ID=rid and wait for api/<rid>.json."""
        f = api_dir / f"{rid}.json"
        if f.exists():
            f.unlink()
        ocs.run(f"{cmd} ID={rid}")
        t0 = time.time()
        while time.time() - t0 < timeout:
            if f.exists():
                time.sleep(0.3)
                return json.loads(f.read_text())
            time.sleep(0.5)
        raise TimeoutError(f"{rid}: geen resultaat")

    def layer_like(part):
        return [n for n in ocs.layers() if part in n]

    def capture(name, bounds=None, focus=None):
        path = OUT / f"{name}.png"
        if path.exists():
            path.unlink()
        body = {"path": str(path), "document_id": ocs.doc(), "max_dimension": 1800}
        if bounds:
            body.update({"view": "region", "bounds": bounds})
        if focus:
            body["focus_handles"] = focus
        r = ocs.call("/capture", body)
        t0 = time.time()
        while not (path.exists() and path.stat().st_size > 0) and time.time() - t0 < 60:
            time.sleep(0.25)
        return path.exists(), f"{path.name} {r.get('error') or ''}".strip()

    def site():
        x, y, w, h = SITE
        ocs.run(f"GIS_SITE RD {x} {y} {w} {h} Dordrecht")
        time.sleep(2)
        ocs.run("GIS_Q_BGT")
        c = ocs.settle(minimum_seconds=2.0, timeout=300, first_change=150)
        build = layer_like("-OG-BEBOUWING-")
        n = sum(len(ocs.handles(l)) for l in build)
        state["rect"] = ocs_e2e.site_rect(ocs)
        return n > 10, f"{c['entities']} elementen, {n} op {build[:1]}"

    def road():
        r = result("as1", f"CIV_AS PI={ROAD} R=150 A=100 NAAM=Rondweg")
        d = r.get("data", {})
        codes = [m["code"] for m in d.get("raakpunten", [])]
        state["as"] = d.get("as")
        state["len"] = d.get("lengte_m")
        xd = ocs.xdata(state["as"]) if state.get("as") else []
        spirals = [x for x in map(str, xd) if "spiraal" in x]
        ok = r["ok"] and codes == ["BA", "BO", "BC", "EC", "EO", "EA"] and len(d.get("overgangsbogen", [])) == 2 and len(spirals) == 2
        a = [round(s["A"], 1) for s in d.get("overgangsbogen", [])]
        return ok, f"{r['message']} | codes {codes} | A {a} | XDATA {spirals[:1]}"

    def offset_and_station():
        r = result("off1", f"CIV_AS_OFFSET AS={state['as']} VERSCHUIVING=-3.5;3.5")
        lines = r.get("data", {}).get("lijnen", [])
        # A point 2 m right of the axis in the middle of the bend.
        s = result("st1", f"CIV_STATION AS={state['as']} 105950,423350")
        pts = s.get("data", {}).get("punten", [])
        return r["ok"] and len(lines) == 2 and s["ok"] and len(pts) == 1, f"{r['message']} | {s['message']}"

    def tin():
        r = result("tin1", "CIV_TIN BRON=AHN AFSTAND=5", timeout=400)
        return r["ok"], r["message"]

    def profile():
        r = result("lp1", f"CIV_LENGTEPROFIEL AS={state['as']}", timeout=300)
        n = state["len"]
        p = result("op1", f"CIV_ONTWERPPROFIEL AS={state['as']} PVI=0:1.0;{n/2:.1f}:2.0;{n:.1f}:1.2 BOOG=120")
        return r["ok"] and p["ok"], f"{r['message']} | {p['message']}"

    def criteria():
        c = result("crit1", f"CIV_ONTWERPCRITERIA RICHTLIJN=roa CATEGORIE=niethoofdbaan V=70 AS={state['as']}")
        time.sleep(1)
        t = result("toets1", f"CIV_AS_TOETS AS={state['as']}")
        d = t.get("data", {})
        v = d.get("overtredingen", [])
        marks = layer_like("-AL-AANKLEDING-G")
        n_marks = sum(len(ocs.handles(l)) for l in marks)
        ok = c["ok"] and t["ok"] and len(v) == 1 and v[0]["toets"] == "boogstraal" and abs(v[0]["grens"] - 160) < 1e-6 and n_marks > 0 and d.get("ontwerpprofiel")
        state["violation"] = v[0] if v else None
        return ok, f"{c['message']} | {t['message']} | markers {n_marks}"

    def sight():
        # NOA 80 km/h: 103 m stopping sight; the building inside the bend blocks it.
        r = result("zicht1", f"CIV_ZICHT AS={state['as']} RICHTLIJN=noa CATEGORIE=autosnelweg V=80 OM=10 RICHTING=beide", timeout=400)
        d = r.get("data", {})
        st = d.get("stations", [])
        state["zicht"] = d
        deficits = d.get("tekorten", [])
        ok = r["ok"] and abs(d.get("nodig_m", 0) - 103) < 1e-6 and len(st) > 10 and d.get("obstakelranden", 0) > 0 and d.get("terrein") and any(t["oorzaak"] == "plattegrond" for t in deficits)
        worst = min((s["beschikbaar_m"] for s in st), default=None)
        return ok, f"{r['message']} | {len(st)} stations, min {worst} m, {len(d.get('tekorten', []))} tekort(en), {d.get('obstakelranden')} randen"

    def rail():
        r = result("sp1", f"CIV_SPOOR PI={RAIL} R=600 LS=80 V=80 VMIN=40 SPOREN=2 SPOORAFSTAND=4.5 NAAM=Spoor", timeout=300)
        d = r.get("data", {})
        state["spoor"] = d.get("as")
        bogen = d.get("bogen", [])
        ramps = d.get("overgangen", [])
        tracks = layer_like("-AM-AS_SPOOR_TREIN-G")
        n = sum(len(ocs.handles(l)) for l in tracks)
        cant = layer_like("-VH-VERKANTING-G")
        ok = r["ok"] and len(bogen) == 1 and abs(bogen[0]["deq_mm"] - 11.8 * 80 * 80 / 600) < 1e-6 and len(ramps) == 2 and n >= 2 and cant
        state["diagram"] = d.get("diagram_plaats")
        return ok, f"{r['message']} | D {bogen[0]['d_mm'] if bogen else None} mm, ramps {[round(x['helling_mm_per_m'], 2) for x in ramps]}"

    def turnout():
        r = result("w1", f"CIV_WISSEL AS={state['spoor']} STATION=60 TYPE=1:9 ZIJDE=rechts LENGTE=40")
        d = r.get("data", {})
        state["wissel"] = d
        ok = r["ok"] and d.get("type") == "1:9" and abs(d.get("hoek_graden", 0) - 6.3402) < 1e-3 and d.get("as")
        return ok, f"{r['message']} | WB {d.get('wisselbegin')} MS {d.get('mathematisch_snijpunt')}"

    def ifc():
        path = OUT / "as-spoor.ifc"
        if path.exists():
            path.unlink()
        r = result("ifc1", f"CIV_IFC BESTAND={path}", timeout=400)
        py = REPO / "target" / "venv" / "bin" / "python"
        if not path.exists():
            return False, r["message"]
        text = path.read_text(errors="replace")
        found = {k: text.count(k) for k in ("IFCCLOTHOID(", "IFCALIGNMENTCANT(", ".LINEARTRANSITION.", "IFCALIGNMENT(")}
        if not py.exists():
            return all(found.values()), f"{r['message']} | {found} | IfcOpenShell not installed in target/venv"
        v = subprocess.run([str(py), str(REPO / "tests" / "ifc_validate.py"), str(path)], capture_output=True, text=True)
        tail = " / ".join(l for l in v.stdout.splitlines() if "error" in l or "misfit" in l)
        return v.returncode == 0 and all(found.values()), f"{found} | {tail}"

    def captures():
        # The terrain mesh clutters the plan: freeze its layer.
        for l in layer_like("-OG-TRIANGULATIE-"):
            ocs.run(f"LAYER FREEZE {l}")
        try:
            ocs.run("REGEN")
        except Exception:  # noqa: BLE001
            pass
        ocs.run("ZOOM EXTENTS")
        time.sleep(1.5)
        shots = [capture("overzicht", focus=None)]
        x, y, w, h = SITE
        rect = state.get("rect") or [x - w / 2, y - h / 2, x + w / 2, y + h / 2]
        shots.append(capture("plan", rect))
        # Drawing = RD + offset (zero for an absolute project location).
        ox, oy = (rect[0] + rect[2]) / 2 - x, (rect[1] + rect[3]) / 2 - y
        # The road, and the bend with its main points, clothoid labels and the violation.
        shots.append(capture("as", focus=[state["as"]]))
        cx, cy = BEND[0] + ox, BEND[1] + oy
        shots.append(capture("bocht", [cx - 110, cy - 85, cx + 110, cy + 85]))
        marks = [h for l in layer_like("-AL-AANKLEDING-G") for h in ocs.handles(l)]
        if marks:
            shots.append(capture("toetsing", focus=marks[:3]))
        # The turnout on the track.
        if state.get("wissel", {}).get("wisselbegin"):
            wx, wy = state["wissel"]["wisselbegin"]
            wx, wy = wx + ox, wy + oy
            shots.append(capture("wissel", [wx - 25, wy - 30, wx + 85, wy + 30]))
        # The diagrams: one view each.
        for name, rid in [("lengteprofiel", "lp1"), ("zichtafstand", "zicht1"), ("verkanting", "sp1")]:
            f = api_dir / f"{rid}.json"
            if f.exists():
                d = json.loads(f.read_text()).get("data", {})
                h = d.get("lengteprofiel") or d.get("diagram")
                if h:
                    shots.append(capture(name, focus=[h]))
        return all(s[0] for s in shots), ", ".join(s[1] for s in shots)

    if step("site + BGT", site) and step("road with clothoids", road):
        step("offset + station", offset_and_station)
        step("terrain (TIN from AHN)", tin)
        step("length + design profile", profile)
        step("design criteria check", criteria)
        step("sight distance", sight)
    if step("rail with cant", rail):
        step("turnout 1:9", turnout)
    step("IFC 4.3 export", ifc)
    step("captures", captures)
    try:
        ocs.run(f"QSAVEAS {OUT / 'as-spoor.dwg'}")
        time.sleep(3)
    except Exception:  # noqa: BLE001
        pass
    ocs.stop()
    (OUT / "state.json").write_text(json.dumps(state, indent=1, default=str))
    lines = ["# Clothoids, criteria, rail and sight distance e2e", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {str(d).replace('|', '/')} | {t:.1f} |" for n, ok, d, t in results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if all(ok for _, ok, _, _ in results) else 1)


if __name__ == "__main__":
    main()
