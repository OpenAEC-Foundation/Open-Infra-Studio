#!/usr/bin/env python3
"""End-to-end test of the sewer design tools (CIV_RIOOL*) in Open CAD Studio.

Runs its own Open CAD Studio with an isolated configuration folder
(``XDG_CONFIG_HOME``), so the plugin installed for daily use is left alone.
A window appears, as with ``ocs_e2e.py``.

The drawing is a copy of ``examples/dordrecht-nlcs`` (BGT, cadastre and a
KLIC delivery around the Grote Kerk in Dordrecht), with the site set to
``GIS_SITE RD 104885 425320 200 150 Dordrecht`` and AHN as ground level.

Steps
  ontwerp      CIV_RIOOL: a DWA run of 6 manholes, PVC 250, cover 0,80 m,
               1:250; BOB, cover and slope checked, NLCS RI layers and XDATA.
  profiel      CIV_RIOOL_PROFIEL: the length profile, frame and pipes.
  tweede riool CIV_RIOOL STELSEL=HWA crossing the first run.
  toets        CIV_RIOOL_TOETS Q=…: KLIC crossings (depth unknown in the
               KLIC sample → warnings; a 3D test line on a KLIC layer with a
               known level → an error), the crossing with the HWA run,
               capacity per pipe, markers and the CSV report.
  bijwerken    a manhole moved in the drawing, then CIV_RIOOL_BIJWERKEN; a
               pipe made larger by argument.
  export       CIV_RIOOL_EXPORT FORMAAT=GWSW and FORMAAT=CSV.
  terug        a new drawing: CIV_RIOOL_IMPORT reads the GWSW-OroX back by
               the GWSW import's route; CIV_RIOOL_GWSW takes it over as an
               editable network; a new run joins one of its manholes.

Usage
  python3 tests/e2e/riolering_e2e.py [--ocs PATH] [--port 18812] [--no-install]

The report and captures go to target/e2e-riolering/.
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
OUT = REPO / "target" / "e2e-riolering"
SITE = "GIS_SITE RD 104885 425320 200 150 Dordrecht"
# Along the eastern and southern streets of the KLIC sample, upstream first.
RUN = [(104960.0, 425345.0), (104956.0, 425312.0), (104950.0, 425286.0), (104910.0, 425284.0), (104875.0, 425285.0), (104850.0, 425286.0)]
# A storm water pipe across the street, crossing the run between P104 and P105.
HWA = [(104880.0, 425292.0), (104900.0, 425279.0)]


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


def pts(points):
    return ";".join(f"{x},{y}" for x, y in points)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18812)
    ap.add_argument("--no-install", action="store_true")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.no_install:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    cfg = xdg / "OpenCADStudio" / "open-infra"
    cfg.mkdir(parents=True, exist_ok=True)
    # Dutch interface, the Dutch modules on.
    (cfg / "settings.json").write_text(json.dumps({"language": "nl", "netherlands": True}))
    api = cfg / "api"
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

    def run_id(cmd, rid, timeout=300):
        """Run a command with ID= and wait for its result file."""
        f = api / f"{rid}.json"
        if f.exists():
            f.unlink()
        ocs.run(f"{cmd} ID={rid}")
        t0 = time.time()
        while not f.exists() and time.time() - t0 < timeout:
            time.sleep(0.5)
        if not f.exists():
            raise RuntimeError(f"{rid}: geen resultaat binnen {timeout} s")
        time.sleep(0.3)
        r = json.loads(f.read_text())
        time.sleep(1.0)
        return r

    def xd(handle):
        return dict(v.split("=", 1) for v in ocs.xdata(handle) if "=" in v)

    def capture(name, bounds):
        path = OUT / f"{name}.png"
        if path.exists():
            path.unlink()
        ocs.call("/capture", {"path": str(path), "document_id": ocs.doc(), "view": "region", "bounds": bounds, "max_dimension": 1600})
        t0 = time.time()
        while not (path.exists() and path.stat().st_size > 0) and time.time() - t0 < 60:
            time.sleep(0.25)
        return path.exists()

    dwg = OUT / "riolering.dwg"
    shutil.copy(REPO / "examples" / "dordrecht-nlcs" / "dordrecht-grote-kerk-nlcs.dwg", dwg)
    ocs.open(dwg)

    def site():
        ocs.run(SITE)
        time.sleep(2)
        klic = [l for l in ocs.layers() if "-KL-" in l]
        return len(klic) > 5, f"projectlocatie gezet; {len(klic)} KLIC-lagen in de tekening"

    step("projectlocatie Dordrecht + KLIC-voorbeeld", site)

    def ontwerp():
        r = run_id(f"CIV_RIOOL PUNTEN={pts(RUN)} STELSEL=DWA DIAMETER=250 MATERIAAL=PVC DEKKING=0.8 VERHANG=1:250 PUT=800 PUTNR=P101 NAAM=DWA1", "riool_ontwerp")
        state["ontwerp"] = r
        if not r["ok"]:
            return False, r["message"]
        d = r["data"]
        putten, leidingen = d["putten"], d["leidingen"]
        problems = []
        if len(putten) != 6 or len(leidingen) != 5:
            problems.append(f"{len(putten)} putten / {len(leidingen)} leidingen")
        if any(p["maaiveld"] is None for p in putten):
            problems.append("put zonder maaiveld")
        for l in leidingen:
            if l["dekking_min"] is None or l["dekking_min"] < 0.8 - 0.006:
                problems.append(f"{l['naam']} dekking {l['dekking_min']}")
            if l["verhang"] < 1 / 250 - 1e-6:
                problems.append(f"{l['naam']} verhang {l['verhang']}")
        for a, b in zip(leidingen, leidingen[1:]):
            if b["bob_van"] > a["bob_naar"] + 1e-6:
                problems.append(f"BOB stijgt in put {a['naar']}")
        lay = ocs.layers()
        for want in ["N-XX-RI-DWA_RIOOLLEIDING_PVC_250-G", "N-XX-RI-DWA_RIOOLPUT_INSPECTIEPUT-G", "N-XX-RI-DWA_PUTNUMMER-T", "N-XX-RI-DWA_BOB-T", "N-XX-RI-DWA_PUTDEKSELNIVEAU-T", "N-XX-RI-DWA_STROMINGSRICHTING-G"]:
            if want not in lay:
                problems.append(f"laag {want} ontbreekt")
        x = xd(leidingen[0]["handle"])
        if x.get("riool") != "DWA1" or x.get("van") != "P101" or "bob_van" not in x:
            problems.append(f"XDATA leiding: {x}")
        x = xd(putten[0]["handle"])
        if x.get("put") != "P101" or "maaiveld" not in x:
            problems.append(f"XDATA put: {x}")
        state["bob"] = [(l["bob_van"], l["bob_naar"]) for l in leidingen]
        summary = ", ".join(f"{l['naam']} {l['bob_van']:+.2f}→{l['bob_naar']:+.2f} (dekking ≥ {l['dekking_min'] or 0:.2f}, 1:{1 / l['verhang']:.0f}, laagste maaiveld {l['maaiveld_laagst'] or 0:+.2f} op {l['maaiveld_laagst_op'] or 0:.0f} m)" for l in leidingen)
        return not problems, ("; ".join(problems) + " | " if problems else "") + f"maaiveld {d['maaiveld']}; " + summary

    step("ontwerp DWA-riool (6 putten)", ontwerp)

    def profiel():
        # Below the example drawing (300 × 250 m around the site), so the
        # capture shows the profile alone.
        r = run_id("CIV_RIOOL_PROFIEL RIOOL=DWA1 PLAATS=104780,424950", "riool_profiel")
        state["profiel"] = r
        if not r["ok"]:
            return False, r["message"]
        d = r["data"]
        frame = ocs.entity(d["kader"])
        x = xd(d["kader"])
        ok = d["putten"][0] == "P101" and d["putten"][-1] == "P106" and x.get("rioolprofiel") == "DWA1" and frame.get("layer", "").endswith("AL-PROFIELBALK-G")
        return ok, f"{r['message']} kader {frame.get('layer')}"

    step("lengteprofiel", profiel)

    def tweede():
        r = run_id(f"CIV_RIOOL PUNTEN={pts(HWA)} STELSEL=HWA DIAMETER=315 PUTNR=H1 NAAM=HWA1", "riool_hwa")
        layers = ocs.layers()
        return r["ok"] and "N-XX-RI-HWA_RIOOLLEIDING_PVC_315-G" in layers, r["message"]

    step("tweede riool (HWA) dat het eerste kruist", tweede)

    def testlijn():
        # A cable with a known level on a KLIC layer, across P102–P103, at
        # the level of the pipe: a clash.
        l = state["ontwerp"]["data"]["leidingen"][1]
        z = (l["bob_van"] + l["bob_naar"]) / 2 + 0.1
        before = ocs.entities()
        ocs.run(f"LINE 104945,425300,{z:.3f} 104962,425301,{z:.3f} ")
        time.sleep(1.5)
        hs = ocs.handles("0")
        if not hs or ocs.entities() <= before:
            return False, "LINE niet getekend"
        h = hs[-1]
        r = ocs.call("/set_properties", {"collection": "entities", "handle": h, "updates": [{"path": "/common/layer", "value": "B-OI-KL-GAS_LD-G"}], "document_id": ocs.doc()})
        e = ocs.entity(h)
        state["testlijn"] = h
        return e.get("layer") == "B-OI-KL-GAS_LD-G", f"testkabel {h} op {e.get('layer')} ({r.get('error') or 'ok'})"

    step("testkabel met bekende hoogte op een KLIC-laag", testlijn)

    def toets():
        r = run_id("CIV_RIOOL_TOETS RIOOL=DWA1 Q=12", "riool_toets")
        state["toets"] = r
        if not r["ok"]:
            return False, r["message"]
        d = r["data"]
        b = d["bevindingen"]
        kinds = {}
        for f in b:
            kinds.setdefault((f["soort"], f["ernst"]), 0)
            kinds[(f["soort"], f["ernst"])] += 1
        klic_warn = kinds.get(("klic", "waarschuwing"), 0)
        klic_err = kinds.get(("klic", "fout"), 0)
        cross = kinds.get(("kruising", "fout"), 0)
        markers = len(ocs.handles("OI-RIOOLTOETS")) + len(ocs.handles("OI-RIOOLTOETS-LET-OP"))
        hyd = d["leidingen"]
        csv = Path(d["csv"])
        ok = klic_warn > 0 and klic_err > 0 and cross > 0 and markers > 0 and len(hyd) == 5 and all(h["capaciteit_vol"] > 0 for h in hyd) and csv.exists()
        cap = ", ".join(f"{h['leiding']} {h['capaciteit_vol']:.0f} l/s v {h['snelheid_vol']:.2f}" for h in hyd[:2])
        return ok, f"{r['message'][:160]} | {kinds} | {markers} markeringen | {cap}"

    step("toets: dekking, verhang, capaciteit, KLIC en kruising", toets)

    def bijwerken():
        p = state["ontwerp"]["data"]["putten"][2]
        ocs.call("/entities/transform", {"handles": [p["handle"]], "action": "move", "vector": [3.0, 2.0, 0.0], "document_id": ocs.doc()})
        time.sleep(1)
        r = run_id("CIV_RIOOL_BIJWERKEN RIOOL=DWA1", "riool_bijwerken")
        if not r["ok"]:
            return False, r["message"]
        d = r["data"]
        p3 = next(q for q in d["putten"] if q["naam"] == "P103")
        moved = abs(p3["x"] - (p["x"] + 3.0)) < 0.01 and abs(p3["y"] - (p["y"] + 2.0)) < 0.01
        x = xd(p3["handle"])
        changed = any("P103" in c for c in d["wijzigingen"])
        r2 = run_id("CIV_RIOOL_BIJWERKEN RIOOL=DWA1 LEIDING=P104-P105 DIAMETER=315", "riool_bijwerken2")
        l = next(q for q in r2["data"]["leidingen"] if q["naam"] == "P104-P105")
        e = ocs.entity(l["handle"])
        bigger = l["diameter"] == 315 and e.get("layer") == "N-XX-RI-DWA_RIOOLLEIDING_PVC_315-G"
        stale = len(ocs.handles("OI-RIOOLTOETS")) == 0
        return moved and changed and bigger and abs(float(x.get("x", 0)) - p3["x"]) < 0.01, f"{r['message'][:200]} | {r2['message'][:120]} | P104-P105 op {e.get('layer')}; oude markeringen weg: {stale}"

    step("bijwerken: put verplaatst, leiding groter", bijwerken)

    def export():
        ttl = OUT / "riool_dwa1.orox.ttl"
        r = run_id(f"CIV_RIOOL_EXPORT RIOOL=DWA1 FORMAAT=GWSW PAD={ttl}", "riool_export")
        r2 = run_id(f"CIV_RIOOL_EXPORT RIOOL=DWA1 FORMAAT=CSV PAD={OUT / 'riool_dwa1.csv'}", "riool_csv")
        text = ttl.read_text() if ttl.exists() else ""
        puts = (OUT / "riool_dwa1_putten.csv").read_text().splitlines() if (OUT / "riool_dwa1_putten.csv").exists() else []
        ok = r["ok"] and r2["ok"] and "gwsw:Vuilwaterriool" in text and "gwsw:BobBeginpuntLeiding" in text and len(puts) == 7
        state["export"] = r
        return ok, f"{r['message']} ({len(text)} tekens); {r2['message']}"

    step("export GWSW-OroX en CSV", export)

    def ifc():
        path = OUT / "riolering.ifc"
        if path.exists():
            path.unlink()
        r = run_id(f"CIV_IFC BESTAND={path}", "riool_ifc", timeout=600)
        text = path.read_text(errors="replace") if path.exists() else ""
        pipes = [l for l in text.splitlines() if "IFCPIPESEGMENT(" in l and "'P10" in l]
        chambers = [l for l in text.splitlines() if "IFCDISTRIBUTIONCHAMBERELEMENT(" in l and "'P10" in l]
        ok = r["ok"] and len(pipes) == 5 and len(chambers) == 6 and "IFCSWEPTDISKSOLID" in text and "'InvertLevelStart'" in text and "'OpenInfra_Sewer'" in text
        return ok, f"{len(pipes)} IfcPipeSegment, {len(chambers)} IfcDistributionChamberElement van DWA1; {r['message'][:200]}"

    step("IFC-export: leidingen en putten met BOB", ifc)

    # The pictures: plan and profile.
    xs = [x for x, _ in RUN] + [x for x, _ in HWA]
    ys = [y for _, y in RUN] + [y for _, y in HWA]
    step("afbeelding plattegrond", lambda: (capture("riolering-plan", [min(xs) - 15, min(ys) - 15, max(xs) + 15, max(ys) + 15]), "riolering-plan.png"))
    step("afbeelding detail P102-P104", lambda: (capture("riolering-detail", [104900, 425270, 104975, 425320]), "riolering-detail.png"))
    place = state.get("profiel", {}).get("data", {}).get("plaats")
    if place:
        length = state["profiel"]["data"]["lengte"]
        step("afbeelding lengteprofiel", lambda: (capture("riolering-profiel", [place[0] - 25, place[1] - 5, place[0] + length + 5, place[1] + 70]), "riolering-profiel.png"))
        step("afbeelding lengteprofiel detail", lambda: (capture("riolering-profiel-detail", [place[0] - 25, place[1] - 2, place[0] + 60, place[1] + 60]), "riolering-profiel-detail.png"))
    try:
        ocs.call("/save", {"path": str(dwg), "document_id": ocs.doc()}, timeout=300)
    except Exception:  # noqa: BLE001
        ocs.run("QSAVE")

    # Reading the export back, and an existing network made editable.
    dwg2 = OUT / "riolering-gwsw.dwg"
    shutil.copy(REPO / "tests" / "data" / "leeg-m.dwg", dwg2)
    ocs.open(dwg2)
    ocs.run(SITE)
    time.sleep(2)

    def terug():
        ttl = OUT / "riool_dwa1.orox.ttl"
        r = run_id(f"CIV_RIOOL_IMPORT {ttl}", "riool_import")
        if not r["ok"]:
            return False, r["message"]
        pipes = ocs.handles("B-XX-RI-DWA_RIOOLLEIDING_PVC_250-G") + ocs.handles("B-XX-RI-DWA_RIOOLLEIDING_PVC_315-G")
        bobs = {}
        for h in pipes:
            x = xd(h)
            bobs[x.get("naam")] = (float(x.get("bob_beginpunt_leiding", "nan")), float(x.get("bob_eindpunt_leiding", "nan")))
        first = bobs.get("P101-P102")
        want = state["ontwerp"]["data"]["leidingen"][0]
        same = first is not None and abs(first[0] - want["bob_van"]) < 0.002
        return len(pipes) == 5 and same, f"{r['message']} | P101-P102 BOB {first} (ontwerp {want['bob_van']:+.3f})"

    step("GWSW-OroX teruglezen (GWSW-import)", terug)

    def overnemen():
        r = run_id("CIV_RIOOL_GWSW NAAM=Bestaand", "riool_gwsw")
        if not r["ok"]:
            return False, r["message"]
        left = ocs.handles("B-XX-RI-DWA_RIOOLLEIDING_PVC_250-G")
        data = [xd(h) for h in left]
        ok = all(x.get("riool") == "Bestaand" and x.get("bestaand") == "ja" for x in data) and len(data) >= 3
        return ok, f"{r['message'][:220]} | {len(data)} leidingen op de B-laag, allemaal riool=Bestaand/bestaand=ja: {ok}"

    step("GWSW overnemen als bewerkbaar riool", overnemen)

    def uitbreiden():
        p103 = next(p for p in state["ontwerp"]["data"]["putten"] if p["naam"] == "P103")
        # The moved P103 is where the export put it.
        x, y = p103["x"] + 3.0, p103["y"] + 2.0
        # From the east along the street, into P103.
        r = run_id(f"CIV_RIOOL PUNTEN={x + 40},{y - 3};{x},{y} STELSEL=DWA DIAMETER=250", "riool_uitbreiding")
        if not r["ok"]:
            return False, r["message"]
        d = r["data"]
        new = [l for l in d["leidingen"] if not l["bestaand"]]
        old = [l for l in d["leidingen"] if l["bestaand"]]
        ok = d["aangesloten"] and d["riool"] == "Bestaand" and len(new) == 1 and new[0]["naar"] == "P103" and len(old) == 5
        return ok, f"{r['message'][:240]}"

    step("nieuw riool sluit aan op het bestaande", uitbreiden)
    step("afbeelding overgenomen + uitbreiding", lambda: (capture("riolering-gwsw", [min(xs) - 15, min(ys) - 15, max(xs) + 15, max(ys) + 60]), "riolering-gwsw.png"))
    try:
        ocs.call("/save", {"path": str(dwg2), "document_id": ocs.doc()}, timeout=300)
    except Exception:  # noqa: BLE001
        pass
    ocs.stop()
    lines = ["# Riolering e2e", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {str(d).replace('|', '/')} | {t:.1f} |" for n, ok, d, t in results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if all(ok for _, ok, _, _ in results) else 1)


if __name__ == "__main__":
    main()
