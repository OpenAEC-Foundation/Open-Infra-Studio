#!/usr/bin/env python3
"""A place anywhere in the world, through the GIS window as a user would:
type a place name in the location search, pick the hit, save it with OK, then
lay the terrain with its aerial photo in the drawing and look at it in 3D.

Runs like ui_knoppen.py (its own D-Bus session, accessibility bus and config
folder; plugins built with `--features ui-test`):

    python3 tests/e2e/ui_wereld.py [--ocs <AppImage>] [--plaats "Half Dome, Yosemite"] [--verwacht usgs]

The report and the 3D picture are in target/e2e/ui/wereld.md and wereld-3d.png.
"""

import argparse
import os
import re
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ui_knoppen as uk
from ui_knoppen import OUT, ROOT, Ocs, Tester, Ui, a11y_bus, is_button, prepare


def type_into(ui, node, text):
    """Focus a text field and type `text` (EditableText when the field offers it, keystrokes otherwise)."""
    A = ui.A
    try:
        node.grab_focus()
    except Exception:
        pass
    ui.click(node)
    time.sleep(0.3)
    try:
        if A.EditableText.set_text_contents(node, text):
            return "editable"
    except Exception:
        pass
    A.generate_keyboard_event(0, text, A.KeySynthType.STRING)
    return "keys"


def wait_for(fn, timeout, step=0.5):
    t0 = time.time()
    while time.time() - t0 < timeout:
        v = fn()
        if v:
            return v
        time.sleep(step)
    return None


def inner(a):
    (OUT / "inner.pid").write_text(str(os.getpid()))
    launcher = a11y_bus()
    ocs = Ocs(a)
    ui = Ui()
    t = Tester(a, ocs, ui)
    steps = []

    def step(name, ok, detail=""):
        steps.append((name, ok, detail))
        print(f"{'OK ' if ok else 'FOUT'} {name}: {detail}", flush=True)
        return ok

    try:
        time.sleep(3)
        ocs.cancel()
        w = t.window("gis")
        nodes = ui.nodes(w)
        label = next((n for n in nodes if n["role"] == "label" and n["text"].startswith("Zoek plaats")), None)
        fields = [n for n in nodes if n["role"] in ("text", "entry")]
        field = min(fields, key=lambda n: abs(n["y"] - label["y"])) if label and fields else None
        if not step("zoekveld gevonden", field is not None, f"{len(fields)} tekstvelden"):
            return
        how = type_into(ui, field["node"], a.plaats)
        typed = wait_for(lambda: any(n["role"] in ("text", "entry") and a.plaats in n["value"] for n in ui.nodes(w)), 5)
        step("plaatsnaam getypt", bool(typed), how)
        zoek = next((n for n in ui.nodes(w) if is_button(n) and n["name"] == "Zoeken"), None)
        if not step("knop Zoeken", zoek is not None and zoek["enabled"], ""):
            return
        ui.click(zoek["node"])
        word = a.plaats.split(",")[0].strip().lower()
        clickable = ("push button", "button", "toggle button", "list item", "menu item")
        hit = wait_for(lambda: next((n for n in ui.nodes(w) if n["role"] in clickable and word in (n["name"] or n["text"]).lower()), None), 40)
        if not hit:
            hits = [n["text"] or n["name"] for n in ui.nodes(w) if word in (n["text"] or n["name"]).lower()]
            step("zoekresultaat", False, f"geen klikbaar resultaat; teksten met '{word}': {hits[:5]}")
            return
        step("zoekresultaat", True, hit["name"] or hit["text"])
        ui.click(hit["node"])
        time.sleep(1.5)
        spins = {n["name"].split()[0] if n["name"] else "": n["value"] for n in ui.nodes(w) if n["role"] in ("spin button", "slider")}
        lat = next((v for v in spins.values() if v.startswith("lat ")), None) or next((n["value"] for n in ui.nodes(w) if n["value"].startswith("lat ")), "")
        lon = next((n["value"] for n in ui.nodes(w) if n["value"].startswith("lon ")), "")
        step("coördinaten van de plaats", bool(lat and lon), f"{lat} {lon}")
        size = next((n for n in ui.nodes(w) if is_button(n) and n["name"] == str(a.maat)), None)
        if size is not None:
            ui.click(size["node"])
        foto = next((n for n in ui.nodes(w) if n["role"] in ("check box", "checkbox") and n["name"].startswith("Luchtfoto importeren")), None)
        if foto is not None and foto["toggled"]:
            ui.click(foto["node"])  # the 3D terrain brings its own photo
        ok = next((n for n in ui.nodes(w) if is_button(n) and n["name"] == "OK"), None)
        if not step("knop OK", ok is not None and ok["enabled"], ""):
            return
        ui.click(ok["node"])
        time.sleep(4)
        r = ocs.mcp.run(f"CIV_FOTOTERREIN CEL={a.cel}", timeout=900)
        msg = r.get("message", "")
        step("terrein met luchtfoto", r.get("ok", False), msg[:300])
        if a.verwacht:
            step(f"luchtfoto {a.verwacht}", a.verwacht.lower() in msg.lower(), "")
        heights = [float(x) for x in re.findall(r"(-?\d+(?:\.\d+)?) (?:tot|to) (-?\d+(?:\.\d+)?)", msg)[0]] if re.search(r"(?:tot|to) -?\d", msg) else []
        if heights and a.hoogte:
            lo, hi = (float(x) for x in a.hoogte.split(":"))
            step("hoogtebereik", lo <= heights[1] <= hi, f"{heights[0]} … {heights[1]} m, verwacht top in {lo}–{hi}")
        for c in ("VSCURRENT GOURAUDSHADED", "-VIEW SWISO", "ZOOM E"):
            ocs.mcp.host(c)
        time.sleep(3)
        pic = OUT / "wereld-3d.png"
        ocs.mcp.request("capture", {"path": str(pic), "document_id": ocs.mcp.document_id(), "max_dimension": 1600}, timeout=120)
        wait_for(lambda: pic.exists() and pic.stat().st_size > 0, 30)
        step("3D-beeld", pic.exists(), str(pic))
        crash = ocs.crashed()
        step("geen crash", crash is None, crash or "")
    finally:
        lines = [f"# Wereldscenario: {a.plaats}", ""] + [f"- {'✓' if ok else '✗'} **{n}** {d}" for n, ok, d in steps]
        (OUT / "wereld.md").write_text("\n".join(lines) + "\n")
        ocs.stop()
        launcher.terminate()
    sys.exit(0 if steps and all(ok for _, ok, _ in steps) else 1)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ocs", help="Open CAD Studio (AppImage of binary)")
    ap.add_argument("--plaats", default="Half Dome, Yosemite")
    ap.add_argument("--maat", type=int, default=2000, help="begrenzing (knop 250/500/1000/2000)")
    ap.add_argument("--cel", type=float, default=10.0)
    ap.add_argument("--verwacht", default="usgs", help="tekst die in de uitkomst moet staan (bijv. usgs, sentinel)")
    ap.add_argument("--hoogte", default="2400:2800", help="bereik waarin het hoogste punt moet liggen (m)")
    ap.add_argument("--tekening", help=argparse.SUPPRESS)
    ap.add_argument("--drawing", help=argparse.SUPPRESS)
    a = ap.parse_args()
    if os.environ.get("OI_UI_INNER"):
        sys.path.insert(0, str(ROOT / "mcp"))
        inner(a)
    else:
        uk.__file__ = __file__  # outer() re-runs this script inside the private D-Bus session
        uk.outer(a)


if __name__ == "__main__":
    main()
