#!/usr/bin/env python3
"""BGT mutation (GIS_BGT_MUTATIE) end to end, in a real Open CAD Studio.

Starts Open CAD Studio with its own config folder (the plugins from
target/release), imports the BGT around the Grote Kerk in Dordrecht and
makes a design the way a designer would:

  - the largest road part is resurfaced: the BGT object goes to layer V and a
    new asphalt surface (layer R, no object data) takes its place;
  - a second road part is changed as built (layer R);
  - a planned cycle path (layer N) with its function in the object data.

Then GIS_BGT_MUTATIE must give 2 new (one plan topography), 1 changed and
1 ended object, the asphalt must take the function of the road beneath, and
with --xsd the GML must be valid against IMGeo 2.1.1 (needs lxml; the
CityGML and GML schemas are fetched once into target/e2e/mutatie/xsd).

    cargo build --release -p open-infra && cargo build --release -p open-infra-civiel
    python3 tests/e2e/bgt_mutatie_e2e.py [--ocs <AppImage>] [--xsd]
"""

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from ocs_e2e import DATA, default_ocs

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "e2e" / "mutatie"
IMGEO_XSD = "https://schemas.geonovum.nl/imgeo/2.1/imgeo-2.1.1.xsd"


def install(cfg):
    shutil.rmtree(cfg, ignore_errors=True)
    rustc = subprocess.run(["rustc", "--version"], capture_output=True, text=True).stdout.strip()
    lock = (ROOT / "Cargo.lock").read_text()
    # The host's opencadcodec (git); pointcloud-core brings a crates.io acadrust too.
    acad = next(b.split('source = "', 1)[1].split('"', 1)[0] for b in lock.split("[[package]]") if 'name = "acadrust"' in b and "opencadcodec" in b)
    for pid, lib, toml in (("openaec.open-infra", "libopen_infra.so", ROOT / "plugin.toml"),
                           ("openaec.open-infra-civiel", "libopen_infra_civiel.so", ROOT / "civiel" / "plugin.toml")):
        d = cfg / "OpenCADStudio" / "plugins" / pid
        d.mkdir(parents=True)
        shutil.copy2(ROOT / "target" / "release" / lib, d / lib)
        (d / "plugin.toml").write_text(toml.read_text().replace("__RUSTC_VERSION__", rustc).replace("__ACADRUST_SOURCE__", acad))


def object_data(e):
    recs = e["properties"]["common"]["extended_data"]["records"]
    return dict(v["String"].split("=", 1) for r in recs if r["application_name"] == "OPENINFRA" for v in r["values"] if "String" in v and "=" in v["String"])


def area(e):
    v = [(p["location"]["x"], p["location"]["y"]) for p in e["properties"].get("vertices", [])]
    return abs(sum(v[i][0] * v[(i + 1) % len(v)][1] - v[(i + 1) % len(v)][0] * v[i][1] for i in range(len(v)))) / 2 if len(v) > 2 else 0


