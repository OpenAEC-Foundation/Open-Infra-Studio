#!/usr/bin/env python3
"""End-to-end test of corridors with the parts library, JSON assemblies,
junctions and roundabouts, in a real Open CAD Studio.

Runs its own Open CAD Studio with an isolated configuration folder
(``XDG_CONFIG_HOME``), so the plugin installed for daily use is left alone.
A window appears, as with ``ocs_e2e.py``.

The site is open farmland in the Dordrecht polders (AHN, about +0,3 m NAP):

  town street   ``CIV_CORRIDOR PROFIELTYPE=stadsstraat`` on a 340 m axis: lanes,
                gutters, kerbs, a footway, a separating verge, a cycle path
                and a ditch, with pavement layers. Checks the feature lines
                and layers, the formation and subgrade surfaces and the
                material quantities (asphalt, granulate, kerb length) against
                hand-computed values, and the CSV files.
  assembly      a user JSON assembly with a cut/fill condition (in cut a
                ditch, in fill a wider shoulder) in the isolated config
                folder: ``CIV_PROFIELTYPEN`` lists it, ``CIV_PROFIELTYPE``
                draws it, and a corridor whose profile runs from cut into
                fill must switch parts.
  T junction    ``CIV_KRUISPUNT`` of the town street and the side road: kerb
                returns, the TIN patch, then the side road's corridor from
                the mouth on (``TOT=``).
  roundabout    ``CIV_ROTONDE`` (CROW, built-up area) on two crossing axes:
                four arms, and the arms' corridors from their mouths.
  captures      plan and 3D views in target/e2e-corridor/*.png.

Usage
  python3 tests/e2e/corridor_e2e.py [--ocs PATH] [--port 18816] [--no-install]

The report goes to target/e2e-corridor/rapport.md.
"""

import argparse
import csv
import json
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ocs_e2e  # noqa: E402

REPO = ocs_e2e.REPO
OUT = REPO / "target" / "e2e-corridor"

# Open fields in the Dordrecht polders, about +0,3 m NAP.
SITE = "GIS_SITE RD 108400 420450 400 320 Dordrecht Zuidpolder"

