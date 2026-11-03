#!/usr/bin/env python3
"""End-to-end test van Open Infra in Open CAD Studio: data ophalen op
verschillende locaties, de tekening bewerken, opslaan en weer openen.

Start Open CAD Studio met een lege tekening en de REST-koppeling
(``OpenCADStudio start.dwg --http <poort>``), met de plugin uit de plugin-map.
Er verschijnt dus een venster; zonder venster verwerkt Open CAD Studio de
achtergrondverzoeken van de plugin alleen tijdens een plugin-commando.

Scenario's
  importeren   per locatie: projectlocatie, BGT, kadaster, AHN-hoogtelijnen en
               -hoogtepunten (NL) of OpenStreetMap (buiten NL), tekenblad.
               Gecontroleerd: aantal elementen, NLCS-lagen, alles binnen het
               importgebied, objectgegevens (XDATA) aanwezig.
  bewerken     een BGT-object verplaatsen (XDATA blijft), naar een andere laag
               zetten, een laag leeg maken, ongedaan maken / opnieuw, de
               laatste import ongedaan maken, opnieuw importeren (dubbel?).
  opslaan      opslaan, kopie openen en vergelijken (elementen, lagen, XDATA).
  eenheden     een mm-tekening: alles 1000 × zo groot.
  relatief     projectlocatie relatief: het basispunt op 0,0.
  verhuizen    in dezelfde tekening een tweede locatie importeren.

Gebruik
  python3 tests/e2e/ocs_e2e.py [--ocs PAD] [--locaties dordrecht,londen] [--snel]

Het rapport komt in target/e2e/rapport.md (en .json).
"""

import argparse
import json
import os
import secrets
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DATA = REPO / "tests" / "data"
OUT = REPO / "target" / "e2e"


