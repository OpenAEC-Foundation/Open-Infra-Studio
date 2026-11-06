#!/usr/bin/env python3
"""End-to-end test of the design problems (CIV_PROBLEMEN) in Open CAD Studio
(Dordrecht).

Runs its own Open CAD Studio with an isolated configuration folder
(``XDG_CONFIG_HOME`` under ``target/e2e-problemen``), so the plugin installed
for daily use is left alone. A window appears, as with ``ocs_e2e.py``.

Steps
  site           ``GIS_SITE RD 104885 425320 600 400 Dordrecht``.
  axis           ``CIV_AS PI=… R=300;200;40``: the curves at PI 1 and PI 2
                 overlap on the 145,6 m straight between them (T 61,4 + 107,6 m),
                 so they are drawn scaled to fit; R 40 at PI 3 is below the
                 minimum of 80 m (ROA 2017, niet-hoofdbaan 50 km/h).
  profiles       length profile (AHN) and a design profile whose vertical
                 curve at PVI 1 (BOOG=120) does not fit: drawn 40 m, 80 m fit.
  problems       ``CIV_PROBLEMEN``: the overlap with its three fixes (worked
                 by hand: R 185,6 at PI 1, R 156,5 at PI 2, both × 0,8615),
                 the radius with its fix R = 80, the vertical curve with
                 L = 80 m; markers on OI-PROBLEMEN.
  accept         a transition-curve problem at PI 3 with a reason: accepted.
  fix            the overlap (FIX=pi2): solved; one undo (U) brings back
                 R 200 at PI 2, REDO the fix; the acceptance on the changed
                 axis is outdated; the length profile is out of date and its
                 fix makes it again.
  vertical fix   the vertical curve gets L = 80 m: solved.
  accept         the radius at PI 3: accepted; saved, opened again: still
                 accepted (the XRecord OPENINFRA/PROBLEMEN).
  captures       the plan with the markers before and after, the length
                 profile, and the Problems page of the window.

Usage
  python3 tests/e2e/problemen_e2e.py [--ocs PATH] [--port 18821]

The report goes to target/e2e-problemen/rapport.md.
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
OUT = REPO / "target" / "e2e-problemen"

SITE = (104885, 425320, 600, 400)
PIS = [(104600, 425200), (104760, 425330), (104900, 425370), (105040, 425250), (105160, 425260)]
RADII = [300, 200, 40]


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


def tangent(r, a, p, b):
    """T = R·tan(Δ/2) at PI p between a and b (no transitions)."""
    d1 = math.atan2(p[1] - a[1], p[0] - a[0])
    d2 = math.atan2(b[1] - p[1], b[0] - p[0])
    delta = abs((d2 - d1 + math.pi) % (2 * math.pi) - math.pi)
    return r * math.tan(delta / 2), math.tan(delta / 2)


def by_hand():
    """The overlap and its fixes, worked out independently of the plugin."""
    leg = math.dist(PIS[1], PIS[2])
    t1, k1 = tangent(RADII[0], PIS[0], PIS[1], PIS[2])
    t2, k2 = tangent(RADII[1], PIS[1], PIS[2], PIS[3])
    return {
        "leg": leg,
        "used": t1 + t2,
        "r1": (leg - t2) / k1,
        "r2": (leg - t1) / k2,
        "k": leg / (t1 + t2),
    }


def window_picture(title, path):
    """The pixels of our window with this title, as PNG (X11)."""
    out = subprocess.run(["wmctrl", "-l"], capture_output=True, text=True).stdout
    ids = [int(line.split()[0], 16) for line in out.splitlines() if title in line]
    if not ids:
        return False
    from Xlib import X, display
    from PIL import Image
    d = display.Display()
    w = d.create_resource_object("window", ids[-1])
    g = w.get_geometry()
    raw = w.get_image(0, 0, g.width, g.height, X.ZPixmap, 0xFFFFFFFF).data
    Image.frombytes("RGB", (g.width, g.height), raw, "raw", "BGRX").save(path)
    return True


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18821)
    ap.add_argument("--no-install", action="store_true", help="use the plugins already installed in the isolated folder")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.no_install:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    cfg = xdg / "OpenCADStudio" / "open-infra"
    cfg.mkdir(parents=True, exist_ok=True)
    # English interface; the author name for acceptances.
    (cfg / "settings.json").write_text(json.dumps({"language": "en", "author": "E2E Tester"}))
    api_dir = cfg / "api"
    ocs_e2e.OUT = OUT
    ocs = ocs_e2e.Ocs(args.ocs, args.port)
    results = []
    state = {"hand": by_hand()}

    def step(name, fn):
        t = time.time()
        try:
            ok, detail = fn()
        except Exception as e:  # noqa: BLE001
            ok, detail = False, f"{type(e).__name__}: {e}"
        results.append((name, ok, detail, time.time() - t))
        print(("✔" if ok else "✘"), name, "—", detail, flush=True)
        return ok

    def result(rid, cmd, timeout=300):
        f = api_dir / f"{rid}.json"
        if f.exists():
            f.unlink()
        ocs.run(f"{cmd} ID={rid}")
        t0 = time.time()
        while time.time() - t0 < timeout:
            if f.exists():
                time.sleep(0.3)
                return json.loads(f.read_text())
            time.sleep(0.5)
        raise TimeoutError(f"{rid}: no result")

    def capture(name, bounds=None, focus=None):
        path = OUT / f"{name}.png"
        if path.exists():
            path.unlink()
        body = {"path": str(path), "document_id": ocs.doc(), "max_dimension": 1800}
        if bounds:
            body.update({"view": "region", "bounds": bounds})
        if focus:
            body["focus_handles"] = focus
        r = ocs.call("/capture", body)
        t0 = time.time()
        while not (path.exists() and path.stat().st_size > 0) and time.time() - t0 < 60:
            time.sleep(0.25)
        return path.exists(), f"{path.name} {r.get('error') or ''}".strip()

    def problems(rid):
        r = result(rid, "CIV_PROBLEMEN", timeout=400)
        state[rid] = r
        return r, r.get("data", {}).get("problems", [])

    def pick(lst, code, part=None):
        return [p for p in lst if p["code"] == code and (part is None or p["object"].endswith(part))]

    def layer_like(part):
        return [n for n in ocs.layers() if part in n]

    def site():
        x, y, w, h = SITE
        ocs.run(f"GIS_SITE RD {x} {y} {w} {h} Dordrecht")
        time.sleep(2)
        state["rect"] = ocs_e2e.site_rect(ocs)
        return True, f"site {state['rect']}"

    def axis():
        pis = ";".join(f"{x},{y}" for x, y in PIS)
        r = result("as1", f"CIV_AS PI={pis} R={';'.join(map(str, RADII))} NAAM=Problemenweg")
        d = r.get("data", {})
        state["as"] = d.get("as")
        state["len"] = d.get("lengte_m")
        drawn = d.get("straal_getekend", [])
        h = state["hand"]
        ok = r["ok"] and d.get("past") is False and len(drawn) == 3 and abs(drawn[0] - 300 * h["k"]) < 0.01 and abs(drawn[1] - 200 * h["k"]) < 0.01 and abs(drawn[2] - 40) < 1e-6
        xd = [str(x) for x in ocs.xdata(state["as"])] if state.get("as") else []
        import re
        stored = re.findall(r"(?<![a-z])pi\d+=[-0-9.,LA]+", " ".join(xd))
        crit = result("crit1", f"CIV_ONTWERPCRITERIA RICHTLIJN=roa CATEGORIE=niethoofdbaan V=50 AS={state['as']}")
        return ok and len(stored) == 5 and crit["ok"], f"{r['message']} | drawn R {[round(v, 2) for v in drawn]} (by hand ×{h['k']:.4f}) | {stored[1:2]} | {crit['message'][:80]}"

    def profiles():
        lp = result("lp1", f"CIV_LENGTEPROFIEL AS={state['as']}", timeout=400)
        n = state.get("len") or 600
        op = result("op1", f"CIV_ONTWERPPROFIEL AS={state['as']} PVI=0:1.0;40:1.8;300:3.0;{n - 5:.1f}:1.5 BOOG=120")
        pvi = op.get("data", {}).get("pvi", [])
        state["profiel"] = op.get("data", {}).get("ontwerpprofiel")
        ok = lp["ok"] and op["ok"] and len(pvi) == 4 and abs(pvi[1]["boog_m"] - 40) < 1e-6 and abs(pvi[2]["boog_m"] - 120) < 1e-6
        return ok, f"{lp['message'][:90]} | {op['message']} | L {[p['boog_m'] for p in pvi]}"

    def first_run():
        r, lst = problems("pr1")
        h = state["hand"]
        ov = pick(lst, "alignment.curve_overlap", "/PI1-PI2")
        rm = pick(lst, "criteria.radius_min", "/PI3")
        vc = pick(lst, "profile.curve_no_fit", "/PVI1")
        tm = pick(lst, "criteria.transition_missing", "/PI3")
        ok = r["ok"] and len(ov) == 1 and len(rm) == 1 and len(vc) == 1 and len(tm) == 1
        detail = [r["message"][:120]]
        if ov:
            o = ov[0]
            state["overlap"] = o
            fx = {f["id"]: f for f in o["fixes"]}
            radii = lambda i: fx[i]["action"]["radii"]  # noqa: E731
            ok = ok and set(fx) == {"pi1", "pi2", "beide"}
            ok = ok and abs(o["found"]["value"] - h["used"]) < 0.01 and abs(o["required"]["value"] - h["leg"]) < 0.01
            ok = ok and abs(radii("pi1")[1] - h["r1"]) < 0.011 and radii("pi1")[2] == 200
            ok = ok and abs(radii("pi2")[2] - h["r2"]) < 0.011 and radii("pi2")[1] == 300
            ok = ok and abs(radii("beide")[1] - 300 * h["k"]) < 0.011 and abs(radii("beide")[2] - 200 * h["k"]) < 0.011
            ok = ok and o["severity"] == "error" and o["message"]["nl"] and o["message"]["en"] and o["location"] and o["markers"]
            detail.append(f"overlap {o['found']['value']:.2f} > {o['required']['value']:.2f} m; fixes R1 {radii('pi1')[1]} (hand {h['r1']:.2f}), R2 {radii('pi2')[2]} (hand {h['r2']:.2f}), both {radii('beide')[1:3]}")
        if rm:
            f = rm[0]["fixes"]
            ok = ok and len(f) == 1 and f[0]["action"]["radii"][3] == 80 and rm[0]["required"]["value"] == 80
            detail.append(f"radius {rm[0]['found']['value']} < {rm[0]['required']['value']}: {f[0]['label']['en'] if f else rm[0].get('no_fix')}")
        if vc:
            f = vc[0]["fixes"]
            ok = ok and len(f) == 1 and abs(f[0]["action"]["lengths"][1] - 80) < 1e-6
            state["vertical"] = vc[0]
            detail.append(f"vertical: {f[0]['label']['en'] if f else '-'}")
        marks = sum(len(ocs.handles(l)) for l in layer_like("OI-PROBLEMEN"))
        ok = ok and marks > 0
        detail.append(f"{marks} marker entities, {len(lst)} problems")
        return ok, " | ".join(detail)

    def accept_transition():
        obj = f"as:{state['as']}/PI3"
        a = result("acc1", f'CIV_PROBLEEM_ACCEPTEER CODE=criteria.transition_missing OBJECT={obj} REDEN="urban street: no transition curves"')
        _, lst = problems("pr2")
        tm = pick(lst, "criteria.transition_missing", "/PI3")
        ok = a["ok"] and tm and tm[0]["status"] == "accepted" and tm[0]["accepted"]["author"] == "E2E Tester" and tm[0]["accepted"]["reason"].startswith("urban")
        return ok, f"{a['message']} | status {tm[0]['status'] if tm else None}"

    def pi2():
        """The layout record of PI 2 on the axis: "x,y,R" (XDATA pi2=)."""
        import re
        for x in ocs.xdata(state["as"]):
            m = re.search(r"pi2=([-0-9.,LA]+)", str(x))
            if m:
                return "pi2=" + m.group(1)
        return ""

    def fix_overlap():
        o = state["overlap"]
        want = next(f for f in o["fixes"] if f["id"] == "pi2")["action"]["radii"][2]
        f = result("fix1", f"CIV_PROBLEEM_HERSTEL CODE={o['code']} OBJECT={o['object']} FIX=pi2", timeout=400)
        d = f.get("data", {})
        # One undo step: U brings back R 200 at PI 2, REDO the fix (read from the axis's object data).
        after = pi2()
        ocs.run("U")
        time.sleep(2)
        undone = pi2()
        ocs.run("REDO")
        time.sleep(2)
        redone = pi2()
        r = lambda t: float(t.split(",")[2]) if t.count(",") >= 2 else None  # noqa: E731
        state["undo"] = (after, undone, redone)
        undo_ok = r(after) is not None and abs(r(after) - want) < 1e-6 and r(undone) == 200 and abs(r(redone) - want) < 1e-6
        _, lst = problems("pr3")
        gone = not pick(lst, "alignment.curve_overlap")
        tm = pick(lst, "criteria.transition_missing", "/PI3")
        outdated = bool(tm) and tm[0]["status"] == "open" and tm[0].get("acceptance_outdated")
        stale = pick(lst, "products.stale")
        state["stale"] = stale[0] if stale else None
        ok = f["ok"] and d.get("solved") is True and gone and outdated and bool(stale) and undo_ok
        return ok, f"{f['message'][:140]} | undo {state['undo']} | gone {gone} | acceptance outdated {outdated} | stale {[p['object'] for p in stale]}"

    def fix_vertical():
        v = state["vertical"]
        f = result("fix2", f"CIV_PROBLEEM_HERSTEL CODE={v['code']} OBJECT={v['object']}", timeout=300)
        _, lst = problems("pr6")
        gone = not pick(lst, "profile.curve_no_fit", "/PVI1")
        return f["ok"] and f["data"].get("solved") and gone, f"{f['message'][:160]} | gone {gone}"

    def fix_stale():
        s = state.get("stale")
        if not s:
            return False, "no stale product"
        f = result("fix3", f"CIV_PROBLEEM_HERSTEL CODE={s['code']} OBJECT={s['object']}", timeout=400)
        _, lst = problems("pr7")
        gone = not pick(lst, "products.stale")
        return f["ok"] and gone, f"{f['message'][:160]} | gone {gone}"

    def accept_radius():
        obj = f"as:{state['as']}/PI3"
        a = result("acc2", f'CIV_PROBLEEM_ACCEPTEER CODE=criteria.radius_min OBJECT={obj} REDEN="existing bridge abutment; 30 km/h zone"')
        _, lst = problems("pr8")
        rm = pick(lst, "criteria.radius_min", "/PI3")
        ok = a["ok"] and rm and rm[0]["status"] == "accepted"
        accepted_layer = sum(len(ocs.handles(l)) for l in layer_like("OI-PROBLEMEN-GEACCEPTEERD"))
        return ok and accepted_layer > 0, f"{a['message']} | status {rm[0]['status'] if rm else None} | accepted markers {accepted_layer}"

    def saved_and_opened():
        path = OUT / "problemen.dwg"
        if path.exists():
            path.unlink()
        r = ocs.call("/save", {"path": str(path), "document_id": ocs.doc()}, timeout=300)
        if r.get("ok") is False:
            return False, f"save: {r.get('error')}"
        t0 = time.time()
        while not path.exists() and time.time() - t0 < 30:
            time.sleep(0.5)
        time.sleep(2)
        # A copy, so the drawing really comes from the file.
        import shutil
        copy = OUT / "problemen-kopie.dwg"
        shutil.copy(path, copy)
        ocs.open(copy)
        time.sleep(4)
        _, lst = problems("pr9")
        rm = pick(lst, "criteria.radius_min", "/PI3")
        ok = path.exists() and rm and rm[0]["status"] == "accepted" and rm[0]["accepted"]["reason"].startswith("existing")
        return ok, f"{copy.name}: radius at PI 3 {rm[0]['status'] if rm else None} after opening the saved drawing ({len(lst)} problems)"

    def captures():
        for l in layer_like("-OG-TRIANGULATIE-"):
            ocs.run(f"LAYER FREEZE {l}")
        ocs.run("ZOOM EXTENTS")
        time.sleep(1.5)
        x, y, w, h = SITE
        rect = state.get("rect") or [x - w / 2, y - h / 2, x + w / 2, y + h / 2]
        ox, oy = (rect[0] + rect[2]) / 2 - x, (rect[1] + rect[3]) / 2 - y
        shots = [capture("plan-na", [PIS[0][0] + ox - 40, PIS[3][1] + oy - 160, PIS[4][0] + ox + 40, PIS[2][1] + oy + 80])]
        bend = PIS[3]
        shots.append(capture("pi3", [bend[0] + ox - 60, bend[1] + oy - 45, bend[0] + ox + 60, bend[1] + oy + 45]))
        f = api_dir / "lp1.json"
        if f.exists():
            at = json.loads(f.read_text()).get("data", {}).get("plaats")
            if at:
                n = state.get("len") or 640
                shots.append(capture("lengteprofiel", [at[0] - 30, at[1] - 50, at[0] + n + 40, at[1] + 90]))
        # The Problems page of the window.
        ocs.run("CIV_PROBLEMEN")
        time.sleep(12)
        page = OUT / "problemen-venster.png"
        got = window_picture("Open Infra — Civiel", page)
        shots.append((got, page.name if got else "window not found"))
        return all(s[0] for s in shots), ", ".join(s[1] for s in shots)

    def before_capture():
        x, y, w, h = SITE
        rect = state.get("rect") or [x - w / 2, y - h / 2, x + w / 2, y + h / 2]
        ox, oy = (rect[0] + rect[2]) / 2 - x, (rect[1] + rect[3]) / 2 - y
        a = capture("plan-voor", [PIS[0][0] + ox - 40, PIS[3][1] + oy - 160, PIS[4][0] + ox + 40, PIS[2][1] + oy + 80])
        # The overlapping bend between PI 1 and PI 2, with its marker.
        mx, my = (PIS[1][0] + PIS[2][0]) / 2 + ox, (PIS[1][1] + PIS[2][1]) / 2 + oy
        b = capture("bocht-voor", [mx - 110, my - 80, mx + 110, my + 50])
        return a[0] and b[0], f"{a[1]}, {b[1]}"

    if step("site", site) and step("axis with overlapping curves", axis):
        if step("length and design profile", profiles):
            if step("problems: computed fixes", first_run):
                step("capture before", before_capture)
                step("accept a problem", accept_transition)
                step("fix the overlap (one undo step)", fix_overlap)
                step("fix the vertical curve", fix_vertical)
                step("fix the out-of-date profile", fix_stale)
                step("accept the radius", accept_radius)
                step("saved and opened again", saved_and_opened)
                step("captures", captures)
    ocs.stop()
    (OUT / "state.json").write_text(json.dumps(state, indent=1, default=str))
    lines = ["# Design problems e2e", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {str(d).replace('|', '/')} | {t:.1f} |" for n, ok, d, t in results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if results and all(ok for _, ok, _, _ in results) else 1)


if __name__ == "__main__":
    main()