def validate(gml):
    """IMGeo 2.1.1 schema check; imported schemas cached in OUT/xsd."""
    from lxml import etree
    cache = OUT / "xsd"
    cache.mkdir(exist_ok=True)

    class Fetch(etree.Resolver):
        def resolve(self, url, pubid, ctx):
            if not url.startswith("http"):
                return None
            f = cache / (hashlib.md5(url.encode()).hexdigest() + ".xsd")
            if not f.exists():
                u = url.replace("http://", "https://", 1)
                f.write_bytes(urllib.request.urlopen(urllib.request.Request(u, headers={"User-Agent": "open-infra-e2e"}), timeout=60).read())
            return self.resolve_string(f.read_bytes(), ctx, base_url=url)

    parser = etree.XMLParser()
    parser.resolvers.add(Fetch())
    schema = etree.XMLSchema(etree.parse(IMGEO_XSD, parser))
    ok = schema.validate(etree.parse(str(gml)))
    return ok, [f"r{e.line}: {e.message}" for e in list(schema.error_log)[:10]]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ocs", help="Open CAD Studio (AppImage of binary)")
    ap.add_argument("--xsd", action="store_true", help="also check the GML against the IMGeo 2.1.1 schema")
    a = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    cfg = OUT / "cfg"
    install(cfg)
    dwg = OUT / "mutatie.dwg"
    shutil.copy2(DATA / "leeg-m.dwg", dwg)
    os.environ["XDG_CONFIG_HOME"] = str(cfg)
    ocs = subprocess.Popen([a.ocs or default_ocs(), str(dwg)], stdout=open(OUT / "ocs.log", "w"), stderr=subprocess.STDOUT, start_new_session=True)
    sys.path.insert(0, str(ROOT / "mcp"))
    import open_infra_mcp as mcp  # after XDG_CONFIG_HOME
    failures = []

    def check(cond, what):
        print(("OK   " if cond else "FOUT ") + what, flush=True)
        if not cond:
            failures.append(what)

    try:
        for _ in range(90):
            try:
                mcp.OCS.pick()
                d = mcp.OCS.document_id()
                break
            except Exception:
                time.sleep(1)
        time.sleep(3)
        print(mcp.OCS.run("GIS_SITE RD 104900 425350 160 120 Mutatietest")["message"])
        print(mcp.OCS.run("GIS_Q_BGT", timeout=300)["message"])
        ents = mcp.OCS.request("query", {"document_id": d, "detail": "full", "limit": 5000})["entities"]
        roads = sorted([e for e in ents if object_data(e).get("objecttype") == "wegdeel" and e["properties"].get("is_closed")], key=area, reverse=True)
        check(len(roads) >= 2, f"{len(roads)} wegdelen uit de BGT")
        w1, w2 = roads[0], roads[1]
        relayer = lambda e, status: mcp.OCS.request("set_properties", {"document_id": d, "collection": "entities", "handle": e["handle"], "updates": [{"path": "/common/layer", "value": status + e["layer"][1:]}]})
        relayer(w1, "V")
        verts = [[p["location"]["x"], p["location"]["y"]] for p in w1["properties"]["vertices"]]
        mcp.OCS.request("entities_create", {"document_id": d, "entities": [{"type": "LwPolyline", "vertices": verts, "closed": True, "layer": "R-XX-VH-GESLOTENVERHARDING_ASFALT-G"}]})
        relayer(w2, "R")
        b = w2["bounds"]
        cx, cy = (b["min"][0] + b["max"][0]) / 2, b["max"][1] + 3
        made = mcp.OCS.request("entities_create", {"document_id": d, "entities": [{"type": "LwPolyline", "vertices": [[cx - 10, cy], [cx + 10, cy], [cx + 10, cy + 2.5], [cx - 10, cy + 2.5]], "closed": True, "layer": "N-XX-VH-GESLOTENVERHARDING_ASFALT-G"}]})
        path_handle = made["result"]["handles"][0]
        mcp.OCS.request("xdata_set", {"document_id": d, "app": "OPENINFRA", "handles": [path_handle], "data": [{"code": 1000, "value": "objecttype=wegdeel"}, {"code": 1000, "value": "functie=fietspad"}]})
        out = mcp.OCS.run("GIS_BGT_MUTATIE", timeout=300)
        print(out["message"])
        data = out.get("data") or {}
        check(out["ok"], "GIS_BGT_MUTATIE geeft een resultaat")
        check((data.get("nieuw"), data.get("gewijzigd"), data.get("beeindigd"), data.get("fouten")) == (2, 1, 1, 0), f"2 nieuw, 1 gewijzigd, 1 beëindigd, 0 fouten (kreeg {data.get('nieuw')}/{data.get('gewijzigd')}/{data.get('beeindigd')}/{data.get('fouten')})")
        gml = Path(data.get("gml", ""))
        text = gml.read_text() if gml.is_file() else ""
        functie = object_data(w1).get("functie", "")
        check(text.count(f">{functie}</tra:function>") >= 2, f"het nieuwe asfalt neemt de functie '{functie}' van de weg eronder over")
        check("<imgeo:plus-status codeSpace=\"http://www.geostandaarden.nl/imgeo/def/2.1#StatusPlus\">plan</imgeo:plus-status>" in text, "het fietspad (laag N) is plantopografie")
        check(f"<imgeo:lokaalID>{object_data(w1)['lokaal_id']}</imgeo:lokaalID>" in text and "<cit:terminationDate>" in text, "het beëindigde wegdeel houdt zijn lokaalID")
        if a.xsd:
            ok, errors = validate(gml)
            check(ok, "GML geldig volgens IMGeo 2.1.1" + ("" if ok else ": " + "; ".join(errors)))
    finally:
        try:
            os.killpg(ocs.pid, 15)
        except ProcessLookupError:
            pass
    print(f"\n{'Alles goed' if not failures else f'{len(failures)} fout(en)'}; uitvoer in {OUT}")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