class Ocs:
    """Een Open CAD Studio zonder venster, via REST."""

    def __init__(self, binary, port):
        self.port = port
        self.token = secrets.token_hex(16)
        env = dict(os.environ, OCS_API_TOKEN=self.token)
        self.log = open(OUT / "ocs.log", "w")
        # With a drawing on the command line Open CAD Studio opens its window
        # and keeps handling the plugin's background requests (imports run
        # on worker threads). Without a window (`--http` alone) it only does
        # so while it executes a plugin command, so imports would stall.
        start = OUT / "start.dwg"
        shutil.copy(DATA / "leeg-m.dwg", start)
        self.proc = subprocess.Popen([binary, str(start), "--http", str(port)], env=env, stdout=self.log, stderr=subprocess.STDOUT)
        for _ in range(90):
            time.sleep(0.5)
            try:
                if self.get("/ready").get("ready"):
                    # Startup questions (file association, donation) block commands.
                    for _ in range(3):
                        self.call("/cancel", {"document_id": self.doc()})
                    return
            except OSError:
                pass
        raise SystemExit("Open CAD Studio start niet (zie target/e2e/ocs.log)")

    def call(self, path, body=None, method=None, timeout=300):
        req = urllib.request.Request(
            f"http://127.0.0.1:{self.port}/api/v1{path}",
            data=None if body is None else json.dumps(body).encode(),
            method=method or ("POST" if body is not None else "GET"),
            headers={"Authorization": "Bearer " + self.token, "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read() or b"{}")
        except urllib.error.HTTPError as e:
            return json.loads(e.read() or b"{}")

    def get(self, path):
        return self.call(path)

    def doc(self):
        return self.get("/state")["document_id"]

    def run(self, cmd):
        r = self.call("/run", {"cmd": cmd, "document_id": self.doc()})
        if not r.get("ok"):
            raise RuntimeError(f"{cmd}: {r.get('error')}")
        return r

    def count(self):
        d = self.get("/capabilities")
        return {c["name"]: c["count"] for c in d["records"]["collections"]}

    def entities(self):
        return self.count()["entities"]

    def settle(self, minimum_seconds=2.0, timeout=600, first_change=300):
        """Wacht tot een import klaar is: eerst tot er iets verandert (een
        trage dienst als Overpass kan minuten nodig hebben), daarna tot het
        aantal elementen stilstaat."""
        start = time.time()
        begin = self.count()
        while time.time() - start < first_change:
            time.sleep(1.0)
            c = self.count()
            if (c["entities"], c["layers"]) != (begin["entities"], begin["layers"]):
                break
        last, stable = None, 0
        time.sleep(minimum_seconds)
        while time.time() - start < timeout:
            c = self.count()
            key = (c["entities"], c["layers"])
            stable = stable + 1 if key == last else 0
            last = key
            if stable >= 3:
                return c
            time.sleep(1.5)
        return self.count()

    def layers(self):
        return [l["name"] for l in self.get("/layers").get("layers", [])]

    def extents(self):
        v = self.get("/sysvars?names=extmin,extmax")["result"]["values"]
        return v["extmin"], v["extmax"]

    def in_bounds(self, b):
        r = self.get(f"/entities?bounds={b[0]},{b[1]},{b[2]},{b[3]}&limit=1&fields=handle")
        return r.get("count", 0)

    def first(self, layer):
        r = self.get(f"/entities?layer={urllib.request.quote(layer)}&limit=1&fields=handle,layer")
        e = r.get("entities") or []
        return e[0]["handle"] if e else None

    def handles(self, layer):
        r = self.get(f"/entities?layer={urllib.request.quote(layer)}&fields=handle")
        return [e["handle"] for e in r.get("entities", [])]

    def entity(self, handle):
        r = self.get(f"/entities/{handle}")
        e = r.get("entities") or [r.get("entity") or {}]
        return e[0]

    def xdata(self, handle):
        r = self.get(f"/entities/{handle}/xdata")
        items = r.get("items") or [{}]
        return items[0].get("xdata", {}).get("OPENINFRA", [])

    def open(self, path):
        r = self.call("/documents", {"path": str(path)})
        if not r.get("ok", True) and r.get("error"):
            raise RuntimeError(f"openen {path}: {r['error']}")
        time.sleep(1)

    def stop(self):
        self.proc.terminate()
        try:
            self.proc.wait(10)
        except subprocess.TimeoutExpired:
            self.proc.kill()


class Rapport:
    def __init__(self):
        self.regels = []

    def stap(self, scenario, locatie, naam, fn):
        t = time.time()
        try:
            ok, detail = fn()
            status = "ok" if ok else "fout"
        except Exception as e:  # het rapport wil alles zien
            status, detail = "fout", f"{type(e).__name__}: {e}"
        sec = time.time() - t
        teken = {"ok": "✔", "fout": "✘", "let op": "⚠"}.get(status, "?")
        if status == "ok" and str(detail).startswith("LET OP"):
            status, teken = "let op", "⚠"
        print(f"  {teken} {scenario:<10} {locatie:<11} {naam:<28} {sec:6.1f}s  {str(detail)[:110]}", flush=True)
        self.regels.append({"scenario": scenario, "locatie": locatie, "stap": naam, "status": status, "detail": str(detail), "seconden": round(sec, 1)})

    def schrijf(self):
        md = ["# End-to-end test Open Infra in Open CAD Studio", "", "| scenario | locatie | stap | status | tijd (s) | detail |", "|---|---|---|---|---|---|"]
        for r in self.regels:
            md.append(f"| {r['scenario']} | {r['locatie']} | {r['stap']} | {r['status']} | {r['seconden']} | {r['detail'].replace('|', '/')} |")
        fout = sum(r["status"] == "fout" for r in self.regels)
        letop = sum(r["status"] == "let op" for r in self.regels)
        md += ["", f"{len(self.regels)} stappen, {fout} fout, {letop} let op."]
        (OUT / "rapport.md").write_text("\n".join(md) + "\n")
        (OUT / "rapport.json").write_text(json.dumps(self.regels, indent=1, ensure_ascii=False))
        return fout


def locaties():
    return {l["id"]: l for l in json.loads((DATA / "locaties.json").read_text())["locaties"]}


def site_command(l, extra=""):
    return f"GIS_SITE {l['lon']} {l['lat']} {l['breedte']} {l['hoogte']} {l['crs']} {extra} {l['naam']}".replace("  ", " ")


def nieuwe_tekening(ocs, naam, leeg="leeg-m.dwg"):
    pad = OUT / f"{naam}.dwg"
    shutil.copy(DATA / leeg, pad)
    ocs.open(pad)
    return pad


def site_rect(ocs, factor=1.0):
    """Het importgebied in tekeningcoördinaten, uit de projectlocatie die de
    plugin in de DWG bewaart (XRecord OPENINFRA/SITE)."""
    rec = ocs.get("/records?collection=objects&limit=500")
    for r in rec.get("records", []):
        if r.get("type") == "XRecord" and r.get("properties", {}).get("name") == "SITE":
            site = json.loads(r["properties"]["entries"][0]["value"]["String"])
            w, h = site["width"] * factor / 2, site["height"] * factor / 2
            if not site.get("absolute", True):
                return [-w, -h, w, h]
            e, n = site["origin_e"] * factor, site["origin_n"] * factor
            return [e - w, n - h, e + w, n + h]
    raise RuntimeError("projectlocatie niet in de tekening gevonden")


def importeer(ocs, rap, l, cmd, label, minimaal=1):
    voor = ocs.count()

    def stap():
        ocs.run(cmd)
        na = ocs.settle()
        nieuw = na["entities"] - voor["entities"]
        lagen = na["layers"] - voor["layers"]
        return nieuw >= minimaal, f"+{nieuw} elementen, +{lagen} lagen"

    rap.stap("importeren", l["id"], label, stap)


def scenario_importeren(ocs, rap, l, nl):
    nieuwe_tekening(ocs, f"import-{l['id']}")
    rap.stap("importeren", l["id"], "projectlocatie", lambda: (ocs.run(site_command(l)) and True, "GIS_SITE"))
    time.sleep(1.5)
    if nl:
        importeer(ocs, rap, l, "GIS_Q_BGT", "BGT (NLCS)", 20)
        importeer(ocs, rap, l, "GIS_Q_BRK", "kadaster (NLCS)", 10)
        importeer(ocs, rap, l, "GIS_HOOGTELIJNEN", "AHN-hoogtelijnen", 1)
        importeer(ocs, rap, l, "GIS_HOOGTEPUNTEN", "AHN-hoogtepunten", 10)
    else:
        importeer(ocs, rap, l, "GIS_Q_OSM", "OpenStreetMap", 20)
    importeer(ocs, rap, l, "GIS_TEKENBLAD Locatietest", "tekenblad", 20)

    def controle():
        lagen = ocs.layers()
        nlcs = [n for n in lagen if n.count("-") >= 4 and n[0] in "BNTV" and n[1] == "-"]
        rect = site_rect(ocs)
        binnen = ocs.in_bounds([rect[0] - 1, rect[1] - 1, rect[2] + 1, rect[3] + 1])
        totaal = ocs.entities()
        h = ocs.first(next((n for n in nlcs if n.endswith("-G") and "-AL-" not in n), "0"))
        xd = ocs.xdata(h) if h else []
        bron = next((v for v in xd if v.startswith("bron=")), "geen XDATA")
        # Het tekenblad ligt deels buiten het importgebied; de data niet.
        ok = len(nlcs) >= (10 if nl else 5) and binnen >= totaal * 0.5 and (not nl or bron != "geen XDATA")
        return ok, f"{len(nlcs)} NLCS-lagen, {binnen}/{totaal} elementen binnen het importgebied, voorbeeld {bron}"

    rap.stap("importeren", l["id"], "controle lagen/gebied/XDATA", controle)


def scenario_bewerken(ocs, rap, l):
    laag = next((n for n in ocs.layers() if n.startswith("B-XX-VH-") and n.endswith("-G")), None)
    if laag is None:
        rap.stap("bewerken", l["id"], "BGT-laag gevonden", lambda: (False, "geen B-XX-VH-…-G laag"))
        return
    h = ocs.first(laag)

    def verplaatsen():
        voor = ocs.entity(h)
        xd = ocs.xdata(h)
        r = ocs.call("/entities/transform", {"handles": [h], "action": "move", "vector": [10.0, 0.0, 0.0], "document_id": ocs.doc()})
        na = ocs.entity(h)
        dx = na["bounds"]["min"][0] - voor["bounds"]["min"][0]
        ok = r.get("ok", True) and abs(dx - 10.0) < 1e-6 and ocs.xdata(h) == xd
        return ok, f"{dx:+.3f} m verschoven, XDATA {'behouden' if ocs.xdata(h) == xd else 'GEWIJZIGD'} ({len(xd)} waarden)"

    rap.stap("bewerken", l["id"], "object verplaatsen", verplaatsen)

    def andere_laag():
        r = ocs.call("/set_properties", {"collection": "entities", "handle": h, "updates": [{"path": "/common/layer", "value": "0"}], "document_id": ocs.doc()})
        e = ocs.entity(h)
        return e.get("layer") == "0", f"laag nu {e.get('layer')} ({r.get('error') or 'ok'})"

    rap.stap("bewerken", l["id"], "naar andere laag", andere_laag)

    def laag_leegmaken():
        hs = ocs.handles(laag)
        voor = ocs.entities()
        r = ocs.call("/entities", {"handles": hs, "document_id": ocs.doc()}, method="DELETE")
        na = ocs.entities()
        return na == voor - len(hs), f"{len(hs)} elementen van {laag} gewist ({r.get('error') or 'ok'})"

    rap.stap("bewerken", l["id"], "laag leegmaken", laag_leegmaken)

    def ongedaan_opnieuw():
        a = ocs.entities()
        ocs.run("U")
        time.sleep(0.5)
        b = ocs.entities()
        ocs.run("REDO")
        time.sleep(0.5)
        c = ocs.entities()
        return b > a and c == a, f"{a} → ongedaan {b} → opnieuw {c}"

    rap.stap("bewerken", l["id"], "ongedaan maken / opnieuw", ongedaan_opnieuw)

    def import_ongedaan():
        voor = ocs.entities()
        ocs.run("GIS_TEKENBLAD Tweede blad")
        na = ocs.settle()["entities"]
        ocs.run("U")
        time.sleep(0.5)
        terug = ocs.entities()
        if terug == voor:
            return True, f"tekenblad +{na - voor}, na ongedaan maken {terug - voor:+d}"
        # Bekend: Open CAD Studio v2026.42 legt elementen die een plugin op de
        # achtergrond toevoegt niet vast in de ongedaan-maakstap.
        return na > voor, f"LET OP: import niet ongedaan te maken (Open CAD Studio-host) — tekenblad +{na - voor}, na U {terug - voor:+d}"

    rap.stap("bewerken", l["id"], "import in één stap ongedaan", import_ongedaan)

    def opnieuw_importeren():
        voor = ocs.entities()
        lagen_voor = ocs.count()["layers"]
        ocs.run("GIS_Q_BGT")
        na = ocs.settle(first_change=60)
        extra = na["entities"] - voor
        detail = f"+{extra} elementen, +{na['layers'] - lagen_voor} lagen"
        if extra > 0:
            detail = "LET OP: opnieuw importeren voegt alles nogmaals toe — " + detail
        return na["layers"] == lagen_voor, detail

    rap.stap("bewerken", l["id"], "opnieuw importeren", opnieuw_importeren)


def scenario_opslaan(ocs, rap, l):
    def opslaan_en_openen():
        totaal, lagen = ocs.entities(), sorted(ocs.layers())
        laag = next((n for n in lagen if n.startswith("B-XX-OG-") and n.endswith("-G")), None) or next(n for n in lagen if n.startswith("B-"))
        h = ocs.first(laag)
        xd = ocs.xdata(h)
        ocs.run("QSAVE")
        time.sleep(2)
        pad = OUT / f"import-{l['id']}.dwg"
        kopie = OUT / f"heropend-{l['id']}.dwg"
        shutil.copy(pad, kopie)
        ocs.open(kopie)
        t2, l2 = ocs.entities(), sorted(ocs.layers())
        xd2 = ocs.xdata(h)
        ok = t2 == totaal and l2 == lagen and xd2 == xd
        return ok, f"{pad.stat().st_size // 1024} kB; elementen {totaal}→{t2}, lagen {len(lagen)}→{len(l2)}, XDATA {'gelijk' if xd2 == xd else 'ANDERS'}"

    rap.stap("opslaan", l["id"], "opslaan en heropenen", opslaan_en_openen)


def scenario_eenheden(ocs, rap, l):
    nieuwe_tekening(ocs, f"mm-{l['id']}", "leeg-mm.dwg")

    def mm():
        ocs.run(site_command(l))
        time.sleep(1.5)
        ocs.run("GIS_Q_BGT")
        ocs.settle()
        lo, hi = ocs.extents()
        breedte = hi[0] - lo[0]
        verwacht = l["breedte"] * 1000
        return abs(breedte / verwacht - 1) < 0.05, f"breedte {breedte:.0f} mm, verwacht ~{verwacht:.0f} mm"

    rap.stap("eenheden", l["id"], "mm-tekening", mm)


def scenario_relatief(ocs, rap, l):
    nieuwe_tekening(ocs, f"relatief-{l['id']}")

    def relatief():
        ocs.run(site_command(l, "RELATIEF"))
        time.sleep(1.5)
        ocs.run("GIS_Q_BGT")
        ocs.settle()
        lo, hi = ocs.extents()
        midden = ((lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2)
        ok = abs(midden[0]) < l["breedte"] and abs(midden[1]) < l["hoogte"]
        return ok, f"midden van de data op ({midden[0]:.1f}, {midden[1]:.1f}), basispunt 0,0"

    rap.stap("relatief", l["id"], "basispunt op 0,0", relatief)


def scenario_verhuizen(ocs, rap, van, naar):
    def verhuizen():
        ocs.run(site_command(naar))
        time.sleep(1.5)
        ocs.run("GIS_Q_BGT")
        ocs.settle()
        rect = site_rect(ocs)
        binnen = ocs.in_bounds(rect)
        return binnen > 20, f"{binnen} elementen bij {naar['naam']} in dezelfde tekening als {van['naam']}"

    rap.stap("verhuizen", van["id"], f"tweede locatie ({naar['id']})", verhuizen)


def default_ocs():
    """The Open CAD Studio this plugin is built for (v2026.44): the AppImage of
    the OpenAEC installer, else one on the PATH."""
    import glob
    found = sorted(glob.glob(str(Path.home() / ".local/share/openaec-installer/apps/open-cad-studio/OpenCADStudio-v2026.44*.AppImage")))
    if found:
        return found[-1]
    import shutil
    return shutil.which("OpenCADStudio") or "OpenCADStudio"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", default_ocs()))
    ap.add_argument("--poort", type=int, default=18740)
    ap.add_argument("--locaties", default="dordrecht,vaals,lelystad,londen")
    ap.add_argument("--snel", action="store_true", help="alleen Dordrecht, zonder buitenland")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    alle = locaties()
    ids = ["dordrecht"] if args.snel else args.locaties.split(",")
    ocs = Ocs(args.ocs, args.poort)
    rap = Rapport()
    # The edit, save and unit scenarios use BGT: on the first Dutch location.
    eerste_nl = next((i for i in ids if alle[i].get("verwacht", {}).get("ahn") is not None), None)
    try:
        for i in ids:
            l = alle[i]
            nl = l.get("verwacht", {}).get("ahn") is not None
            print(f"{i} — {l['naam']}", flush=True)
            scenario_importeren(ocs, rap, l, nl)
            if i == eerste_nl:
                scenario_bewerken(ocs, rap, l)
                scenario_opslaan(ocs, rap, l)
                scenario_eenheden(ocs, rap, l)
                scenario_relatief(ocs, rap, l)
                tweede = alle["amsterdam"]
                nieuwe_tekening(ocs, f"verhuizen-{l['id']}")
                ocs.run(site_command(l))
                time.sleep(1.5)
                ocs.run("GIS_Q_BGT")
                ocs.settle()
                scenario_verhuizen(ocs, rap, l, tweede)
    finally:
        ocs.stop()
    fout = rap.schrijf()
    print(f"\nRapport: {OUT / 'rapport.md'}")
    sys.exit(1 if fout else 0)


if __name__ == "__main__":
    main()
