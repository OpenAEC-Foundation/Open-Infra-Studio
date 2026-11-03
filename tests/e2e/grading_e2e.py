#!/usr/bin/env python3
"""End-to-end test of grading in Open CAD Studio: feature lines, plateaus and
slopes, the combined design surface, TIN-to-TIN earthwork and the mass haul
diagram of a corridor.

Runs its own Open CAD Studio on its own port with an isolated configuration
folder (``XDG_CONFIG_HOME``), so the plugin installed for daily use is left
alone. A window appears, as with ``ocs_e2e.py``.

The site is a polder in the Dordtse Biesbosch (RD 109500 420500, 300 × 200 m):
AHN ground between about −1 and +1.6 m NAP, with ditches and ponds.

Steps
  terrain        CIV_TIN BRON=AHN: the existing ground (B).
  feature lines  CIV_KANTLIJN from the ground (with grade breaks), from a start
                 level and grades, and from fixed levels; the four edits
                 (level, raise, height point, grade) keep the same handle.
  plateaus       two building plots: one in fill with a fall, one from a
                 polyline drawn in the drawing at the mean ground level (cut and
                 fill slopes); a slope from a feature line to a level. All join
                 one design surface; contours on it.
  volumes        CIV_VOLUME BASIS=TIN against the grid volume of CIV_VOLUME and
                 against the sum of the gradings' own volumes; bulking and
                 compaction; the cut/fill map.
  mass haul      a road corridor (axis, profile, design profile) and its mass
                 haul diagram, from the corridor's list and measured on the TINs.
  captures       plan and 3D of the gradings, the cut/fill map, the diagram and
                 the whole site, in target/e2e-grading/.

Usage
  python3 tests/e2e/grading_e2e.py [--ocs PATH] [--port 18817] [--no-install]

The report goes to target/e2e-grading/rapport.md.
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
OUT = REPO / "target" / "e2e-grading"
SITE = (109500.0, 420500.0)


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


class Run:
    def __init__(self, ocs, xdg):
        self.ocs = ocs
        self.api = xdg / "OpenCADStudio" / "open-infra" / "api"
        self.n = 0
        self.results = []

    def cmd(self, cmd, timeout=600):
        """An Open Infra command with ID=; its JSON result."""
        self.n += 1
        rid = f"grading{self.n}"
        p = self.api / f"{rid}.json"
        if p.exists():
            p.unlink()
        self.ocs.run(f"{cmd} ID={rid}")
        t = time.time()
        while time.time() - t < timeout:
            if p.exists():
                time.sleep(0.3)
                r = json.loads(p.read_text())
                if not r.get("ok"):
                    raise RuntimeError(r.get("message"))
                return r["data"] | {"_message": r.get("message", "")}
            time.sleep(0.5)
        raise RuntimeError(f"{cmd.split()[0]}: no answer in {timeout} s")

    def step(self, name, fn):
        t = time.time()
        try:
            ok, detail = fn()
        except Exception as e:  # noqa: BLE001 — the report wants everything
            ok, detail = False, f"{type(e).__name__}: {e}"
        self.results.append((name, ok, str(detail), time.time() - t))
        print(("✔" if ok else "✘"), name, "—", detail, flush=True)
        return ok

    def capture(self, name, bounds=None, size=1400):
        path = OUT / f"{name}.png"
        if path.exists():
            path.unlink()
        body = {"path": str(path), "document_id": self.ocs.doc(), "max_dimension": size}
        if bounds:
            body.update({"view": "region", "bounds": bounds})
        r = self.ocs.call("/capture", body)
        t = time.time()
        while not (path.exists() and path.stat().st_size > 0) and time.time() - t < 60:
            time.sleep(0.3)
        return r.get("ok", False) and path.exists(), path


def rd(dx, dy):
    return f"{SITE[0] + dx:.2f},{SITE[1] + dy:.2f}"


def pts(*xy):
    return ";".join(rd(x, y) for x, y in xy)


def close(a, b, rel=0.0, absolute=1e-6):
    return abs(a - b) <= max(absolute, rel * max(abs(a), abs(b)))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18817)
    ap.add_argument("--no-install", action="store_true", help="use the plugins already in target/e2e-grading/xdg")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.no_install:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    (xdg / "OpenCADStudio" / "open-infra").mkdir(parents=True, exist_ok=True)
    (xdg / "OpenCADStudio" / "open-infra" / "settings.json").write_text(json.dumps({"language": "nl"}))
    ocs_e2e.OUT = OUT
    ocs = ocs_e2e.Ocs(args.ocs, args.port)
    run = Run(ocs, xdg)
    s = {}
    try:
        def site():
            ocs.run(f"GIS_SITE RD {SITE[0]:.0f} {SITE[1]:.0f} 300 200 Biesbosch polder")
            time.sleep(2)
            return True, "GIS_SITE RD 109500 420500 300 200"

        def terrain():
            d = run.cmd("CIV_TIN BRON=AHN AFSTAND=2")
            s["ground"] = (d["hoogte_min"], d["hoogte_max"])
            return d["driehoeken"] > 20000, f"{d['driehoeken']} triangles, {d['hoogte_min']:.2f} to {d['hoogte_max']:.2f} m"

        def feature_surface():
            d = run.cmd(f"CIV_KANTLIJN PUNTEN={pts((100, -80), (140, -80), (140, -40))} HOOGTE=MAAIVELD TUSSENPUNTEN=ja NAAM=Sloot")
            s["kl_ground"] = d
            e = ocs.entity(d["kantlijn"])
            xd = " ".join(map(str, ocs.xdata(d["kantlijn"])))
            ok = d["punten"] >= 3 and "BREUKLIJN" in d["laag"] and d["laag"].startswith("N-") and "kantlijn=Sloot" in xd
            return ok, f"{d['punten']} points (grade breaks), {d['z_min']:.2f} to {d['z_max']:.2f} m, {e.get('type')} on {d['laag']}"

        def feature_grades():
            d = run.cmd(f"CIV_KANTLIJN PUNTEN={pts((100, -95), (130, -95), (150, -95))} START=0.5 HELLING=1;-2 NAAM=Rand")
            s["kl"] = d
            z = d["hoogtes"]
            return close(z[1], 0.8) and close(z[2], 0.4), f"heights {[round(v, 3) for v in z]} (0.5 + 1 % × 30 m − 2 % × 20 m)"

        def feature_levels():
            d = run.cmd(f"CIV_KANTLIJN PUNTEN={pts((100, -60), (120, -60), (140, -60), (140, -50))} PEILEN=1:0.2;3:0.6")
            z = d["hoogtes"]
            return close(z[1], 0.4) and close(z[3], 0.6), f"heights {[round(v, 3) for v in z]}"

        def edits():
            h = s["kl"]["kantlijn"]
            notes = []
            d = run.cmd(f"CIV_KANTLIJN_VERHOOG KANTLIJN={h} DZ=0.25")
            ok = close(d["hoogtes"][0], 0.75) and d["kantlijn"] == h
            notes.append(f"raise → {d['hoogtes'][0]:.2f}")
            d = run.cmd(f"CIV_KANTLIJN_PEIL KANTLIJN={h} PUNT=2 Z=1.4")
            ok &= close(d["hoogtes"][1], 1.4)
            notes.append(f"level → {d['hoogtes'][1]:.2f}")
            d = run.cmd(f"CIV_KANTLIJN_PUNT KANTLIJN={h} BIJ={rd(115, -95)}")
            ok &= d["punten"] == 4 and close(d["hoogtes"][1], (0.75 + 1.4) / 2)
            notes.append(f"point → {d['punten']} points, {d['hoogtes'][1]:.3f}")
            d = run.cmd(f"CIV_KANTLIJN_HELLING KANTLIJN={h} VAN=1 TOT=4 HELLING=1.5")
            ok &= close(d["hoogtes"][3], 0.75 + 0.015 * 50) and close(d["hoogtes"][1], 0.75 + 0.015 * 15)
            ok &= d["kantlijn"] == h
            notes.append(f"grade → {[round(v, 3) for v in d['hoogtes']]}")
            xd = " ".join(map(str, ocs.xdata(h)))
            ok &= "bewerkt=" in xd
            return ok, "; ".join(notes) + f"; same handle {h}"

        def plateau_fill():
            # Ground level under the plot first, from a feature line on it.
            g = run.cmd(f"CIV_KANTLIJN PUNTEN={pts((95, -5), (125, -5), (125, 25), (95, 25))} GESLOTEN=ja NAAM=Kavel_1_maaiveld")
            level = round(g["z_max"] + 0.6, 2)
            s["p1_level"] = level
            d = run.cmd(f"CIV_PLATEAU PUNTEN={pts((95, -5), (125, -5), (125, 25), (95, 25))} PEIL={level} AFSCHOT=1 RICHTING=Z TALUD_AF=1:2 TALUD_OP=1:3 NAAM=Kavel_1")
            s["p1"] = d
            layers = ocs.layers()
            ok = d["zonder_aansluiting"] == 0 and d["gradings_in_ontwerp"] == 1 and any("TALUD_TEEN" in l for l in layers) and any("TALUDARCERING" in l for l in layers)
            return ok, f"level {level} (ground ≤ {g['z_max']:.2f}), plateau {d['oppervlak_plateau_m2']:.0f} m², slopes {d['oppervlak_talud_m2']:.0f} m², fill {d['ophogen_m3']:.1f} m³, cut {d['afgraven_m3']:.1f} m³"

        def plateau_polyline():
            # A polyline drawn in the drawing, at the mean ground level.
            corners = [(95, -45), (125, -45), (125, -15), (110, -15), (110, -30), (95, -30)]
            r = ocs.call("/entities", {"entities": [{"type": "LwPolyline", "vertices": [[SITE[0] + x, SITE[1] + y] for x, y in corners], "closed": True}], "document_id": ocs.doc()})
            h = r["result"]["handles"][0]
            g = run.cmd(f"CIV_KANTLIJN LIJN={h} NAAM=Kavel_2_maaiveld")
            level = round((g["z_min"] + g["z_max"]) / 2, 2)
            d = run.cmd(f"CIV_PLATEAU LIJN={h} PEIL={level} TALUD=1:2 NAAM=Kavel_2")
            s["p2"] = d
            ok = d["gradings_in_ontwerp"] == 2 and d["afgraven_m3"] > 0 and d["ophogen_m3"] > 0
            return ok, f"L-shaped plot at {level}: cut {d['afgraven_m3']:.1f} m³, fill {d['ophogen_m3']:.1f} m³, design surface {d['ontwerp_driehoeken']} triangles"

        def slope():
            d = run.cmd(f"CIV_TALUD LIJN={s['kl']['kantlijn']} ZIJDE=RECHTS NAAR=PEIL PEIL=-0.5 TALUD=1:2 NAAM=Talud_rand")
            s["t1"] = d
            return d["oppervlak_talud_m2"] > 10 and d["ontwerp_driehoeken"] > s["p2"]["ontwerp_driehoeken"], f"{d['oppervlak_talud_m2']:.0f} m², cut {d['afgraven_m3']:.1f} m³, fill {d['ophogen_m3']:.1f} m³"

        def contours():
            d = run.cmd("CIV_HOOGTELIJNEN OPPERVLAK=N INTERVAL=0.1")
            return d.get("elementen", 0) > 0, d["_message"][:120]

        def volume_tin():
            d = run.cmd("CIV_VOLUME BASIS=TIN ONTWERP=BOUWRIJP LOSFACTOR=1.25 VERDICHTING=1.1 VERGELIJK=ja CEL=0.25")
            s["vt"] = d
            parts = sum(s[k]["afgraven_m3"] for k in ("p1", "p2", "t1")), sum(s[k]["ophogen_m3"] for k in ("p1", "p2", "t1"))
            grid_ok = close(d["afgraven_m3"], d["raster_afgraven_m3"], 0.02, 0.5) and close(d["ophogen_m3"], d["raster_ophogen_m3"], 0.02, 0.5)
            sum_ok = close(d["afgraven_m3"], parts[0], 0.01, 0.2) and close(d["ophogen_m3"], parts[1], 0.01, 0.2)
            bal_ok = close(d["balans_m3"], d["afgraven_m3"] * 1.25 - d["ophogen_m3"] * 1.1, 1e-9, 1e-6)
            bands = len(d["banden"])
            return grid_ok and sum_ok and bal_ok and bands >= 2, (
                f"TIN cut {d['afgraven_m3']:.2f} / fill {d['ophogen_m3']:.2f} m³; grid 0.25 m {d['raster_afgraven_m3']:.2f} / {d['raster_ophogen_m3']:.2f}; "
                f"gradings {parts[0]:.2f} / {parts[1]:.2f}; balance {d['balans_m3']:.2f} m³ loose; {bands} bands"
            )

        def volume_grid():
            d = run.cmd("CIV_VOLUME CEL=0.5")
            t = s["vt"]
            ok = close(d["afgraven_m3"], t["afgraven_m3"], 0.03, 1.0) and close(d["ophogen_m3"], t["ophogen_m3"], 0.03, 1.0)
            return ok, f"CIV_VOLUME grid 0.5 m: cut {d['afgraven_m3']:.2f}, fill {d['ophogen_m3']:.2f} m³ against TIN {t['afgraven_m3']:.2f} / {t['ophogen_m3']:.2f}"

        def corridor():
            a = run.cmd(f"CIV_AS PI={pts((-105, -95), (-100, 0), (-115, 95))} R=200 NAAM=Polderweg")
            s["as"] = a
            lp = run.cmd(f"CIV_LENGTEPROFIEL AS={a['as']}")
            mean = (lp["maaiveld_min"] + lp["maaiveld_max"]) / 2
            L = a["lengte_m"]
            pvi = f"0:{mean + 0.5:.2f};{L / 2:.1f}:{mean - 0.45:.2f};{L:.1f}:{mean + 0.5:.2f}"
            run.cmd(f"CIV_ONTWERPPROFIEL AS={a['as']} PVI={pvi} BOOG=60")
            c = run.cmd(f"CIV_CORRIDOR AS={a['as']} SJABLOON=weg FREQUENTIE=10")
            s["c"] = c
            return c["dwarsprofielen"] > 10 and Path(c["csv"]).exists(), f"{L:.0f} m, {c['dwarsprofielen']} sections, cut {c['afgraven_m3']:.0f} m³, fill {c['ophogen_m3']:.0f} m³"

        def mass_haul():
            d = run.cmd(f"CIV_GRONDBALANS AS={s['as']['as']} VRIJ=50 FACTOR=0.9 HERGEBRUIK=80")
            s["gb"] = d
            c = s["c"]
            ok = len(d["stations"]) == c["dwarsprofielen"] and close(d["afgraven_m3"], c["afgraven_m3"], 0.001, 1.0) and close(d["ophogen_m3"], c["ophogen_m3"], 0.001, 1.0)
            layers = ocs.layers()
            ok &= Path(d["csv"]).exists() and "OI-GRONDBALANS-MASSALIJN" in layers and ("OI-GRONDBALANS-VOORUIT" in layers or "OI-GRONDBALANS-TERUG" in layers)
            # The last mass ordinate: reused cut × factor − fill.
            ok &= close(d["eindmassa_m3"], d["afgraven_m3"] * 0.8 * 0.9 - d["ophogen_m3"], 1e-6, 0.5)
            return ok, (
                f"{len(d['stations'])} stations, {len(d['balanspunten'])} balance points {[b['tekst'] for b in d['balanspunten']]}, "
                f"haul {d['transport_m3km']:.2f} m³·km (mean {d['gemiddelde_afstand_m']:.0f} m), beyond free haul {d['bovenmatig_m3']:.0f} m³ / {d['bovenmatig_m3km']:.2f} m³·km, "
                f"surplus {d['overschot_m3']:.0f}, shortage {d['tekort_m3']:.0f} m³"
            )

        def mass_haul_tin():
            d = run.cmd(f"CIV_GRONDBALANS AS={s['as']['as']} BRON=TIN VRIJ=50 FACTOR=0.9 HERGEBRUIK=80 PLAATS={rd(-150, -400)}")
            c = s["gb"]
            ok = close(d["afgraven_m3"], c["afgraven_m3"], 0.03, 5.0) and close(d["ophogen_m3"], c["ophogen_m3"], 0.03, 5.0)
            return ok, f"measured on the TINs: cut {d['afgraven_m3']:.0f}, fill {d['ophogen_m3']:.0f} m³ against the corridor list {c['afgraven_m3']:.0f} / {c['ophogen_m3']:.0f}"

        plots = [SITE[0] + 80, SITE[1] - 105, SITE[0] + 160, SITE[1] + 35]

        def capture_grading():
            # Shaded from the top (the plateaus over the ground), then in 3D.
            ocs.run("-VIEW TOP")
            ocs.run("VSCURRENT GOURAUDSHADED")
            ok1, p1 = run.capture("plan-grading", plots)
            ocs.run("-VIEW SWISO")
            ok2, p2 = run.capture("3d-grading", [SITE[0] + 85, SITE[1] - 50, SITE[0] + 135, SITE[1] + 30])
            ocs.run("-VIEW TOP")
            ocs.run("VSCURRENT WIREFRAME2D")
            return ok1 and ok2, f"{p1.name}, {p2.name}"

        def capture_map():
            ok, p = run.capture("kaart-grondverzet", [SITE[0] + 80, SITE[1] - 105, SITE[0] + 230, SITE[1] + 40])
            return ok, p.name

        def capture_diagram():
            pl = s["gb"]["plaats"]
            L = s["as"]["lengte_m"]
            # Shaded, so the haul areas show.
            ocs.run("VSCURRENT GOURAUDSHADED")
            ok1, p1 = run.capture("massadiagram", [pl[0] - 25, pl[1] - 3, pl[0] + L + 5, pl[1] + 62], 2000)
            ocs.run("VSCURRENT WIREFRAME2D")
            ok2, p2 = run.capture("plan-site", [SITE[0] - 160, SITE[1] - 110, SITE[0] + 160, SITE[1] + 110])
            return ok1 and ok2, f"{p1.name}, {p2.name}"

        def save():
            target = OUT / "grading.dwg"
            if target.exists():
                target.unlink()
            r = ocs.call("/save", {"path": str(target), "document_id": ocs.doc()})
            t = time.time()
            while not target.exists() and time.time() - t < 60:
                time.sleep(0.5)
            if not target.exists():
                # Older hosts: save the drawing under its own name, then copy it.
                ocs.run("QSAVE")
                time.sleep(5)
                start = OUT / "start.dwg"
                if start.exists():
                    target.write_bytes(start.read_bytes())
            return target.exists() and target.stat().st_size > 10_000, f"{target} ({target.stat().st_size if target.exists() else 0} bytes; /save: {r.get('ok')})"

        run.step("site", site)
        run.step("terrain (AHN TIN)", terrain)
        run.step("feature line from the ground", feature_surface)
        run.step("feature line from grades", feature_grades)
        run.step("feature line from levels", feature_levels)
        run.step("feature line edits", edits)
        run.step("plateau in fill with a fall", plateau_fill)
        run.step("plateau from a polyline", plateau_polyline)
        run.step("slope to a level", slope)
        run.step("captures of the grading", capture_grading)
        run.step("contours on the design", contours)
        run.step("TIN-to-TIN volume", volume_tin)
        run.step("capture of the cut/fill map", capture_map)
        run.step("grid volume against TIN", volume_grid)
        run.step("corridor", corridor)
        run.step("mass haul (corridor list)", mass_haul)
        run.step("mass haul (measured on TINs)", mass_haul_tin)
        run.step("captures of the diagram and site", capture_diagram)
        run.step("saved", save)
    finally:
        ocs.stop()
    lines = ["# Grading e2e", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {d.replace('|', '/')} | {t:.1f} |" for n, ok, d, t in run.results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if all(ok for _, ok, _, _ in run.results) else 1)


if __name__ == "__main__":
    main()
