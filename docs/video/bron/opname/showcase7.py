"""Version 7 showcase drawing: as showcase.py, but plain facades (no
windows), plus KLIC in 3D and the BRO CPT columns."""
import subprocess, sys, time
import rec
from showcase import run
from citytest import clear
KLIC = "/home/aia10/Documenten/GIS2BIM/.claude/worktrees/fervent-wozniak-13c6e2/supportfiles/Sandbox/KLIC/Levering_21O072584_1.zip"

if __name__ == "__main__":
    rec.start(rec.V / "showcase7.dwg")
    time.sleep(8)
    rec.cancel()
    subprocess.run(["wmctrl", "-i", "-r", rec.window(), "-b", "add,fullscreen"])
    time.sleep(2)
    if rec.OCS.request("state").get("viewport_size") != [1920.0, 1018.0]:
        rec.host("CLEANSCREEN")
        time.sleep(1.5)
    rec.host("UCSICON OFF")
    rec.host("CURSORSIZE 1")
    rec.OCS.request("pointer", {"document_id": rec.OCS.document_id(), "events": [{"move": [-50, -50]}]})
    clear()
    run("GIS_SITE RD 104920 425330 1000 800 Dordrecht binnenstad")
    run("CIV_BGT3D PANDEN=nee")
    run("CIV_BAG3D RAMEN=nee")
    run(f"CIV_KLIC3D {KLIC} VERGROOT=2")
    run("CIV_SONDERINGEN3D MAX=60")
    run("GIS_Q_NWB")
    d = rec.OCS.document_id()
    best = None
    es = rec.OCS.request("query", {"document_id": d, "layer": "GIS-WEGVAKKEN", "detail": "full", "limit": 2000}).get("entities", [])
    for e in es:
        v = e["properties"].get("vertices", [])
        pts = [(p["location"]["x"], p["location"]["y"]) if "location" in p else (p["position"]["x"], p["position"]["y"]) for p in v]
        if len(pts) < 2:
            continue
        ends = {(round(pts[0][0]), round(pts[0][1])), (round(pts[-1][0]), round(pts[-1][1]))}
        if any(abs(x - 104909) < 3 and abs(y - 425541) < 3 for x, y in ends) and any(abs(x - 105110) < 3 and abs(y - 425705) < 3 for x, y in ends):
            best = e["handle"]
    print("straat:", best, flush=True)
    run("CIV_TIN BRON=AHN AFSTAND=5")
    r = run(f"CIV_AS AS={best} NAAM=Street")
    ax = r["data"].get("as")
    run(f"CIV_LENGTEPROFIEL AS={ax}")
    run(f"CIV_CORRIDOR AS={ax} SJABLOON=weg BOVEN=0.05 FREQUENTIE=10 DWARSPROFIELEN=ja")
    rec.host("QSAVE")
    time.sleep(20)
    print("klaar", flush=True)
