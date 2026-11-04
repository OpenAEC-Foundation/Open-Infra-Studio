#!/usr/bin/env python3
"""Trial trench (proefsleuf) workflow, end to end in a real Open CAD Studio:
KLIC + survey + AHN → cross sections that show what you will meet.

  1. the project location in the centre of Dordrecht and the BGT;
  2. the KLIC sample delivery (GIS2BIM, Dordrecht 2021);
  3. a trench axis along the longest straight stretch (15–40 m) of a
     low-voltage cable of that delivery (found in the GI-xml, so the cross
     sections cross it);
  4. "measured" points: the cable found in the trench at each section,
     imported as a PENZD file (CIV_PUNTEN_IMPORT);
  5. CIV_DWARSPROFIELEN over AHN every 5 m, 6 m either side: each section
     must show the BGT surfaces it cuts, the KLIC crossings at their assumed
     depth and the measured points;
  6. the dependency graph: a length profile too, then a TIN changes the
     ground of both; CIV_BIJWERKEN must find and remake exactly those two;
  7. the objects with their data as GeoPackage (GIS_EXPORT).

Runs its own Open CAD Studio with an isolated configuration folder, like the
other e2e tests:

    python3 tests/e2e/proefsleuf_e2e.py [--ocs PATH] [--port 18822] [--no-install] [--klic ZIP]

The report goes to target/e2e-proefsleuf/rapport.md.
"""

import argparse
import json
import math
import os
import re
import subprocess
import sys
import time
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ocs_e2e  # noqa: E402

REPO = ocs_e2e.REPO
OUT = REPO / "target" / "e2e-proefsleuf"
KLIC = Path.home() / "Documenten/GIS2BIM/supportfiles/Sandbox/KLIC/Levering_21O072584_1.zip"
CENTRE, SIZE = (104885.0, 425330.0), (400.0, 300.0)


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


