"""Recording helpers: my own Open CAD Studio for the promo video."""
import os, subprocess, sys, time, json
from pathlib import Path
V = Path(__file__).resolve().parent
os.environ["XDG_CONFIG_HOME"] = str(V / "cfg")
sys.path.insert(0, "/home/aia10/Documenten/Open-Infra/mcp")
sys.path.insert(0, "/home/aia10/Documenten/Open-Infra/tests/e2e")
import open_infra_mcp as mcp
OCS = mcp.OCS

def start(drawing):
    from ocs_e2e import default_ocs
    env = dict(os.environ, OCS_V4_SNAPSHOT_SEGMENT_SIZE="268435456")
    p = subprocess.Popen([default_ocs(), str(drawing)], stdout=open(V / "ocs.log", "w"), stderr=subprocess.STDOUT, start_new_session=True, env=env)
    (V / "ocs.pgid").write_text(str(p.pid))
    attach()
    return p

def attach():
    for _ in range(120):
        try:
            OCS.session = None
            OCS.pick(); OCS.document_id(); return
        except Exception:
            time.sleep(1)
    raise SystemExit("geen Open CAD Studio")

def pids():
    pg = (V / "ocs.pgid").read_text().strip()
    out = subprocess.run(["ps", "-eo", "pid=,pgid="], capture_output=True, text=True).stdout.split()
    return {out[i] for i in range(0, len(out), 2) if out[i + 1] == pg}

def window(title_part="Open CAD Studio"):
    out = subprocess.run(["wmctrl", "-lp"], capture_output=True, text=True).stdout
    ps = pids()
    for line in out.splitlines():
        f = line.split(None, 4)
        if len(f) >= 5 and f[2] in ps and title_part.lower() in f[4].lower():
            return f[0]
    return None

def host(cmd):
    return OCS.request("run", {"cmd": cmd, "document_id": OCS.document_id()})

def cancel():
    try:
        return OCS.request("cancel", {"document_id": OCS.document_id()}, timeout=10)
    except Exception as e:
        return str(e)

def capture(path, bounds=None, maxdim=1920, view=None):
    path = str(path)
    if os.path.exists(path):
        os.remove(path)
    body = {"path": path, "document_id": OCS.document_id(), "max_dimension": maxdim}
    if bounds:
        body.update({"view": "region", "bounds": list(bounds)})
    elif view:
        body["view"] = view
    r = OCS.request("capture", body, timeout=120)
    t0 = time.time()
    while not (os.path.exists(path) and os.path.getsize(path) > 0) and time.time() - t0 < 30:
        time.sleep(0.05)
    time.sleep(0.05)
    return r


def grab_window(title_part, path):
    """Screenshot of one of our windows (the whole window, ribbon included)."""
    from Xlib import X, display
    from PIL import Image
    out = subprocess.run(["wmctrl", "-lp"], capture_output=True, text=True).stdout
    ps = pids()
    wid = None
    for line in out.splitlines():
        f = line.split(None, 4)
        if len(f) >= 5 and f[2] in ps and title_part.lower() in f[4].lower():
            wid = int(f[0], 16)
    if wid is None:
        raise RuntimeError(f"geen venster '{title_part}'")
    d = display.Display()
    w = d.create_resource_object("window", wid)
    g = w.get_geometry()
    raw = w.get_image(0, 0, g.width, g.height, X.ZPixmap, 0xFFFFFFFF).data
    Image.frombytes("RGB", (g.width, g.height), raw, "raw", "BGRX").save(path)
    return g.width, g.height


def action(name, value=None):
    body = {"name": name, "document_id": OCS.document_id()}
    if value is not None:
        body["value"] = value
    return OCS.request("action", body)


def clean_setup():
    """Fullscreen, no panels, no view cube, UCS icon or cursor; rasters of the
    old import test out of this copy."""
    cancel()
    w = window()
    subprocess.run(["wmctrl", "-i", "-r", w, "-b", "add,fullscreen"])
    time.sleep(2)
    for c in ["CLEANSCREEN", "NAVVCUBE", "UCSICON OFF", "CURSORSIZE 1"]:
        host(c)
    OCS.request("pointer", {"document_id": OCS.document_id(), "events": [{"move": [-50, -50]}]})
    OCS.request("entities_delete", {"document_id": OCS.document_id(), "handles": ["806A", "8266"]})