CONDITIONAL = {
    "naam": "polder-e2e",
    "omschrijving": "Test: polderweg; in ingraving een sloot, in ophoging een bredere berm",
    "bronnen": ["e2e-test"],
    "talud": 3, "ingraving": 2, "reikwijdte": 30,
    "beide": [
        {"type": "rijstrook", "breedte": 2.75, "helling": -2.5, "lagen": "asfalt-etw"},
        {"type": "berm", "breedte": 1.20, "helling": -4.0},
        {"type": "sloot", "als": "ingraving", "bodembreedte": 0.6, "diepte": 0.7, "talud_binnen": 1.5, "talud_buiten": 2.0},
        {"type": "berm", "naam": "berm ophoging", "als": "ophoging", "breedte": 1.5, "helling": -4.0},
    ],
}


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18816)
    ap.add_argument("--no-install", action="store_true", help="use the plugins already in the isolated folder")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.no_install:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    cfg = xdg / "OpenCADStudio" / "open-infra"
    (cfg / "assemblies").mkdir(parents=True, exist_ok=True)
    (cfg / "settings.json").write_text(json.dumps({"language": "nl"}))
    (cfg / "assemblies" / "polder-e2e.json").write_text(json.dumps(CONDITIONAL, indent=1, ensure_ascii=False))
    api_dir = cfg / "api"
    ocs_e2e.OUT = OUT
    ocs = ocs_e2e.Ocs(args.ocs, args.port)
    results = []
    state = {}

    def step(name, fn):
        t = time.time()
        try:
            ok, detail = fn()
        except Exception as e:  # noqa: BLE001 — the report wants everything
            ok, detail = False, f"{type(e).__name__}: {e}"
        results.append((name, ok, detail, time.time() - t))
        print(("✔" if ok else "✘"), name, "—", detail, flush=True)
        return ok

    def cmd(c, rid, timeout=600):
        """An Open Infra command with ID=; waits for its JSON answer."""
        path = api_dir / f"{rid}.json"
        if path.exists():
            path.unlink()
        ocs.run(f"{c} ID={rid}")
        t0 = time.time()
        while time.time() - t0 < timeout:
            if path.exists():
                try:
                    return json.loads(path.read_text())
                except json.JSONDecodeError:
                    pass
            time.sleep(0.5)
        raise TimeoutError(f"{c.split()[0]}: no answer in {timeout} s")

    def layer(part):
        """The design layer (status N) of an NLCS object."""
        return next((l for l in ocs.layers() if l.startswith("N-") and part in l), None)

    def capture(name, bounds=None, size=1600):
        body = {"path": str(OUT / name), "document_id": ocs.doc(), "max_dimension": size}
        if bounds:
            body.update({"view": "region", "bounds": bounds})
        p = OUT / name
        if p.exists():
            p.unlink()
        ocs.call("/capture", body, timeout=180)
        t0 = time.time()
        while not (p.exists() and p.stat().st_size > 0) and time.time() - t0 < 60:
            time.sleep(0.25)
        time.sleep(0.5)
        return p.exists()

    def site():
        r = cmd(SITE, "site", 120)
        return r["ok"], r["message"][:120]

    def terrain():
        r = cmd("CIV_TIN BRON=AHN AFSTAND=2", "tin", 600)
        return r["ok"], r["message"][:160]

    def axes():
        made = {}
        specs = {
            "a": ("PI=108230,420555;108400,420565;108570,420555 R=300 NAAM=Hoofdstraat", [(0, 0.75), (170, 0.85), (None, 0.75)]),
            "b": ("PI=108320,420440;108320,420560 NAAM=Zijweg", [(0, -0.20), (60, 0.0), (None, 0.80)]),
            "c": ("PI=108380,420400;108570,420400 NAAM=Rotonde-WO", [(0, 0.70), (None, 0.70)]),
            "d": ("PI=108470,420310;108470,420490 NAAM=Rotonde-ZN", [(0, 0.70), (None, 0.70)]),
        }
        for key, (pi, pvis) in specs.items():
            r = cmd(f"CIV_AS {pi}", f"as_{key}")
            if not r["ok"]:
                return False, f"axis {key}: {r['message']}"
            h, length = r["data"]["as"], r["data"]["lengte_m"]
            made[key] = (h, length)
            r = cmd(f"CIV_LENGTEPROFIEL AS={h}", f"lp_{key}", 300)
            if not r["ok"]:
                return False, f"length profile {key}: {r['message']}"
            pv = ";".join(f"{length if s is None else s:.2f}:{z}" for s, z in pvis)
            r = cmd(f"CIV_ONTWERPPROFIEL AS={h} PVI={pv} BOOG=60", f"op_{key}")
            if not r["ok"]:
                return False, f"design profile {key}: {r['message']}"
        state["axes"] = made
        return True, ", ".join(f"{k}={h} ({l:.1f} m)" for k, (h, l) in made.items())

    def street():
        h, length = state["axes"]["a"]
        r = cmd(f"CIV_CORRIDOR AS={h} PROFIELTYPE=stadsstraat FREQUENTIE=10 DWARSPROFIELEN=ja", "cor_a", 900)
        if not r["ok"]:
            return False, r["message"]
        d = r["data"]
        state["street"] = d
        mats = {m["materiaal"]: m["volume_m3"] for m in d["materialen"]}
        kerbs = {k["naam"]: k["lengte_m"] for k in d["banden"]}
        checks = []
        # Asphalt surface course: two lanes 3,00 m × 4 cm over the length.
        surf = mats.get("AC 11 surf (deklaag)", 0)
        checks.append(("AC 11 surf", abs(surf - 2 * 3.0 * 0.04 * length) / (2 * 3.0 * 0.04 * length) < 0.03, f"{surf:.1f} m³"))
        # Granulate under lanes (3,00 m) and gutters (0,30 m), 25 cm, both
        # sides, and under the cycle path (2,00 m), 20 cm.
        gran = mats.get("menggranulaat (fundering)", 0)
        want = (2 * (3.0 + 0.3) * 0.25 + 2.0 * 0.20) * length
        checks.append(("menggranulaat", abs(gran - want) / want < 0.03, f"{gran:.1f} m³ (verwacht {want:.1f})"))
        # Red asphalt of the cycle path: 2,00 m × 3 cm on one side.
        red = mats.get("AC 8 surf rood (deklaag)", 0)
        checks.append(("rood asfalt", abs(red - 2.0 * 0.03 * length) / (2.0 * 0.03 * length) < 0.03, f"{red:.1f} m³"))
        kl = kerbs.get("trottoirband 13/15x25", 0)
        checks.append(("trottoirband", abs(kl - 2 * length) / (2 * length) < 0.02, f"{kl:.1f} m"))
        codes = " ".join(p["codes"] for p in d["profielen"])
        checks.append(("codes", all(c in codes for c in ["RS", "GO", "BV", "BA", "TR", "SB", "FP", "SL"]), "punten RS GO BV BA TR SB FP SL"))
        lay = [layer(x) for x in ["-VH-KANTOPSLUITING_TROTTOIRBAND-G", "-GW-WATERGANG_SLOOT_BODEM-G", "-GW-CUNET_BODEM-G", "-VH-FUNDERING-G", "-VH-GESLOTENVERHARDING_ASFALT_ROOD-G", "-VH-OPENVERHARDING_TEGEL-G"]]
        checks.append(("lagen", all(lay), ", ".join(str(x) for x in lay)))
        checks.append(("onderkant en formatie", d["driehoeken_onderkant"] > 100 and d["driehoeken_formatie"] > 100, f"{d['driehoeken_onderkant']} / {d['driehoeken_formatie']} driehoeken"))
        mcsv = Path(d.get("materialen_csv", ""))
        rows = list(csv.reader(mcsv.open(), delimiter=";")) if mcsv.exists() else []
        checks.append(("materialen.csv", len(rows) > 8, f"{len(rows)} regels in {mcsv.name}"))
        xd = ocs.xdata(ocs.first(layer("-GW-CUNET_BODEM-G")))
        checks.append(("XDATA", any("corridor=Corridor Hoofdstraat" in str(x) for x in xd), str(xd[:2])))
        bad = [f"{n}: {v}" for n, ok, v in checks if not ok]
        return not bad, "; ".join(bad) if bad else "; ".join(f"{n} {v}" for n, _, v in checks[:4]) + f"; afgraven tot onderkant {d['afgraven_onderkant_m3']:.0f} m³"

    def assemblies():
        r = cmd("CIV_PROFIELTYPEN", "pt")
        if not r["ok"]:
            return False, r["message"]
        names = {p["naam"]: p for p in r["data"]["profieltypen"]}
        mine = names.get("polder-e2e")
        presets = [n for n in ["weg", "fietspad", "snelweg", "dijk", "stadsstraat", "woonstraat", "polderweg", "2x2-middenberm"] if n in names]
        if not mine or not mine.get("voorwaarden") or "fout" in mine:
            return False, f"polder-e2e: {mine}"
        r2 = cmd("CIV_PROFIELTYPE NAAM=polder-e2e PLAATS=108100,420600", "pt_draw")
        return r2["ok"] and len(presets) == 8, f"{len(names)} profieltypen ({len(presets)} standaard), polder-e2e met voorwaarden; getekend: {r2['message'][-40:]}"

    def junction():
        a, b = state["axes"]["a"][0], state["axes"]["b"][0]
        r = cmd(f"CIV_KRUISPUNT HOOFD={a} ZIJ={b} R=8 PROFIELTYPE_HOOFD=stadsstraat PROFIELTYPE_ZIJ=polder-e2e", "kp", 300)
        if not r["ok"]:
            return False, r["message"]
        d = r["data"]
        state["junction"] = d
        side = [x for x in d["armen"] if x["weg"] == "zijweg"]
        ok = d["type"] == "T" and len(d["armen"]) == 3 and len(d["hoeken"]) == 2 and d["driehoeken"] > 100 and len(side) == 1 and d["haaientanden"] > 0
        tris = ocs.handles(layer("-OG-TRIANGULATIE-G"))
        kp = any("kruispunt=" in str(ocs.xdata(t)) for t in tris)
        if not (ok and kp):
            return False, f"type {d['type']}, armen {len(d['armen'])}, hoeken {len(d['hoeken'])}, driehoeken {d['driehoeken']}, haaientanden {d['haaientanden']}, XDATA kruispunt {kp}"
        return True, f"{d['type']}, {len(d['armen'])} armen, hoeken R {[c['straal_m'] for c in d['hoeken']]}, {d['driehoeken']} driehoeken, zijweg {side[0]['corridor'] if side else '?'}, hoogte {d['hoogte']:.2f}"

    def side_road():
        b = state["axes"]["b"][0]
        side = next(x for x in state["junction"]["armen"] if x["weg"] == "zijweg")
        r = cmd(f"CIV_CORRIDOR AS={b} PROFIELTYPE=polder-e2e {side['corridor']} FREQUENTIE=5", "cor_b", 600)
        if not r["ok"]:
            return False, r["message"]
        d = r["data"]
        sections = d["profielen"]
        ditch = sum(1 for p in sections if "SL" in p["codes"].split())
        berm2 = sum(1 for p in sections if p["codes"].split().count("BE") >= 4)
        end = max(p["station"] for p in sections)
        mouth = float(side["corridor"].split("=")[1])
        return ditch > 0 and berm2 > 0 and abs(end - mouth) < 0.01, f"{len(sections)} profielen: {ditch} met sloot (ingraving), {berm2} met brede berm (ophoging); tot {end:.2f} = mond {mouth:.2f}"

    def roundabout():
        c, dd = state["axes"]["c"][0], state["axes"]["d"][0]
        r = cmd(f"CIV_ROTONDE ASSEN={c};{dd} TYPE=binnen PROFIELTYPE=weg", "rot", 300)
        if not r["ok"]:
            return False, r["message"]
        d = r["data"]
        state["roundabout"] = d
        ok = len(d["armen"]) == 4 and abs(d["buitendiameter_m"] - 32.0) < 1e-6 and abs(d["rijbaan_m"] - 5.5) < 1e-6 and d["driehoeken"] > 300
        ok &= d["hoogte_binnenrand"] > d["hoogte_buitenrand"]
        ok &= layer("-VH-KANTOPSLUITING_ROTONDEBLOK-G") is not None and layer("-VW-MARKERING_DWARS_DRIEHOEK-G") is not None
        return ok, f"D {d['buitendiameter_m']} m, rijbaan {d['rijbaan_m']} m, {len(d['armen'])} armen, {d['driehoeken']} driehoeken, binnenrand {d['hoogte_binnenrand']:.2f} > buitenrand {d['hoogte_buitenrand']:.2f}, {d['haaientanden']} haaientanden"

    def arms():
        done = []
        axes = {state["axes"]["c"][0]: "c", state["axes"]["d"][0]: "d"}
        names = {"Rotonde-WO": state["axes"]["c"][0], "Rotonde-ZN": state["axes"]["d"][0]}
        for i, a in enumerate(state["roundabout"]["armen"]):
            h = names[a["as"]]
            r = cmd(f"CIV_CORRIDOR AS={h} PROFIELTYPE=weg {a['corridor']} FREQUENTIE=10", f"cor_arm{i}", 600)
            if not r["ok"]:
                return False, f"{a['as']} {a['corridor']}: {r['message']}"
            done.append(f"{axes[h]}{a['richting']} {r['data']['dwarsprofielen']}")
        return True, "armcorridors: " + ", ".join(done)

    def ifc():
        """Main's IFC 4.3 export: courses per pavement layer, kerbs, roads."""
        path = OUT / "corridor.ifc"
        r = cmd(f"CIV_IFC BESTAND={path}", "ifc", 600)
        if not r["ok"]:
            return False, r["message"]
        k = r["data"]["klassen"]
        text = path.read_text(errors="replace") if path.exists() else ""
        # One road per axis (the arm corridors of an axis share it), the junction and the roundabout.
        ok = k.get("IfcCourse", 0) >= 10 and k.get("IfcKerb", 0) >= 3 and k.get("IfcRoad", 0) >= 6
        ok &= "IFCCOURSE(" in text and ".PAVEMENT." in text and "IFCKERB(" in text and "'menggranulaat (fundering)'" in text
        ok &= "IFCROAD(" in text and "'Kruispunt Hoofdstraat / Zijweg'" in text and "'Rotonde Rotonde-WO / Rotonde-ZN'" in text
        ok &= r["data"]["overgeslagen"].get("profieltype", 0) > 0
        return ok, ", ".join(f"{c} {n}" for c, n in sorted(k.items()) if c != "IfcAnnotation") + f"; overgeslagen {r['data']['overgeslagen']}"

    def info():
        r = cmd("CIV_INFO", "info")
        n = len(r["data"].get("corridors", []))
        return r["ok"] and n >= 6, f"{n} corridors, terrein ontwerp {r['data'].get('terrein_ontwerp')}"

    def move_ground(dx):
        hs = ocs.handles("B-XX-OG-TRIANGULATIE-G")
        if hs:
            ocs.call("/entities/transform", {"handles": hs, "action": "move", "vector": [dx, 0.0, 0.0], "document_id": ocs.doc()})
            time.sleep(1)

    def captures():
        made = []
        # The existing ground hides the design in plan: out of the way.
        move_ground(5000.0)
        ocs.run("-VIEW TOP")
        ocs.run("VSCURRENT WIREFRAME2D")
        time.sleep(1)
        ocs.run("ZOOM EXTENTS")
        time.sleep(2)
        for name, b in [
            ("plan-overzicht.png", [108200, 420290, 108600, 420610]),
            ("plan-kruispunt.png", [108280, 420520, 108360, 420590]),
            ("plan-rotonde.png", [108410, 420340, 108530, 420460]),
            ("plan-stadsstraat.png", [108440, 420535, 108490, 420585]),
            ("profieltype.png", [108080, 420560, 108125, 420610]),
        ]:
            made.append(name if capture(name, b) else f"{name}✘")
        at = state.get("street", {}).get("dwarsprofielen_plaats")
        if at:
            made.append("dwarsprofielen.png" if capture("dwarsprofielen.png", [at[0], at[1] - 60, at[0] + 90, at[1] + 8]) else "dwarsprofielen✘")
        ocs.run("-VIEW SWISO")
        ocs.run("VSCURRENT GOURAUDSHADED")
        time.sleep(1)
        ocs.run("ZOOM EXTENTS")
        time.sleep(2)
        for name, b in [
            ("3d-ontwerp.png", [108200, 420290, 108600, 420610]),
            ("3d-kruispunt.png", [108280, 420520, 108360, 420590]),
            ("3d-rotonde.png", [108410, 420340, 108530, 420460]),
            ("3d-stadsstraat.png", [108420, 420530, 108500, 420590]),
        ]:
            made.append(name if capture(name, b) else f"{name}✘")
        move_ground(-5000.0)
        time.sleep(1)
        made.append("3d-met-maaiveld.png" if capture("3d-met-maaiveld.png", [108200, 420290, 108600, 420610]) else "3d-met-maaiveld✘")
        return all("✘" not in m for m in made), ", ".join(made)

    ok = step("site", site) and step("terrain (AHN TIN)", terrain) and step("axes and profiles", axes)
    if ok:
        step("town street corridor", street)
        step("JSON assemblies", assemblies)
        if step("T junction", junction):
            step("side road corridor from the mouth", side_road)
        if step("roundabout", roundabout):
            step("arm corridors", arms)
        step("model info", info)
        step("IFC 4.3 export", ifc)
        step("captures", captures)
    try:
        ocs.run(f"QSAVEAS {OUT / 'corridor.dwg'}")
        time.sleep(3)
    except Exception:  # noqa: BLE001
        pass
    ocs.stop()
    lines = ["# Corridor, junction and roundabout e2e", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {d} | {t:.1f} |" for n, ok, d, t in results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if results and all(ok for _, ok, _, _ in results) else 1)


if __name__ == "__main__":
    main()
