#!/usr/bin/env python3
"""End-to-end test of plan production in Open CAD Studio: sheet sets along an
alignment (CIV_BLADEN, CIV_BLADEN_PDF) and labels that follow their source
(CIV_LABELS, CIV_LABELS_BIJWERKEN).

Runs its own Open CAD Studio with an isolated configuration folder
(``XDG_CONFIG_HOME`` under target/e2e-bladen), so the plugins installed for
daily use are left alone. A window appears, as with ``ocs_e2e.py``.

Steps
  model        Dordrecht: project location, the BGT around a 1 km ring road
               (alignment by PIs with two curves), its length profile (AHN)
               and a design profile with two vertical curves.
  sheets A1    1:1000: sheet count, layouts, match lines (model space, with
               their station), view frames, profile views, the JSON result.
  sheets A3    1:500: more sheets, the A1 set stays.
  pdf          every sheet plotted to PDF; rendered to PNG for a look.
  labels       stationing, curve data, PVIs and grades; a TIN from AHN with
               spot heights and slope arrows; survey points with labels.
  follow       the alignment moved and a survey point deleted:
               CIV_LABELS_BIJWERKEN redraws the axis labels at the new place
               and removes the point's labels; the profile labels stay.
  save         QSAVEAS target/e2e-bladen/bladen.dwg.

Usage
  python3 tests/e2e/bladen_e2e.py [--ocs PATH] [--port 18814] [--zonder-bgt]

The report goes to target/e2e-bladen/rapport.md, captures and PDFs beside it.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ocs_e2e  # noqa: E402

REPO = ocs_e2e.REPO
OUT = REPO / "target" / "e2e-bladen"

# A road of about 1 km through Dordrecht, south of the old centre (over
# land, so AHN gives a ground line), two curves (R 250 and R 200).
PIS = "104450,424980;104800,425120;105100,424990;105420,425100"
RADII = "250;200"


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


class Run:
    def __init__(self, ocs, xdg):
        self.ocs = ocs
        self.api = xdg / "OpenCADStudio" / "open-infra" / "api"
        self.results = []

    def civ(self, cmd, rid, timeout=600):
        """A CIV_ command with ID=; waits for its JSON result."""
        path = self.api / f"{rid}.json"
        if path.exists():
            path.unlink()
        self.ocs.run(f"{cmd} ID={rid}")
        t0 = time.time()
        while time.time() - t0 < timeout:
            if path.exists() and path.stat().st_size > 0:
                time.sleep(0.2)
                r = json.loads(path.read_text())
                if not r.get("ok"):
                    raise RuntimeError(f"{cmd.split()[0]}: {r.get('message')}")
                return r
            time.sleep(0.5)
        raise RuntimeError(f"{cmd.split()[0]}: no result after {timeout} s")

    def step(self, name, fn):
        t = time.time()
        try:
            ok, detail = fn()
        except Exception as e:  # noqa: BLE001 — the report wants everything
            ok, detail = False, f"{type(e).__name__}: {e}"
        self.results.append((name, ok, detail, time.time() - t))
        print(("✔" if ok else "✘"), name, "—", detail, flush=True)
        return ok

    def layer(self, part):
        return [l for l in self.ocs.layers() if part in l]

    def on(self, part, fields="handle,type,layer"):
        out = []
        for l in self.layer(part):
            r = self.ocs.get(f"/entities?layer={l}&fields={fields}&limit=100000")
            out.extend(r.get("entities", []))
        return out

    def capture(self, path, layout=None, bounds=None):
        if layout:
            self.ocs.run(f"-LAYOUT SET {layout}")
            time.sleep(1.0)
        body = {"path": str(path), "document_id": self.ocs.doc(), "max_dimension": 2400}
        if bounds:
            body.update({"view": "region", "bounds": bounds})
        else:
            body["view"] = "extents"
        self.ocs.call("/capture", body)
        t0 = time.time()
        while not (path.exists() and path.stat().st_size > 0) and time.time() - t0 < 60:
            time.sleep(0.25)
        if layout:
            self.ocs.run("-LAYOUT SET Model")
            time.sleep(0.5)
        return path.exists()


def position(e):
    """Insertion point of a text, start of a polyline."""
    for k in ("insertion_point", "position", "location", "alignment_point"):
        p = e.get(k) or e.get("properties", {}).get(k)
        if isinstance(p, dict):
            return [p.get("x"), p.get("y")]
        if isinstance(p, list):
            return p[:2]
    v = e.get("vertices") or e.get("properties", {}).get("vertices")
    if v:
        p = v[0].get("location", v[0]) if isinstance(v[0], dict) else v[0]
        return [p["x"], p["y"]] if isinstance(p, dict) else list(p[:2])
    return None


def xd(ocs, h):
    return {s.split("=", 1)[0]: s.split("=", 1)[1] for s in (str(x.get("value", x)) if isinstance(x, dict) else str(x) for x in ocs.xdata(h)) if "=" in s}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18814)
    ap.add_argument("--zonder-bgt", action="store_true", help="skip the BGT import (no PDOK)")
    ap.add_argument("--geen-installatie", action="store_true", help="use the plugins installed by an earlier run")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.geen_installatie:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    oi = xdg / "OpenCADStudio" / "open-infra"
    oi.mkdir(parents=True, exist_ok=True)
    (oi / "settings.json").write_text(json.dumps({"language": "nl"}))
    for old in (oi / "api").glob("*.json") if (oi / "api").exists() else []:
        old.unlink()
    pdf_dir = OUT / "pdf"
    shutil.rmtree(pdf_dir, ignore_errors=True)
    ocs_e2e.OUT = OUT
    ocs = ocs_e2e.Ocs(args.ocs, args.port)
    run = Run(ocs, xdg)
    state = {}
    try:
        def model():
            ocs.run("GIS_SITE RD 104885 425320 200 150 Dordrecht")
            time.sleep(2)
            # The project area around the whole road: AHN heights along it,
            # and the BGT for a plan that looks like one.
            ocs.run("GIS_SITE RD 104935 425050 1150 400 Dordrecht")
            time.sleep(2)
            if not args.zonder_bgt:
                ocs.run("GIS_Q_BGT")
                ocs.settle(minimum_seconds=3.0, timeout=600, first_change=240)
            a = run.civ(f"CIV_AS PI={PIS} R={RADII} NAAM=Rondweg SCHAAL=1000", "as")
            state["as"] = a["data"]["as"]
            state["len"] = a["data"]["lengte_m"]
            # Placed well away from the plan windows of the sheets.
            run.civ(f"CIV_LENGTEPROFIEL AS={state['as']} PLAATS=106200,425300 SCHAAL=1000", "lp", 900)
            n = state["len"]
            run.civ(f"CIV_ONTWERPPROFIEL AS={state['as']} PVI=0:1.0;300:2.6;650:1.4;{n:.2f}:2.2 BOOG=120 SCHAAL=1000", "op")
            return 900 < n < 1200, f"alignment {state['as']} of {n:.1f} m, entities {ocs.entities()}"

        def sheets(paper, scale, rid, expect):
            r = run.civ(f"CIV_BLADEN AS={state['as']} FORMAAT={paper} SCHAAL={scale} VSCHAAL=10", rid, 900)
            d = r["data"]
            state[rid] = d
            n, layouts, matches = d["bladen"], d["layouts"], d["aansluitlijnen"]
            ok = expect(n) and len(layouts) == n and len(matches) == n - 1
            # Match lines in model space carry their station.
            lines = [e for e in run.on("-AL-BLADVERWIJZING_AS-G") if xd(ocs, e["handle"]).get("bladen", "").endswith(paper)]
            stations = sorted(xd(ocs, e["handle"]).get("aansluitlijn") for e in lines)
            ok &= len(lines) == n - 1
            frames = [e for e in run.on("-AL-TEKENBLADGRENS-G") if xd(ocs, e["handle"]).get("bladen", "").endswith(paper)]
            ok &= len(frames) == n
            # The sheets follow each other along the axis.
            parts = d["bladindeling"]
            ok &= abs(parts[0]["van"]) < 1e-6 and abs(parts[-1]["tot"] - state["len"]) < 0.01
            ok &= all(abs(a["tot"] - b["van"]) < 1e-6 for a, b in zip(parts, parts[1:]))
            return ok, f"{n} sheets {layouts}, match lines {stations}, vertical 1:{round(float(scale.split(':')[-1]) / d['vschaal'])}; {r['message'][:160]}"

        def captures():
            done = []
            for key in ("b1", "b2"):
                for i, layout in enumerate(state[key]["layouts"][:2]):
                    p = OUT / f"{layout}.png"
                    if run.capture(p, layout=layout):
                        done.append(p.name)
            p = OUT / "model.png"
            run.capture(p)
            done.append(p.name)
            return len(done) >= 3, ", ".join(done)

        def pdf():
            r = run.civ(f"CIV_BLADEN_PDF AS={state['as']} MAP={pdf_dir}", "pdf", 900)
            files = r["data"]["pdf"]
            want = state["b1"]["bladen"] + state["b2"]["bladen"]
            pngs = []
            for f in files[:3]:
                stem = OUT / ("pdf-" + Path(f).stem)
                subprocess.run(["pdftoppm", "-r", "60", "-png", "-singlefile", f, str(stem)], check=False)
                pngs.append(stem.name + ".png")
            sizes = [Path(f).stat().st_size for f in files]
            return len(files) == want and all(s > 2000 for s in sizes), f"{len(files)} PDFs ({min(sizes)}–{max(sizes)} bytes); rendered {pngs}"

        def labels():
            r1 = run.civ(f"CIV_LABELS AS={state['as']} SOORT=stationering;bogen;pvi;hellingen SCHAAL=1000", "l1")
            ocs.run("GIS_SITE RD 104885 425320 200 150 Dordrecht")
            time.sleep(2)
            run.civ("CIV_TIN BRON=AHN AFSTAND=5", "tin", 600)
            r2 = run.civ("CIV_LABELS SOORT=hoogtes RASTER=25 SCHAAL=500", "l2")
            r3 = run.civ("CIV_LABELS SOORT=hoogtes PUNTEN=104860,425300;104900,425340 SCHAAL=500", "l3")
            r4 = run.civ("CIV_LABELS SOORT=afschot RASTER=20 SCHAAL=500", "l4")
            run.civ("CIV_PUNT 104850,425290 104920,425350 NR=101 CODE=PUT LABELS=nee", "pt")
            r5 = run.civ("CIV_LABELS SOORT=punten SCHAAL=500", "l5")
            kinds = r1["data"]["soorten"]
            ok = all(kinds.get(k, 0) > 0 for k in ("stationering", "bogen", "pvi", "hellingen"))
            ok &= r2["data"]["labels"] > 10 and r3["data"]["labels"] in (3, 6) and r4["data"]["labels"] > 0 and r5["data"]["labels"] >= 4
            style = oi / "labels.json"
            ok &= style.exists()
            return ok, f"axis/profile {kinds}; spot grid {r2['data']['labels']}, picked {r3['data']['labels']}, slope arrows {r4['data']['labels']}, points {r5['data']['labels']}; style file {style.exists()}"

        def follow():
            # A stationing label of the axis, before and after the move.
            def station_labels():
                out = {}
                for e in run.on("-AM-KILOMETRERING-T", "handle,type,layer,insertion_point,alignment_point,value"):
                    x = xd(ocs, e["handle"])
                    if x.get("label") == "stationering":
                        full = ocs.entity(e["handle"])
                        out[full.get("value", full.get("text", e["handle"]))] = position(full)
                return out
            before = station_labels()
            pvi_before = len([e for e in run.on("-AM-AS-T") if xd(ocs, e["handle"]).get("label") == "pvi"])
            ocs.call("/entities/transform", {"handles": [state["as"]], "action": "move", "vector": [10.0, 5.0, 0.0], "document_id": ocs.doc()})
            pts = [e for e in run.on("-OG-HOOGTEPUNT-S") if e.get("type", "").lower() == "point"]
            gone = pts[0]["handle"] if pts else None
            if gone:
                ocs.call("/entities", {"handles": [gone], "document_id": ocs.doc()}, method="DELETE")
            time.sleep(1)
            r = run.civ("CIV_LABELS_BIJWERKEN SCHAAL=1000", "lb")
            after = station_labels()
            moved = [k for k in before if k in after and before[k] and after[k] and abs(after[k][0] - before[k][0] - 10.0) < 1e-3 and abs(after[k][1] - before[k][1] - 5.0) < 1e-3]
            pvi_after = len([e for e in run.on("-AM-AS-T") if xd(ocs, e["handle"]).get("label") == "pvi"])
            d = r["data"]
            ok = d["bijgewerkt"] >= 2 and d["verwijderd"] >= 1 and len(moved) == len(before) > 0 and pvi_after == pvi_before
            # Nothing changed since: a second run touches nothing.
            r2 = run.civ("CIV_LABELS_BIJWERKEN SCHAAL=1000", "lb2")
            ok &= r2["data"]["bijgewerkt"] == 0
            return ok, f"{d}; {len(moved)}/{len(before)} station labels moved by (10, 5); PVI labels {pvi_before}→{pvi_after}; again: {r2['data']}"

        def ifc():
            # The sheets stay out of the IFC export; labels are annotation,
            # and without texts they go as a whole.
            r1 = run.civ(f"CIV_IFC BESTAND={OUT / 'bladen.ifc'}", "ifc1", 900)
            r2 = run.civ(f"CIV_IFC BESTAND={OUT / 'bladen-zonder-tekst.ifc'} TEKST=nee", "ifc2", 900)
            s1, s2 = r1["data"]["overgeslagen"], r2["data"]["overgeslagen"]
            c1, c2 = r1["data"]["klassen"], r2["data"]["klassen"]
            sheet_parts = s1.get("tekenblad en profielkader", 0)
            ok = sheet_parts > 0 and c1.get("IfcAlignment", 0) >= 1 and c2.get("IfcAnnotation", 0) < c1.get("IfcAnnotation", 0)
            # Nothing at sheet coordinates: every annotation lies in the project.
            return ok, f"sheet parts left out {sheet_parts}; annotations {c1.get('IfcAnnotation', 0)} → {c2.get('IfcAnnotation', 0)} without texts; skipped {s1}"

        def save():
            # QSAVE into start.dwg, then a copy (the REST /save of v2026.43
            # leaves out the document id).
            start = OUT / "start.dwg"
            before = start.stat().st_mtime
            ocs.run("QSAVE")
            t0 = time.time()
            while start.stat().st_mtime == before and time.time() - t0 < 120:
                time.sleep(0.5)
            time.sleep(3)
            path = OUT / "bladen.dwg"
            shutil.copy(start, path)
            lossy = None
            # Open a copy: the layouts and the label index come back.
            copy = OUT / "bladen-heropend.dwg"
            shutil.copy(path, copy)
            ocs.open(copy)
            time.sleep(3)
            recs = ocs.get("/records?collection=objects&limit=100000").get("records", [])
            layouts = sorted({r.get("properties", {}).get("name") for r in recs if r.get("type") == "Layout"} - {None, "Model"})
            want = state["b1"]["layouts"] + state["b2"]["layouts"]
            ok = all(l in layouts for l in want)
            # The labels' object data and index came along: nothing to redo.
            r = run.civ("CIV_LABELS_BIJWERKEN SCHAAL=1000", "lb3")
            ok &= r["data"]["bijgewerkt"] == 0 and r["data"]["ongewijzigd"] > 0
            note = f"; lossy: {lossy}" if lossy else ""
            return ok, f"{path.name} {path.stat().st_size // 1024} kB; reopened: {len(layouts)} layouts ({sum(l in layouts for l in want)}/{len(want)} sheets), labels {r['data']}{note}"

        if run.step("model", model):
            a1 = run.step("sheets A1 1:1000", lambda: sheets("A1", "1:1000", "b1", lambda n: n == 2))
            a3 = run.step("sheets A3 1:500", lambda: sheets("A3", "1:500", "b2", lambda n: n >= 6))
            if a1 and a3:
                run.step("captures", captures)
                run.step("pdf", pdf)
            if run.step("labels", labels):
                run.step("follow", follow)
            run.step("ifc", ifc)
            run.step("save", save)
    finally:
        ocs.stop()
    lines = ["# Plan production e2e (sheets and labels)", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {str(d).replace('|', '/')} | {t:.1f} |" for n, ok, d, t in run.results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if run.results and all(ok for _, ok, _, _ in run.results) else 1)


if __name__ == "__main__":
    main()