def cable_in_area(zip_path):
    """The longest straight stretch (at least 15 m, at most 40 m) of a
    low-voltage cable inside the project area: (start, end)."""
    with zipfile.ZipFile(zip_path) as z:
        name = next(n for n in z.namelist() if re.search(r"GI_gebiedsinformatielevering.*\.xml$", n))
        xml = z.read(name).decode("utf-8")
    x0, y0 = CENTRE[0] - SIZE[0] / 2 + 30, CENTRE[1] - SIZE[1] / 2 + 30
    x1, y1 = CENTRE[0] + SIZE[0] / 2 - 30, CENTRE[1] + SIZE[1] / 2 - 30
    best = None
    # IMKL keeps a cable's geometry in the UtilityLink its net:link points to.
    links = set()
    for block in re.findall(r"<imkl:Elektriciteitskabel\b.*?</imkl:Elektriciteitskabel>", xml, re.S):
        links.update(re.findall(r'<net:link xlink:href="([^"]+)"', block))
    for m in re.finditer(r'<us-net-common:UtilityLink gml:id="([^"]+)".*?</us-net-common:UtilityLink>', xml, re.S):
        gid, block = m.group(1), m.group(0)
        if gid not in links:
            continue
        for pos in re.findall(r"<gml:posList[^>]*>([^<]+)</gml:posList>", block):
            v = [float(t) for t in pos.split()]
            pts = list(zip(v[0::2], v[1::2]))
            for a, b in zip(pts, pts[1:]):
                if all(x0 < p[0] < x1 and y0 < p[1] < y1 for p in (a, b)) and (best is None or math.dist(a, b) > math.dist(*best)):
                    best = (a, b)
    if best is None or math.dist(*best) < 15:
        return None
    a, b = best
    n = min(math.dist(a, b), 40.0)
    ux, uy = (b[0] - a[0]) / math.dist(a, b), (b[1] - a[1]) / math.dist(a, b)
    return a, (a[0] + n * ux, a[1] + n * uy)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18822)
    ap.add_argument("--no-install", action="store_true", help="use the plugins already in the isolated folder")
    ap.add_argument("--klic", default=str(KLIC))
    args = ap.parse_args()
    if not Path(args.klic).exists():
        sys.exit(f"KLIC sample delivery not found: {args.klic}")
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.no_install:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    cfg = xdg / "OpenCADStudio" / "open-infra"
    cfg.mkdir(parents=True, exist_ok=True)
    (cfg / "settings.json").write_text(json.dumps({"language": "nl"}))
    api_dir = cfg / "api"
    ocs_e2e.OUT = OUT
    ocs = ocs_e2e.Ocs(args.ocs, args.port)
    results, state = [], {}

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

    def site():
        r = cmd(f"GIS_SITE RD {CENTRE[0]} {CENTRE[1]} {SIZE[0]:.0f} {SIZE[1]:.0f} Proefsleuf Dordrecht", "site", 60)
        return r["ok"], r["message"][:120]

    def bgt():
        r = cmd("GIS_Q_BGT", "bgt", 600)
        return r["ok"], r["message"][:120]

    def klic():
        r = cmd(f"GIS_KLIC_IMPORT {args.klic}", "klic", 600)
        return r["ok"] and "KLIC" in r["message"], r["message"][:160]

    def trench():
        found = cable_in_area(args.klic)
        if not found:
            return False, "no straight low-voltage cable of 15 m or more in the area"
        a, b = found
        state["axis_pts"] = (a, b)
        r = cmd(f"CIV_AS PI={a[0]:.2f},{a[1]:.2f};{b[0]:.2f},{b[1]:.2f} NAAM=Proefsleuf", "as")
        if not r["ok"]:
            return False, r["message"]
        state["as"] = r["data"]["as"]
        return True, f"as {state['as']}, {math.dist(a, b):.1f} m along the cable from {a[0]:.1f},{a[1]:.1f}"

    def survey():
        a, b = state["axis_pts"]
        n = math.dist(a, b)
        ux, uy = (b[0] - a[0]) / n, (b[1] - a[1]) / n
        stations = [i * 5.0 for i in range(int(n // 5) + 1)]
        # Found about 0,6 m below the ground (which is about +2,6 m NAP there).
        rows = [f"{i + 1},{a[0] + ux * s:.3f},{a[1] + uy * s:.3f},{2.0:.3f},KABEL_LS" for i, s in enumerate(stations)]
        path = OUT / "proefsleuf-meting.csv"
        path.write_text("\n".join(rows) + "\n")
        r = cmd(f"CIV_PUNTEN_IMPORT {path}", "meting")
        return r["ok"], r["message"][:120]

    def sections():
        r = cmd(f"CIV_DWARSPROFIELEN AS={state['as']} OM=5 LINKS=6 RECHTS=6", "dp", 900)
        if not r["ok"]:
            return False, r["message"]
        d = r["data"]
        state["dp"] = d
        n, bgt_cut, klic_cut, measured = d.get("dwarsprofielen", 0), d.get("gesneden_bgt", 0), d.get("klic_kruisingen", 0), d.get("meetpunten_in_profiel", 0)
        ok = n >= 4 and bgt_cut > 0 and klic_cut >= n and measured >= n - 1
        return ok, f"{n} sections, {bgt_cut} BGT surface parts, {klic_cut} KLIC crossings, {measured} measured points"

    def graph():
        """The dependency graph: a new TIN changes the ground of both profiles."""
        r = cmd(f"CIV_LENGTEPROFIEL AS={state['as']}", "lp", 600)
        if not r["ok"]:
            return False, r["message"]
        before = cmd("CIV_BIJWERKEN CONTROLE", "graph0", 120)
        r = cmd("CIV_TIN BRON=AHN AFSTAND=5", "tin", 600)
        if not r["ok"]:
            return False, r["message"]
        stale = cmd("CIV_BIJWERKEN CONTROLE", "graph1", 120)
        update = cmd("CIV_BIJWERKEN", "graph2", 900)
        after = cmd("CIV_BIJWERKEN CONTROLE", "graph3", 120)
        n = lambda r: (r.get("data") or {}).get("verouderd")
        ok = n(before) == 0 and n(stale) == 2 and update["ok"] and n(after) == 0
        return ok, f"stale before {n(before)}, after a TIN {n(stale)}, updated: {update['message'][:90]}, after {n(after)}"

    def export():
        path = OUT / "proefsleuf.gpkg"
        r = cmd(f"GIS_EXPORT {path}", "export", 300)
        return r["ok"] and path.exists(), r["message"][:160]

    def shot(name, bounds=None):
        p = OUT / name
        if p.exists():
            p.unlink()
        body = {"path": str(p), "document_id": ocs.doc(), "max_dimension": 1800}
        if bounds:
            body.update({"view": "region", "bounds": bounds})
        ocs.call("/capture", body, timeout=180)
        t0 = time.time()
        while not (p.exists() and p.stat().st_size > 0) and time.time() - t0 < 60:
            time.sleep(0.25)
        return p.exists()

    def captures():
        ocs.run("-VIEW TOP")
        ocs.run("VSCURRENT WIREFRAME2D")
        ocs.run("ZOOM EXTENTS")
        time.sleep(2)
        made = ["proefsleuf.png" if shot("proefsleuf.png") else "proefsleuf✘"]
        at = state.get("dp", {}).get("plaats")
        if at:
            made.append("dwarsprofielen.png" if shot("dwarsprofielen.png", [at[0] - 5, at[1] - 75, at[0] + 145, at[1] + 12]) else "dwarsprofielen✘")
            made.append("dwarsprofiel-0.png" if shot("dwarsprofiel-0.png", [at[0] + 8, at[1] - 22, at[0] + 40, at[1] - 2]) else "dwarsprofiel-0✘")
        return all("✘" not in m for m in made), ", ".join(made)

    if step("site", site) and step("BGT", bgt) and step("KLIC delivery", klic) and step("trench axis along a cable", trench):
        step("survey points", survey)
        step("cross sections with cut objects", sections)
        step("dependency graph (CIV_BIJWERKEN)", graph)
        step("GeoPackage export", export)
        step("capture", captures)
    try:
        ocs.run(f"QSAVEAS {OUT / 'proefsleuf.dwg'}")
        time.sleep(3)
    except Exception:  # noqa: BLE001
        pass
    ocs.stop()
    lines = ["# Trial trench e2e (KLIC + survey + AHN → cross sections)", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {d} | {t:.1f} |" for n, ok, d, t in results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if results and all(ok for _, ok, _, _ in results) else 1)


if __name__ == "__main__":
    main()
