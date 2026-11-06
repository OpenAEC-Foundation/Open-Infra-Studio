#!/usr/bin/env python3
"""End-to-end test of the geotechnics tools in Open CAD Studio: a soil profile
from BRO CPTs and boreholes, the slope stability of a dike section, and the
settlement of a 1.5 m fill in the polder behind it.

Runs its own Open CAD Studio on its own port with an isolated configuration
folder (``XDG_CONFIG_HOME``), so the plugin installed for daily use is left
alone. A window appears, as with ``ocs_e2e.py``.

Two Dutch sites with BRO CPTs (a scan of candidate sites near Gouda and
Dordrecht found three to eleven CPTs at each):

- SITE: a dike at Wieldrecht, on the south side of Dordrecht. Eleven CPTs
  (2018–2019), four of them in a line across the dike from the crest at about
  +7.8 m NAP down its inner side, 20 to 28 m deep: the soil profile and the
  stability of the dike section.
- POLDER: the polder at Moordrecht on the Hollandse IJssel, near Gouda, at
  about −2 m NAP. Three CPTs of 2026, 15 m deep, in a row: a second soil
  profile and the settlement of a 1.5 m fill on its soft soil.

Steps
  site and terrain  GIS_SITE, CIV_TIN BRON=AHN (1 m).
  BRO               GIS_Q_SONDERINGEN: the CPTs and boreholes of the site.
  soil profile      CIV_GRONDPROFIEL along a line across the dike through the
                    CPTs: columns, soil units, polygons, the stored soil
                    model, XDATA.
  stability         CIV_STABILITEIT on a dike section along the same line:
                    Bishop on the soil model with the default phreatic level;
                    the ordinary method of slices with a phreatic line and a
                    13 kPa traffic load on the crest; Bishop with layers given
                    by hand (LAGEN=) and own peat parameters (GROND=).
  settlement        at POLDER: the AHN TIN, a soil profile along the CPT row,
                    CIV_ZETTING of a 1.5 m fill on it: Koppejan and Terzaghi,
                    the map, contours and the time curve.
  captures          the profile, the stability section, the settlement map
                    and the graph, the site, in target/e2e-geotechniek/.

Usage
  python3 tests/e2e/geotechniek_e2e.py [--ocs PATH] [--port 18823] [--no-install]

The report goes to target/e2e-geotechniek/rapport.md.
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
OUT = REPO / "target" / "e2e-geotechniek"
# Wieldrecht, Dordrecht: the dike and the land behind it (RD New, m).
SITE = (104720.0, 420820.0, 440.0, 340.0)
# The polder at Moordrecht, near Gouda.
POLDER = (105830.0, 444720.0, 300.0, 220.0)


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


class Run:
    def __init__(self, ocs, xdg):
        self.ocs = ocs
        self.api = xdg / "OpenCADStudio" / "open-infra" / "api"
        self.n = 0
        self.results = []

    def cmd(self, cmd, timeout=900):
        """An Open Infra command with ID=; its JSON result."""
        self.n += 1
        rid = f"geo{self.n}"
        p = self.api / f"{rid}.json"
        if p.exists():
            p.unlink()
        self.ocs.run(f"{cmd} ID={rid}")
        t = time.time()
        while time.time() - t < timeout:
            if p.exists():
                time.sleep(0.3)
                r = json.loads(p.read_text())
                (OUT / f"{rid}.json").write_text(json.dumps(r, indent=1, ensure_ascii=False))
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

    def capture(self, name, bounds=None, size=1600):
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


    def plot(self, name, window):
        """A region of the model plotted to PDF (vector, no window chrome),
        rendered to PNG with pdftoppm."""
        pdf = OUT / f"{name}.pdf"
        for p in (pdf, OUT / f"{name}-plot.png"):
            if p.exists():
                p.unlink()
        w, h = window[2] - window[0], window[3] - window[1]
        r = self.ocs.call("/plot", {"path": str(pdf), "document_id": self.ocs.doc(), "area": "window", "window": window,
                                    "orientation": "Landscape" if w >= h else "Portrait", "fit": True})
        t = time.time()
        while not (pdf.exists() and pdf.stat().st_size > 0) and time.time() - t < 60:
            time.sleep(0.3)
        if pdf.exists():
            subprocess.run(["pdftoppm", "-r", "110", "-png", "-singlefile", str(pdf), str(OUT / f"{name}-plot")], check=False)
        return (OUT / f"{name}-plot.png").exists(), r


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18823)
    ap.add_argument("--no-install", action="store_true", help="use the plugins already in target/e2e-geotechniek/xdg")
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
            x, y, w, h = SITE
            ocs.run(f"GIS_SITE RD {x:.0f} {y:.0f} {w:.0f} {h:.0f} Dordrecht Wieldrecht dijk")
            time.sleep(2)
            return True, f"GIS_SITE RD {x:.0f} {y:.0f} {w:.0f} {h:.0f}"

        def terrain():
            d = run.cmd("CIV_TIN BRON=AHN AFSTAND=2")
            s["ground"] = (d["hoogte_min"], d["hoogte_max"])
            return d["driehoeken"] > 20000, f"{d['driehoeken']} triangles, {d['hoogte_min']:.2f} to {d['hoogte_max']:.2f} m NAP"

        def bro():
            d = run.cmd("GIS_Q_SONDERINGEN")
            objs = d["objecten"]
            cpts = [o for o in objs if o["soort"] == "sondering"]
            s["cpts"] = cpts
            s["bhrs"] = [o for o in objs if o["soort"] != "sondering"]
            return len(cpts) >= 3, f"{len(cpts)} CPTs, {len(s['bhrs'])} boreholes: {', '.join(o['bro_id'] for o in cpts[:6])}…"

        def line():
            # Across the dike: from the highest CPT (the crest) to the lowest
            # surface, 30 m beyond the crest and 60 m beyond the last one.
            c = [o for o in s["cpts"] if o["maaiveld_m_nap"] is not None]
            hi = max(c, key=lambda o: o["maaiveld_m_nap"])
            lo = max(c, key=lambda o: math.dist((o["x"], o["y"]), (hi["x"], hi["y"])))
            (ax, ay), (bx, by) = (hi["x"], hi["y"]), (lo["x"], lo["y"])
            L = math.dist((ax, ay), (bx, by))
            ux, uy = (bx - ax) / L, (by - ay) / L
            p0, p1 = (ax - 30 * ux, ay - 30 * uy), (bx + 60 * ux, by + 60 * uy)
            d = run.cmd(f"CIV_AS PI={p0[0]:.2f},{p0[1]:.2f};{p1[0]:.2f},{p1[1]:.2f} NAAM=Dijkprofiel")
            s["axis"] = d["as"]
            s["line"] = (p0, p1)
            s["dir"] = (ux, uy)
            # The stability section: the same line, 30 m before the crest to 90 m behind it.
            q0, q1 = p0, (ax + 90 * ux, ay + 90 * uy)
            e = run.cmd(f"CIV_AS PI={q0[0]:.2f},{q0[1]:.2f};{q1[0]:.2f},{q1[1]:.2f} NAAM=Dijkdoorsnede")
            s["section"] = e["as"]
            return bool(d["as"]) and bool(e["as"]), f"profile axis {d['as']} from {hi['bro_id']} (mv {hi['maaiveld_m_nap']:.2f}) to {lo['bro_id']} (mv {lo['maaiveld_m_nap']:.2f}), {L + 90:.0f} m; section {e['as']}, 120 m"

        def profile():
            d = run.cmd(f"CIV_GRONDPROFIEL AS={s['axis']} STRAAL=30 NAAM=Dijkprofiel")
            s["profile"] = d
            m = d["model"]
            cols = m["columns"]
            layers = ocs.layers()
            xd = " ".join(map(str, ocs.xdata(d["grondprofiel"])))
            ok = len(cols) >= 2 and d["polygons"] >= len(d["units"]) and len(d["units"]) >= 2
            ok &= any("MO-BODEM_BOORPROFIEL-A" in l for l in layers) and any("ONDERZOEK_SONDERING-G" in l for l in layers)
            ok &= "grondprofiel=Dijkprofiel" in xd and m["base"] < m["columns"][0]["surface"]
            # Every column's layers are contiguous and run from its surface down.
            for c in cols:
                ok &= abs(c["layers"][0]["top"] - c["surface"]) < 1e-6
                ok &= all(abs(a["bottom"] - b["top"]) < 1e-6 for a, b in zip(c["layers"], c["layers"][1:]))
            kinds = {c["kind"] for c in cols}
            return ok, (f"{len(cols)} columns ({', '.join(sorted(kinds))}), {len(d['units'])} units {d['units']}, {d['polygons']} polygons, "
                        f"base {m['base']:.2f} m NAP; skipped {len(d['skipped'])}")

        def stability():
            d = run.cmd(f"CIV_STABILITEIT LIJN={s['section']} MODEL=Dijkprofiel NAAM=Dijkvak")
            s["stab"] = d
            c = d["critical_circle"]
            sl = d["slices"]
            layers = ocs.layers()
            ok = 0.3 < d["factor_of_safety"] < 10 and len(sl) == 50 and c["entry_x"] < c["exit_x"]
            ok &= all(x["w"] > 0 for x in sl) and any("MO-BODEM-G" in l for l in layers) and any("MO-GRONDWATER-G" in l for l in layers)
            other = d["other_method"]
            ok &= other["on_critical_circle"] is not None and other["on_critical_circle"] <= d["factor_of_safety"] + 1e-6
            xd = " ".join(map(str, ocs.xdata(d["handles"][0])))
            ok &= "veiligheidsfactor=" in xd
            return ok, (f"Bishop F = {d['factor_of_safety']:.3f}, circle ({c['xc']:.1f}, {c['zc']:.1f}) R {c['r']:.1f}, "
                        f"x {c['entry_x']:.1f}–{c['exit_x']:.1f}; Fellenius on it {other['on_critical_circle']:.3f}, min {other['minimum']:.3f}; "
                        f"{d['circles_tried']} circles; soil {d['soil']}; units {sorted(d['units'])}")

        def stability_water_load():
            # A phreatic line falling from the outer side through the dike to
            # the land behind it, and 13 kPa traffic on the crest: the
            # factor must drop.
            surf = s["stab"]["surface"]
            # The crest: the highest ground near the crest CPT (30 m into the section).
            crest_x, crest_z = max((p for p in surf if 10 <= p[0] <= 60), key=lambda p: p[1])
            x0, z0 = surf[0]
            x1, z1 = surf[-1]
            line = f"{x0:.1f}:{min(z0, crest_z) - 1.0:.2f};{crest_x:.1f}:{crest_z - 2.0:.2f};{x1:.1f}:{z1 - 0.5:.2f}"
            d = run.cmd(f"CIV_STABILITEIT LIJN={s['section']} MODEL=Dijkprofiel METHODE=fellenius "
                        f"FREATISCH={line} BELASTING=13:{crest_x - 2:.1f}:{crest_x + 2:.1f} NAAM=Dijkvak_belast")
            s["stab2"] = d
            f2, f1 = d["factor_of_safety"], s["stab"]["other_method"]["minimum"]
            us = [x["u"] for x in d["slices"]]
            return 0.2 < f2 < f1 and max(us) > 0 and d["load"]["q"] == 13, f"Fellenius F = {f2:.3f} (without load and line {f1:.3f}); max u {max(us):.1f} kPa; phreatic {line}"

        def stability_layers():
            # Layers given by hand: clay dike over soft clay, peat and Pleistocene sand.
            lay = "8:klei_zwak_zandig_matig;1:klei_schoon_slap;-2:veen_slap;-6:klei_zwak_zandig_slap;-12:zand_schoon_vast"
            d = run.cmd(f"CIV_STABILITEIT LIJN={s['section']} LAGEN={lay} BASIS=-25 GROND=veen_slap:c=2,phi=17 NAAM=Dijkvak_lagen")
            s["stab3"] = d
            units = d["units"]
            ok = "veen_slap" in units and units["veen_slap"]["c"] == 2 and units["veen_slap"]["phi"] == 17 and 0.2 < d["factor_of_safety"] < 10
            ok &= "own values" in units["veen_slap"]["source"] or "eigen waarden" in units["veen_slap"]["source"]
            return ok, f"Bishop F = {d['factor_of_safety']:.3f} with LAGEN={lay}; peat c' 2, φ' 17 ({units['veen_slap']['source']})"

        def polder():
            x, y, w, h = POLDER
            ocs.run(f"GIS_SITE RD {x:.0f} {y:.0f} {w:.0f} {h:.0f} Moordrecht polder")
            time.sleep(2)
            t = run.cmd("CIV_TIN BRON=AHN AFSTAND=2")
            b = run.cmd("GIS_Q_SONDERINGEN")
            c = [o for o in b["objecten"] if o["soort"] == "sondering" and (o["einddiepte_m"] or 0) > 5]
            a0, a1 = max(((p, q) for i, p in enumerate(c) for q in c[i + 1:]), key=lambda pq: math.dist((pq[0]["x"], pq[0]["y"]), (pq[1]["x"], pq[1]["y"])))
            (ax, ay), (bx, by) = (a0["x"], a0["y"]), (a1["x"], a1["y"])
            L = math.dist((ax, ay), (bx, by))
            ux, uy = (bx - ax) / L, (by - ay) / L
            p0, p1 = (ax - 20 * ux, ay - 20 * uy), (bx + 20 * ux, by + 20 * uy)
            a = run.cmd(f"CIV_AS PI={p0[0]:.2f},{p0[1]:.2f};{p1[0]:.2f},{p1[1]:.2f} NAAM=Polderprofiel")
            d = run.cmd(f"CIV_GRONDPROFIEL AS={a['as']} STRAAL=30 NAAM=Polderprofiel")
            s["polder_profile"] = d
            cols = d["model"]["columns"]
            # A 40 × 30 m fill on the middle of the CPT row.
            mx, my = (ax + bx) / 2, (ay + by) / 2
            nx, ny = uy, -ux
            s["fill"] = [(mx + p * ux + q * nx, my + p * uy + q * ny) for p, q in [(-20, -15), (20, -15), (20, 15), (-20, 15)]]
            s["fill_centre"] = (mx, my)
            ok = len(cols) >= 2 and t["driehoeken"] > 10000 and all(abs(c["layers"][0]["top"] - c["surface"]) < 1e-6 for c in cols)
            return ok, (f"{t['driehoeken']} triangles, {t['hoogte_min']:.2f} to {t['hoogte_max']:.2f} m NAP; {len(c)} CPTs deeper than 5 m; profile "
                        f"{len(cols)} columns, units {d['units']}; fill centre {mx:.0f}, {my:.0f}")

        def settlement():
            area = ";".join(f"{x:.2f},{y:.2f}" for x, y in s["fill"])
            c = s["fill_centre"]
            d = run.cmd(f"CIV_ZETTING GEBIED={area} HOOGTE=1.5 GAMMA=18 TIJD=30 CEL=2 PUNT={c[0]:.2f},{c[1]:.2f} MODEL=Polderprofiel")
            s["zet"] = d
            pt = d["point"]
            curve = [p["settlement_m"] for p in d["curve"]]
            mono = all(b >= a - 1e-9 for a, b in zip(curve, curve[1:]))
            layers = ocs.layers()
            ok = 0.02 < pt["total_m"] < 3 and mono and d["contours"] > 0 and any(l.startswith("OI-ZETTING-") for l in layers)
            ok &= abs(curve[-1] - pt["total_m"]) < 1e-6 and len(pt["layers"]) >= 2 and d["max_settlement_m"] >= pt["total_m"] - 1e-9
            ok &= any("HOOGTE_ZETTING-G" in l for l in layers)
            t90 = max((p["t90"] for p in pt["packages"]), default=0) / 365.25
            # Under the middle of a wide fill the stress just below the ground is the full load.
            top = pt["layers"][0]
            ok &= abs(top["dsigma"] - 27.0) < 3.0
            return ok, (f"Koppejan, 30 years: {pt['total_m'] * 1000:.0f} mm at the centre (primary {pt['primary_m'] * 1000:.0f} mm), max {d['max_settlement_m'] * 1000:.0f} mm; "
                        f"t90 {t90:.1f} years; {len(pt['layers'])} layers, {d['contours']} contours, {d['points']} points; soil {d['soil']}")

        def settlement_terzaghi():
            area = ";".join(f"{x:.2f},{y:.2f}" for x, y in s["fill"])
            c = s["fill_centre"]
            d = run.cmd(f"CIV_ZETTING GEBIED={area} HOOGTE=1.5 TIJD=30 CEL=2 PUNT={c[0]:.2f},{c[1]:.2f} METHODE=terzaghi MODEL=Polderprofiel BANDEN=4")
            s["zet2"] = d
            k = s["zet"]["point"]["primary_m"]
            t = d["point"]["primary_m"]
            # Both use one table: C'p and Cc/(1+e0) = ln 10 / C'p give the same primary settlement.
            same = d["point"]["x"] == s["zet"]["point"]["x"] and d["point"]["y"] == s["zet"]["point"]["y"]
            return same and abs(t - k) / max(k, 1e-6) < 0.05, f"Terzaghi total {d['point']['total_m'] * 1000:.0f} mm, primary {t * 1000:.0f} mm against Koppejan primary {k * 1000:.0f} mm (same table, ln 10/C'p = Cc/(1+e0))"

        def units():
            d = run.cmd("CIV_GRONDSOORTEN")
            rows = {r["code"]: r for r in d["units"]}
            r = rows["klei_schoon_slap"]
            return len(rows) == 26 and r["gamma"] == 14 and r["cp"] == 7, f"{len(rows)} units; klei_schoon_slap γ {r['gamma']}, φ' {r['phi']}, C'p {r['cp']}, C's {r['cs']}"

        def views(names_extents):
            ocs.run("-VIEW TOP")
            ocs.run("VSCURRENT WIREFRAME2D")
            shots = []
            for name, ext in names_extents:
                ok, p = run.capture(name, ext, 2000)
                shots.append((ok, p.name))
                okp, _ = run.plot(name, ext)
                shots.append((okp, f"{name}-plot.png"))
            return all(o for o, _ in shots), ", ".join(n + ("" if o else " (failed)") for o, n in shots)

        def captures_settlement():
            # Before the Terzaghi run draws its map over this one.
            return views([("zettingskaart", s["zet"]["map_extent"]), ("zetting-tijd", s["zet"]["graph_extent"]), ("zetting-legenda", s["zet"]["extent"])])

        def captures():
            ok, text = views([("grondprofiel", s["profile"]["extent"]), ("grondprofiel-polder", s["polder_profile"]["extent"]),
                              ("stabiliteit", s["stab"]["extent"]), ("stabiliteit-belast", s["stab2"]["extent"]), ("stabiliteit-lagen", s["stab3"]["extent"])])
            for name, (x, y, w, h) in [("plan-dijk", SITE), ("plan-polder", POLDER)]:
                o, p = run.capture(name, [x - w / 2, y - h / 2, x + w / 2, y + h / 2])
                ok &= o
                text += f", {p.name}"
            return ok, text

        def save():
            target = OUT / "geotechniek.dwg"
            if target.exists():
                target.unlink()
            r = ocs.call("/save", {"path": str(target), "document_id": ocs.doc()})
            t = time.time()
            while not target.exists() and time.time() - t < 60:
                time.sleep(0.5)
            if not target.exists():
                ocs.run("QSAVE")
                time.sleep(5)
                start = OUT / "start.dwg"
                if start.exists():
                    target.write_bytes(start.read_bytes())
            return target.exists() and target.stat().st_size > 10_000, f"{target} ({target.stat().st_size if target.exists() else 0} bytes; /save: {r.get('ok')})"

        run.step("site", site)
        run.step("terrain (AHN TIN)", terrain)
        run.step("BRO CPTs and boreholes", bro)
        run.step("profile line", line)
        run.step("soil profile", profile)
        run.step("stability (Bishop, soil model)", stability)
        run.step("stability (Fellenius, phreatic line, load)", stability_water_load)
        run.step("stability (given layers)", stability_layers)
        run.step("polder: TIN, BRO, soil profile, fill", polder)
        run.step("settlement of a 1.5 m fill (Koppejan)", settlement)
        run.step("captures of the settlement", captures_settlement)
        run.step("settlement (Terzaghi)", settlement_terzaghi)
        run.step("soil units", units)
        run.step("captures", captures)
        run.step("saved", save)
    finally:
        ocs.stop()
    lines = ["# Geotechnics e2e", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {d.replace('|', '/')} | {t:.1f} |" for n, ok, d, t in run.results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if all(ok for _, ok, _, _ in run.results) else 1)


if __name__ == "__main__":
    main()
