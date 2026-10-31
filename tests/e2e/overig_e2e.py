#!/usr/bin/env python3
"""End-to-end test of survey codes, parcels, hydrology, reverse swept paths
and parking bays in Open CAD Studio.

Runs its own Open CAD Studio with an isolated configuration folder
(``XDG_CONFIG_HOME`` under target/e2e-overig), so the plugin installed for
daily use is left alone. A window appears, as with ``ocs_e2e.py``.

Steps (Dordrecht, around the Grote Kerk)
  codes        CIV_PUNTEN_IMPORT tests/data/landmeten/dordrecht-codes.csv CODES=JA:
               kerbs with arcs, a closed building, a hedge as 3D polylines on
               their NLCS layers; trees, manholes, gullies, lamp posts as
               symbol blocks; an unknown code reported. CIV_CODES_TOEPASSEN
               again replaces the linework instead of doubling it.
  parcels      a 2100 m² plot split by area (400 m²) perpendicular to a street,
               another into 4 equal parts parallel to 30°; CIV_PERCEEL_INFO.
  hydrology    a TIN from AHN, two water drops, catchments of two picked
               outlets on AHN and of the coded gullies on the TIN, ponds.
  swept path   a semitrailer reversing round a gentle arc (no jackknife) and
               a tight one (jackknife warning).
  parking      90° bays with a car checked in reverse, 45° bays, parallel
               bays with a car checked along a reversing S.
  captures     PNGs of each part in target/e2e-overig/.

Usage
  slot python3 tests/e2e/overig_e2e.py [--ocs PATH] [--port 18818]

The report goes to target/e2e-overig/rapport.md.
"""

import argparse
import json
import math
import os
import shutil
import subprocess
import sys
import time
import urllib.parse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ocs_e2e  # noqa: E402

