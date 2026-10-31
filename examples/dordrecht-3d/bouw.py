"""Build the Dordrecht example: plan (BGT, cadastre, KLIC on NLCS layers)
and 3D model (BGT in 3D, 3D BAG, KLIC in 3D, BRO CPTs and boreholes, a
street design with axis, profiles and corridor) in one drawing. Saves it as
DWG, exports it as IFC 4.3 and captures the plan and 3D views.

    python examples/dordrecht-3d/bouw.py --klic <Levering_21O072584_1.zip> [--config <map>] [--ocs <OpenCADStudio>]
    python examples/dordrecht-3d/bouw.py --beelden [--backend vulkan]      # only the views of the saved drawing

The drawing is saved and exported before the views are captured: a large
3D model can bring the GPU renderer down while capturing, and the model
must not be lost with it.

It starts its own Open CAD Studio window (--new-instance, REST API on
--port), so other windows stay untouched. The Open Infra plugins come from
<config>/OpenCADStudio/plugins (scripts/install.sh). Needs the internet:
PDOK, 3D BAG, BRO and AHN.
"""
import argparse
import json
import os
import pathlib
import secrets
import shutil
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
NAME = "dordrecht-3d"
SITE = "GIS_SITE RD 104980 425430 700 560 Dordrecht binnenstad"
# The street of the design: the NWB road section between these points (RD).
STREET = ((104909, 425541), (105110, 425705))
# The detail view: around the Grote Kerk (RD).
KERK = (104780.0, 425240.0, 105000.0, 425420.0)


class Session:
    """My own Open CAD Studio window, driven over REST."""

    def __init__(self, a, dwg):
        self.port = a.port
        self.token = secrets.token_hex(16)
        os.environ.update(OCS_API_TOKEN=self.token, OI_REST_PORT=str(a.port))
        sys.path.insert(0, str(ROOT / "mcp"))
        import open_infra_mcp as mcp

        self.mcp = mcp
        self.ocs = mcp.OCS
        # A large drawing: let the plugin read it whole (IFC export).
        env = dict(os.environ, OCS_V4_SNAPSHOT_SEGMENT_SIZE="268435456")
        args = [a.ocs, "--new-instance", str(dwg), "--http", str(a.port)] + (["--backend", a.backend] if a.backend else [])
        self.proc = subprocess.Popen(args, env=env, stdout=open(HERE / "ocs.log", "w"), stderr=subprocess.STDOUT)
        for _ in range(180):
            time.sleep(1)
            try:
                if mcp._rest(self.port, self.token, "/ready", timeout=5).get("ready"):
                    break
            except OSError:
                pass
        else:
            raise SystemExit("Open CAD Studio start niet (zie ocs.log)")
        # Startup questions (file association, donation) block commands.
        for _ in range(3):
            self.ocs.request("cancel", {"document_id": self.ocs.document_id()})

    def run(self, cmd, timeout=1800):
        t0 = time.time()
        r = self.ocs.run(cmd, timeout=timeout)
        print(f"{cmd.split()[0]} ({time.time() - t0:.0f} s): {r.get('ok')} {str(r.get('message'))[:200]}", flush=True)
        return r

    def host(self, cmd):
        return self.ocs.host(cmd)

    def view(self, preset, style):
        self.host(f"-VIEW {preset}")
        self.host(f"VSCURRENT {style}")
        self.host("ZOOM EXTENTS")

    def capture(self, name, preset, style, bounds=None, maxdim=2400):
        path = HERE / name
        try:
            self.view(preset, style)
            time.sleep(3)
            if path.exists():
                path.unlink()
            body = {"path": str(path), "document_id": self.ocs.document_id(), "max_dimension": maxdim}
            if bounds:
                body.update({"view": "region", "bounds": list(bounds)})
            self.ocs.request("capture", body, timeout=180)
            for _ in range(60):
                if path.exists() and path.stat().st_size > 0:
                    break
                time.sleep(1)
            time.sleep(1)
            crop(path)
            print(f"{name}: {path.stat().st_size if path.exists() else 0} bytes", flush=True)
        except Exception as e:
            print(f"{name}: niet gelukt ({e})", flush=True)

    def save(self, dwg):
        before = dwg.stat().st_mtime
        self.host("QSAVE")
        for _ in range(120):
            if dwg.stat().st_mtime != before:
                break
            time.sleep(1)
        time.sleep(5)
        print(f"{dwg.name}: {dwg.stat().st_size / 1e6:.1f} MB", flush=True)

    def street(self):
        """The handle of the NWB road section of the design."""
        d = self.ocs.document_id()
        found = self.ocs.request("query", {"document_id": d, "layer": "GIS-WEGVAKKEN", "detail": "full", "limit": 5000}).get("entities")
        if found is None:
            found = self.mcp._rest(self.port, self.token, "/entities?layer=GIS-WEGVAKKEN&detail=full&limit=5000").get("entities", [])
        for e in found:
            v = e.get("properties", {}).get("vertices", [])
            pts = [(p["location"]["x"], p["location"]["y"]) if "location" in p else (p["position"]["x"], p["position"]["y"]) for p in v]
            if len(pts) >= 2 and all(any(abs(x - sx) < 3 and abs(y - sy) < 3 for x, y in (pts[0], pts[-1])) for sx, sy in STREET):
                return e["handle"]
        raise SystemExit("de ontwerpstraat (NWB-wegvak) is niet gevonden")

    def close(self):
        self.proc.kill()


