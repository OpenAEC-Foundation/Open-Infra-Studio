#!/usr/bin/env python3
"""Large model test through the MCP server, as an AI agent would build it.

Starts mcp/open_infra_mcp.py, picks an Open CAD Studio window (by pid,
session id or drawing name: --venster) and builds in an empty metric drawing:

  A. the 3D model of Dordrecht city centre (1.2 x 1.0 km): AHN terrain with
     the aerial photo, the BGT in 3D, 3D BAG LoD 2.2;
  B. civil work: existing ground (TIN), a ring road (alignment by PIs with
     curves, length and design profile, corridor with cross sections) and a
     dike (corridor at a fixed crest level), earthwork design against
     existing ground;
  C. checks: model info, quantities, NLCS check (these two need the shared
     view: start Open CAD Studio with OCS_V4_SNAPSHOT_SEGMENT_SIZE=268435456
     for a drawing this size);
  D. captures (overview, Grote Kerk, ring road, plan) in target/e2e/mcp/.

    python3 tests/e2e/mcp_model.py --venster dordrecht-mega
"""

import argparse
import base64
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SERVER = ROOT / "mcp" / "open_infra_mcp.py"
OUT = ROOT / "target" / "e2e" / "mcp"


class Client:
    def __init__(self, window):
        env = dict(os.environ, OI_SESSION=window) if window else dict(os.environ)
        # A batch build: changes applied at once, not kept as proposals (#92).
        env.setdefault("OI_ASSISTANT", "apply")
        self.p = subprocess.Popen([sys.executable, str(SERVER)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env)
        self.n = 0
        self.info = self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "mcp_model", "version": "1"}})
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.p.stdin.flush()

    def rpc(self, method, params=None):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params or {}}) + "\n")
        self.p.stdin.flush()
        return json.loads(self.p.stdout.readline())

    def call(self, name, args=None, image=None):
        t0 = time.time()
        r = self.rpc("tools/call", {"name": name, "arguments": args or {}})["result"]
        text = next((c["text"] for c in r["content"] if c["type"] == "text"), "")
        for c in r["content"]:
            if c["type"] == "image" and image:
                image.write_bytes(base64.b64decode(c["data"]))
        ok = not r.get("isError")
        print(("OK   " if ok else "FOUT ") + f"{name} ({time.time() - t0:.1f} s): " + text.split("\n")[0][:240], flush=True)
        return ok, r.get("structuredContent") or {}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--venster", help="pid, session id or drawing name of the Open CAD Studio window")
    ap.add_argument("--zonder-3d", action="store_true", help="skip part A")
    a = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    c = Client(a.venster)
    results = []
    step = lambda *x, **k: results.append((x[0], c.call(*x, **k)[0])) or results[-1]
    t0 = time.time()
    c.call("oi_sessions", {"select": a.venster} if a.venster else {})
    step("oi_site", {"x": 104900, "y": 425350, "width": 1200, "height": 1000, "name": "Dordrecht binnenstad"})
    if not a.zonder_3d:
        step("civ_photo_terrain", {"cell": 2})
        step("civ_bgt3d", {"buildings": False})
        step("civ_bag3d", {"lod": "2.2"})
    step("civ_tin", {"source": "AHN", "spacing": 5})
    ok, road = c.call("civ_alignment", {"pis": [[104350, 424900], [104800, 424950], [105300, 424900], [105480, 425100]], "radii": [200, 150], "name": "Rondweg Zuid"})
    results.append(("civ_alignment", ok))
    step("civ_length_profile", {"axis": road.get("as")})
    step("civ_design_profile", {"axis": road.get("as"), "pvis": [[0, 1.5], [400, 3.0], [800, 2.0], [road.get("lengte_m", 1180), 2.5]], "curve_length": 120})
    step("civ_corridor", {"axis": road.get("as"), "assembly": "weg", "frequency": 10, "sections": True})
    ok, dike = c.call("civ_alignment", {"pis": [[104330, 424900], [104360, 425500], [104450, 425830]], "radii": [300], "name": "Dijk West"})
    results.append(("civ_alignment", ok))
    step("civ_length_profile", {"axis": dike.get("as")})
    step("civ_corridor", {"axis": dike.get("as"), "assembly": "dijk", "level": 5.0, "crest_half_width": 3.0, "fill_slope": 3, "frequency": 20, "sections": True})
    step("civ_volume", {"cell": 1})
    step("oi_status")
    # These two need the shared view of a large drawing (see above).
    c.call("quantities")
    c.call("nlcs_check")
    step("oi_view", {"preset": "SWISO", "style": "GOURAUDSHADED"})
    step("oi_capture", {"max_dimension": 1600}, image=OUT / "overzicht.png")
    step("oi_capture", {"region": [104700, 425200, 105050, 425450], "max_dimension": 1600}, image=OUT / "grote-kerk.png")
    step("oi_capture", {"region": [104300, 424800, 105550, 425200], "max_dimension": 1600}, image=OUT / "rondweg.png")
    step("oi_view", {"preset": "TOP", "style": "WIREFRAME2D"})
    step("oi_capture", {"max_dimension": 1600}, image=OUT / "boven.png")
    failed = [n for n, ok in results if not ok]
    print(f"\n{len(results) - len(failed)}/{len(results)} stappen goed in {time.time() - t0:.0f} s; beelden in {OUT}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