REPO = ocs_e2e.REPO
OUT = REPO / "target" / "e2e-overig"
DATA = REPO / "tests" / "data"


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18818)
    ap.add_argument("--no-install", action="store_true")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.no_install:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    cfg = xdg / "OpenCADStudio" / "open-infra"
    cfg.mkdir(parents=True, exist_ok=True)
    # Dutch interface (the layer names below are the Dutch ones); a fresh
    # code table from the plugin.
    (cfg / "settings.json").write_text(json.dumps({"language": "nl"}))
    shutil.rmtree(cfg / "landmeten", ignore_errors=True)
    api = cfg / "api"
    ocs_e2e.OUT = OUT
    ocs = ocs_e2e.Ocs(args.ocs, args.port)
    results = []
    images = []

    def step(name, fn):
        t = time.time()
        try:
            ok, detail = fn()
        except Exception as e:  # noqa: BLE001
            ok, detail = False, f"{type(e).__name__}: {e}"
        results.append((name, ok, detail, time.time() - t))
        print(("✔" if ok else "✘"), name, "—", detail, flush=True)

    def cmd(line, rid, timeout=300):
        f = api / f"{rid}.json"
        if f.exists():
            f.unlink()
        ocs.run(f"{line} ID={rid}")
        t0 = time.time()
        while time.time() - t0 < timeout:
            if f.exists():
                time.sleep(0.2)
                out = json.loads(f.read_text())
                print(f"   {line.split()[0]}: {out.get('message')}", flush=True)
                return out
            time.sleep(0.5)
        raise TimeoutError(f"{line.split()[0]}: no answer in {timeout} s")

    def on_layer(layer, kind=None):
        r = ocs.get(f"/entities?layer={urllib.parse.quote(layer)}&fields=handle,type,layer")
        ents = r.get("entities", []) if isinstance(r, dict) else []
        if kind:
            ents = [e for e in ents if str(e.get("type", "")).lower().replace(" ", "").replace("_", "") in kind]
        return ents

    def create(points, closed, layer="0"):
        r = ocs.call("/entities_create", {"document_id": ocs.doc(), "entities": [{"type": "LwPolyline", "vertices": points, "closed": closed, "layer": layer}]})
        return r["result"]["handles"][0]

    def capture(name, region=None):
        path = OUT / f"{name}.png"
        if path.exists():
            path.unlink()
        body = {"path": str(path), "document_id": ocs.doc(), "max_dimension": 1600}
        if region:
            body.update({"view": "region", "bounds": region})
        ocs.call("/capture", body, timeout=180)
        t0 = time.time()
        while not (path.exists() and path.stat().st_size > 0) and time.time() - t0 < 60:
            time.sleep(0.25)
        if path.exists():
            images.append(path.name)
        return path.exists()

    def site():
        out = cmd("GIS_SITE RD 104885 425320 200 150 Dordrecht", "site", 60)
        return out["ok"], out["message"]

    # ── 1. Survey codes ──────────────────────────────────────────────────
    csv = DATA / "landmeten" / "dordrecht-codes.csv"

    def codes():
        out = cmd(f"CIV_PUNTEN_IMPORT {csv} CODES=JA", "codes", 120)
        d = out["data"]
        kerbs = on_layer("B-XX-VH-KANTOPSLUITING_TROTTOIRBAND-G")
        trees = on_layer("B-XX-GR-BOOM-S")
        gullies = on_layer("B-XX-RI-GWA_KOLK-S")
        building = on_layer("B-XX-OG-BEBOUWING-G")
        xd = ocs.xdata(kerbs[0]["handle"]) if kerbs else []
        ok = (
            out["ok"]
            and d.get("lijnen") == 5
            and d.get("symbolen") == 11
            and d.get("teksten") == 1
            and d.get("onbekende_codes") == {"XYZ": 1}
            and len(kerbs) == 2
            and len(trees) == 2
            and len(gullies) == 3
            and len(building) == 1
            and any("code=LTRB" in str(x) for x in xd)
        )
        e = ocs.entity(building[0]["handle"]) if building else {}
        closed = json.dumps(e).find('"closed": true') >= 0 or json.dumps(e).find('"closed":true') >= 0
        return ok, f"{d.get('lijnen')} lines, {d.get('symbolen')} symbols, {d.get('teksten')} text, unknown {d.get('onbekende_codes')}; kerbs {len(kerbs)}, trees {len(trees)}, gullies {len(gullies)}, building closed {closed}; kerb XDATA {xd[:3]}"

    def codes_again():
        before = len(on_layer("B-XX-VH-KANTOPSLUITING_TROTTOIRBAND-G"))
        out = cmd("CIV_CODES_TOEPASSEN", "codes2", 120)
        after = len(on_layer("B-XX-VH-KANTOPSLUITING_TROTTOIRBAND-G"))
        trees = len(on_layer("B-XX-GR-BOOM-S"))
        d = out["data"]
        return out["ok"] and before == after == 2 and trees == 2 and d.get("lijnen") == 5, f"kerbs {before} → {after}, trees {trees}: {out['message']}"

    # ── 2. Parcels ───────────────────────────────────────────────────────
    handles = {}

    def parcels():
        plot = create([[104900, 425330], [104960, 425330], [104960, 425360], [104900, 425370]], True)
        street = create([[104890, 425326], [104970, 425326]], False)
        handles.update(plot=plot, street=street)
        out = cmd(f"CIV_PERCELEN KAVEL={plot} STRAAT={street} OPP=400", "perc1", 120)
        rows = out["data"].get("percelen", [])
        areas = [r["oppervlakte_m2"] for r in rows]
        csv_file = Path(out["data"].get("csv", ""))
        ok = out["ok"] and len(rows) == 5 and all(abs(a - 400.0) <= 0.01 for a in areas[:4]) and abs(areas[-1] - 500.0) <= 0.01 and abs(sum(areas) - 2100.0) <= 0.02 and csv_file.is_file()
        handles["parcels"] = [r["handle"] for r in rows]
        layer = on_layer("N-XX-KG-GRENS_PERCEEL-G")
        return ok and len(layer) == 5, f"areas {areas}, fronts {[r['front_m'] for r in rows]}, CSV {csv_file.name}, {len(layer)} on N-XX-KG-GRENS_PERCEEL-G"

    def parcels_equal():
        plot = create([[104900, 425375], [104950, 425375], [104950, 425392], [104900, 425392]], True)
        out = cmd(f"CIV_PERCELEN KAVEL={plot} N=4 RICHTING=30 PREFIX=K START=11", "perc2", 120)
        rows = out["data"].get("percelen", [])
        areas = [r["oppervlakte_m2"] for r in rows]
        names = [r["perceel"] for r in rows]
        return out["ok"] and len(rows) == 4 and all(abs(a - 212.5) <= 0.01 for a in areas) and names[0] == "K11", f"{names}: {areas}"

    def parcel_info():
        hs = ";".join(handles.get("parcels", [])[:2])
        out = cmd(f"CIV_PERCEEL_INFO PERCEEL={hs} STRAAT={handles['street']}", "pinfo", 60)
        rows = out["data"].get("percelen", [])
        return out["ok"] and len(rows) == 2 and all(abs(r["oppervlakte_m2"] - 400.0) <= 0.01 and r["front_m"] and r["front_m"] > 5 for r in rows), out["message"]

    # ── 3. Hydrology ─────────────────────────────────────────────────────
    def tin():
        out = cmd("CIV_TIN BRON=AHN AFSTAND=2", "tin", 600)
        return out["ok"] and out["data"].get("driehoeken", 0) > 1000, out["message"]

    def drops():
        out = cmd("CIV_WATERDRUPPEL PUNTEN=104870,425340;104920,425280;104830,425310", "drop", 300)
        rows = out["data"].get("druppels", [])
        lines = on_layer("OI-WATERDRUPPEL", kind=("polyline3d", "polyline"))
        down = all(r["verval_m"] >= 0 for r in rows)
        return out["ok"] and len(rows) >= 2 and down and len(lines) >= 2, f"{[(r['lengte_m'], r['verval_m'], r['einde']) for r in rows]}; {len(lines)} 3D polylines"

    def catchments():
        out = cmd("CIV_STROOMGEBIED BRON=AHN CEL=1 SNAP=3 PUNTEN=104860,425300;104905,425345", "sg", 600)
        d = out["data"]
        sg = d.get("stroomgebieden", [])
        ponds = d.get("plassen", [])
        outlines = on_layer("OI-STROOMGEBIED")
        ok = out["ok"] and len(sg) == 2 and all(r["oppervlakte_m2"] > 0 for r in sg) and len(outlines) >= 2
        return ok, f"{[(r['stroomgebied'], r['oppervlakte_m2']) for r in sg]}; {len(ponds)} ponds (deepest {ponds[0]['diepte_m'] if ponds else '-'} m, {ponds[0]['volume_m3'] if ponds else '-'} m³)"

    def catchments_gullies():
        out = cmd("CIV_STROOMGEBIED BRON=TIN CEL=1 LAAG=*KOLK* PLASSEN=nee", "sg2", 600)
        sg = out["data"].get("stroomgebieden", [])
        skipped = out["data"].get("overgeslagen", [])
        return out["ok"] and len(sg) + len(skipped) == 3 and len(sg) >= 1, f"{[(r['afvoerpunt'], r['oppervlakte_m2']) for r in sg]} skipped {skipped}"

    def ponds_only():
        out = cmd("CIV_PLASSEN BRON=AHN CEL=1 MINDIEPTE=0.05 MINOPP=2", "ponds", 600)
        ponds = out["data"].get("plassen", [])
        layer = on_layer("OI-PLAS")
        return out["ok"] and all(p["diepte_m"] >= 0.05 and p["volume_m3"] > 0 for p in ponds), f"{len(ponds)} ponds, {len(layer)} entities on OI-PLAS; {out['message']}"

    # ── 4. Reverse swept paths ───────────────────────────────────────────
    def arc_path(x0, y0, r, sweep, lead):
        pts = [(x0 - lead, y0), (x0, y0)]
        for k in range(1, int(sweep) + 1):
            a = math.radians(k) - math.pi / 2
            pts.append((x0 + r * math.cos(a), y0 + r + r * math.sin(a)))
        return ";".join(f"{x:.3f},{y:.3f}" for x, y in pts)

    def swept_gentle():
        path = arc_path(104800, 425345, 30.0, 45, 20)
        before = len(on_layer("OI-VEEGPAD"))
        out = cmd(f"GIS_VEEGPAD eu_tractor_semitrailer_16_5 10 ACHTERUIT PAD={path}", "vp1", 120)
        d = out["data"]
        traces = len(on_layer("OI-VEEGPAD")) - before
        ok = out["ok"] and d.get("achteruit") is True and not d.get("scharen") and 0 < d.get("knikhoek_max_graden", 0) < d.get("knikhoek_grens_graden", 0) and traces == 8
        return ok, f"{traces} body traces; {out['message']}"

    def swept_jackknife():
        path = arc_path(104835, 425365, 5.0, 100, 15)
        out = cmd(f"GIS_VEEGPAD eu_tractor_semitrailer_16_5 10 ACHTERUIT PAD={path}", "vp2", 120)
        d = out["data"]
        return out["ok"] and d.get("scharen") is True, out["message"]

    # ── 5. Parking bays ──────────────────────────────────────────────────
    def parking_90():
        out = cmd("GIS_PARKEERVAKKEN HOEK=90 LIJN=104900,425260;104950,425260 MET_VEEGPAD=car_vw_golf", "pk1", 120)
        d = out["data"]
        bays = on_layer("OI-PARKEREN")
        c = d.get("controle") or {}
        return out["ok"] and d.get("vakken") == 40 and d.get("rijbaan_m") == 6.0 and len(bays) == 40 and c.get("past") is True, f"{len(bays)} bays; {out['message']}"

    def parking_45():
        out = cmd("GIS_PARKEERVAKKEN HOEK=45 KLASSE=nietopenbaar ZIJDE=links START=101 LIJN=104900,425285;104950,425285", "pk2", 120)
        d = out["data"]
        return out["ok"] and d.get("vakken", 0) >= 10 and d.get("rijbaan_m") == 3.0 and d.get("diepte_m") == 4.95, out["message"]

    def parking_parallel():
        out = cmd("GIS_PARKEERVAKKEN HOEK=LANGS ZIJDE=links START=201 LIJN=104800,425250;104850,425250 MET_VEEGPAD=car_vw_golf", "pk3", 120)
        d = out["data"]
        c = d.get("controle") or {}
        return out["ok"] and d.get("vakken") == 8 and c.get("achteruit") is True and c.get("past") is True, out["message"]

    def captures():
        ok = capture("overzicht")
        ok &= capture("codes", [104795, 425265, 104880, 425330])
        ok &= capture("percelen", [104885, 425320, 104975, 425395])
        ok &= capture("parkeren", [104795, 425240, 104960, 425295])
        ok &= capture("veegpad", [104770, 425335, 104870, 425395])
        return ok, ", ".join(images)

    step("site", site)
    step("codes to linework and symbols", codes)
    step("codes again (replace)", codes_again)
    step("parcels by area along a street", parcels)
    step("parcels equal, parallel to 30°", parcels_equal)
    step("parcel info", parcel_info)
    step("parking 90° with swept check", parking_90)
    step("parking 45°", parking_45)
    step("parking parallel with swept check", parking_parallel)
    step("swept path reversing, gentle", swept_gentle)
    step("swept path reversing, jackknife", swept_jackknife)
    step("TIN from AHN", tin)
    step("water drops", drops)
    step("catchments on AHN", catchments)
    step("catchments of the coded gullies on the TIN", catchments_gullies)
    step("ponds", ponds_only)
    step("captures", captures)
    try:
        ocs.run(f"QSAVEAS {OUT / 'overig.dwg'}")
        time.sleep(3)
    except Exception:  # noqa: BLE001
        pass
    ocs.stop()
    lines = ["# Survey codes, parcels, hydrology, swept paths and parking e2e", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {d} | {t:.1f} |" for n, ok, d, t in results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if all(ok for _, ok, _, _ in results) else 1)


if __name__ == "__main__":
    main()