def build(s, klic, dwg):
    s.run(SITE)
    # Plan: base data on NLCS layers.
    s.run("GIS_Q_BGT")
    s.run("GIS_Q_BRK")
    s.run(f'GIS_KLIC_IMPORT "{klic}"')
    # 3D model.
    s.run("CIV_BGT3D PANDEN=nee")
    s.run("CIV_BAG3D RAMEN=nee")
    s.run(f'CIV_KLIC3D "{klic}" VERGROOT=2')
    s.run("CIV_SONDERINGEN3D MAX=40")
    # Street design: axis on an NWB road section, terrain, profiles, corridor.
    s.run("GIS_Q_NWB")
    street = s.street()
    s.run("CIV_TIN BRON=AHN AFSTAND=5")
    axis = s.run(f"CIV_AS AS={street} NAAM=Ontwerpstraat")["data"]
    ax, length = axis["as"], float(axis["lengte_m"])
    frame = s.run(f"CIV_LENGTEPROFIEL AS={ax}")["data"]
    z = (float(frame["maaiveld_min"]) + float(frame["maaiveld_max"])) / 2.0
    s.run(f"CIV_ONTWERPPROFIEL AS={ax} PVI=0:{z + 0.3:.2f};{length / 2:.1f}:{z + 0.6:.2f};{length:.1f}:{z + 0.3:.2f} BOOG=40")
    s.run(f"CIV_CORRIDOR AS={ax} SJABLOON=weg FREQUENTIE=10 DWARSPROFIELEN=ja")
    s.view("TOP", "WIREFRAME2D")
    s.save(dwg)
    ifc = HERE / (NAME + ".ifc")
    data = s.run(f'CIV_IFC BESTAND="{ifc}"').get("data", {})
    data["bestand"] = ifc.name
    (HERE / "ifc-rapport.json").write_text(json.dumps(data, indent=1, ensure_ascii=False), encoding="utf-8")
    # The IFC in the repository: zipped (ifcZIP), a fifth of the size.
    import zipfile

    with zipfile.ZipFile(HERE / (NAME + ".ifczip"), "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        z.write(ifc, ifc.name)


def crop(path):
    """Cut the viewport's toolbar (top) and command line (bottom) off."""
    try:
        from PIL import Image
    except ImportError:
        return
    im = Image.open(path)
    w, h = im.size
    im.crop((0, round(h * 0.035), w, round(h * 0.95))).save(path)


def views(s):
    """The plan from above, the 3D model, and beneath a see-through ground:
    without screen furniture and cursor, and without the TIN, which is a
    work surface for the design."""
    s.host("CLEANSCREEN")
    s.host("UCSICON OFF")
    s.host("-LAYER F B-XX-OG-TRIANGULATIE-G")
    s.ocs.request("cancel", {"document_id": s.ocs.document_id()})
    try:
        s.ocs.request("pointer", {"document_id": s.ocs.document_id(), "events": [{"move": [-50, -50]}, {"leave": True}]})
    except Exception:
        pass
    s.capture(f"{NAME}-plattegrond.png", "TOP", "GOURAUDSHADED")
    s.capture(f"{NAME}-3d.png", "NEISO", "GOURAUDSHADED")
    s.run("CIV_DOORZICHT PCT=78")
    s.capture(f"{NAME}-3d-ondergrond.png", "NEISO", "GOURAUDSHADED")
    s.capture(f"{NAME}-ondergrond-grote-kerk.png", "NEISO", "GOURAUDSHADED", bounds=KERK)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--klic", help="a KLIC delivery (zip) for the area")
    ap.add_argument("--beelden", action="store_true", help="only capture the views of the saved drawing")
    ap.add_argument("--backend", help="GPU backend for Open CAD Studio (dx12, vulkan, gl)")
    ap.add_argument("--config", help="config folder with OpenCADStudio/plugins (APPDATA, XDG_CONFIG_HOME)")
    ap.add_argument("--ocs", default=r"C:\Program Files\Open CAD Studio\OpenCADStudio.exe" if sys.platform == "win32" else "OpenCADStudio")
    ap.add_argument("--port", type=int, default=18790)
    a = ap.parse_args()
    if not a.beelden and not a.klic:
        ap.error("--klic is nodig om het model te bouwen")
    if a.config:
        os.environ["APPDATA" if sys.platform == "win32" else "XDG_CONFIG_HOME"] = a.config
    dwg = HERE / f"{NAME}.dwg"
    if not a.beelden:
        shutil.copy(ROOT / "tests" / "data" / "leeg-m.dwg", dwg)
    s = Session(a, dwg)
    try:
        if not a.beelden:
            build(s, a.klic, dwg)
        views(s)
    finally:
        s.close()


if __name__ == "__main__":
    main()
